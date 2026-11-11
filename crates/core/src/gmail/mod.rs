//! Client Gmail REST v1. Rispetta il budget di quota, ritenta con attesa crescente sugli errori
//! 429/403 di limite e sui 5xx, rinnova l'access token sui 401.

pub mod batch;
pub mod limiter;
pub mod types;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::json;

use crate::auth::TokenManager;
use crate::util::{b64url, backoff_ms, random_bytes};
use crate::{Error, Result};
pub use limiter::{Limiter, Priority};
pub use types::*;

const API: &str = "https://gmail.googleapis.com";
const USER: &str = "/gmail/v1/users/me";
const MAX_RETRIES: u32 = 5;
/// Elementi per batch HTTP di download: 10 `threads.get` completi sono già 1.200 unità, un quinto
/// del budget al minuto.
pub const FETCH_PER_BATCH: usize = 10;

/// Header chiesti nel formato `metadata` (thread fuori dalla finestra dei corpi).
pub const METADATA_HEADERS: &[&str] = &[
    "From", "To", "Cc", "Bcc", "Reply-To", "Subject", "Date", "Message-ID", "In-Reply-To", "References",
    "List-Unsubscribe", "List-Id", "Precedence", "Auto-Submitted",
];

/// Costo in unità di quota: tabella di Gmail in vigore dal 1° maggio 2026 per i progetti creati da
/// allora (la spec, sez. 5.5, usava quella vecchia: `threads.get` a 10).
pub mod cost {
    use super::ThreadFormat;

    pub const PROFILE: u32 = 1;
    pub const LABELS_LIST: u32 = 1;
    pub const HISTORY_LIST: u32 = 2;
    pub const THREADS_LIST: u32 = 10;
    pub const MESSAGES_LIST: u32 = 5;
    pub const THREADS_MODIFY: u32 = 10;
    pub const THREADS_TRASH: u32 = 20;
    pub const THREADS_UNTRASH: u32 = 10;
    pub const MESSAGES_BATCH_MODIFY: u32 = 50;
    pub const MESSAGES_SEND: u32 = 100;
    pub const LABELS_CREATE: u32 = 5;
    pub const LABELS_UPDATE: u32 = 5;
    pub const ATTACHMENTS_GET: u32 = 5;
    pub const SEND_AS_LIST: u32 = 1;

    pub const fn threads_get(f: ThreadFormat) -> u32 {
        match f {
            ThreadFormat::Full => 120,
            ThreadFormat::Metadata => 40,
        }
    }

    pub const fn messages_get(f: ThreadFormat) -> u32 {
        match f {
            ThreadFormat::Full => 60,
            ThreadFormat::Metadata => 20,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadFormat {
    Full,
    Metadata,
}

#[derive(Debug, Clone, Default)]
pub struct ListThreads {
    pub label_ids: Vec<String>,
    pub q: Option<String>,
    pub page_token: Option<String>,
    pub max_results: u32,
}

/// Operazione verso Gmail prodotta dalla coda in uscita.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteOp {
    Modify { thread_id: String, add: Vec<String>, remove: Vec<String> },
    Trash { thread_id: String },
    Untrash { thread_id: String },
}

impl RemoteOp {
    pub fn thread_id(&self) -> &str {
        match self {
            RemoteOp::Modify { thread_id, .. } | RemoteOp::Trash { thread_id } | RemoteOp::Untrash { thread_id } => thread_id,
        }
    }
}

/// Quello che il nucleo chiede a Gmail. Astratto per poter provare sync e coda senza rete.
pub trait GmailApi: Send + Sync + 'static {
    fn profile(&self) -> impl Future<Output = Result<Profile>> + Send;
    fn labels(&self) -> impl Future<Output = Result<Vec<Label>>> + Send;
    fn list_threads(&self, req: ListThreads, prio: Priority) -> impl Future<Output = Result<ThreadsPage>> + Send;
    /// `Error::NotFound` se `start_history_id` è troppo vecchio.
    fn history(&self, start_history_id: i64, page_token: Option<String>) -> impl Future<Output = Result<HistoryPage>> + Send;
    /// Messaggi (ID e thread), dal più recente. Costa la metà di `threads.list` e dice quanti
    /// messaggi ha ogni thread.
    fn list_messages(&self, req: ListThreads, prio: Priority) -> impl Future<Output = Result<MessagesPage>> + Send;
    /// Un risultato per ID, nello stesso ordine. `Error::NotFound` per i thread spariti.
    fn get_threads(&self, ids: &[String], format: ThreadFormat, prio: Priority) -> impl Future<Output = Result<Vec<Result<Thread>>>> + Send;
    /// Come `get_threads`, per singoli messaggi: costa la metà di un thread.
    fn get_messages(&self, ids: &[String], format: ThreadFormat, prio: Priority) -> impl Future<Output = Result<Vec<Result<Message>>>> + Send;
    /// Un risultato per operazione, nello stesso ordine.
    fn apply_ops(&self, ops: &[RemoteOp]) -> impl Future<Output = Result<Vec<Result<()>>>> + Send;
    /// Etichette su fino a 1.000 messaggi in una chiamata (sgombero in blocco).
    fn batch_modify_messages(&self, ids: &[String], add: &[String], remove: &[String]) -> impl Future<Output = Result<()>> + Send;
    /// Invia un messaggio RFC 2822 (in base64url). Restituisce l'ID Gmail del messaggio inviato.
    fn send_message(&self, raw: &str, thread_id: Option<&str>) -> impl Future<Output = Result<String>> + Send;
    /// C'è già un messaggio con questo Message-ID? Prima di ripetere un invio fallito.
    fn has_message_id(&self, rfc_message_id: &str) -> impl Future<Output = Result<bool>> + Send;
    fn create_label(&self, name: &str) -> impl Future<Output = Result<Label>> + Send;
    fn rename_label(&self, id: &str, name: &str) -> impl Future<Output = Result<Label>> + Send;
    /// Il contenuto di un allegato (`messages.attachments.get`).
    fn get_attachment(&self, message_id: &str, attachment_id: &str) -> impl Future<Output = Result<Vec<u8>>> + Send;
    /// Gli indirizzi "invia come", con le firme (`settings.sendAs.list`; basta `gmail.modify`).
    fn send_as(&self) -> impl Future<Output = Result<Vec<SendAs>>> + Send;
}

/// Oltre questa dimensione (in base64url) il messaggio va con il caricamento multipart: nel JSON di
/// `messages.send` Gmail accetta circa 5 MB, con il caricamento fino a 35.
const UPLOAD_OVER: usize = 4_000_000;

pub struct GmailClient {
    http: reqwest::Client,
    tokens: Arc<TokenManager>,
    limiter: Limiter,
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        // "gzip" nello User-Agent: Google comprime le risposte solo se c'è.
        .user_agent(concat!("diMail/", env!("CARGO_PKG_VERSION"), " (gzip)"))
        .gzip(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .expect("client HTTP")
}

fn api_error(status: u16, body: &[u8], retry_after: Option<Duration>) -> Error {
    if status == 404 {
        return Error::NotFound;
    }
    let parsed: Option<ApiErrorBody> = serde_json::from_slice(body).ok();
    let (message, reason) = match parsed {
        Some(b) => {
            let reason = b.error.errors.first().and_then(|e| e.reason.clone()).or(b.error.status);
            (b.error.message, reason)
        }
        None => (String::from_utf8_lossy(&body[..body.len().min(200)]).into_owned(), None),
    };
    Error::Api { status, reason, message, retry_after }
}

/// L'attesa indicata da Gmail: l'header `Retry-After`, oppure "Retry after <data RFC 3339>" nel
/// messaggio dei rifiuti per quota.
fn retry_hint(err: &Error) -> Option<Duration> {
    let Error::Api { retry_after, message, .. } = err else { return None };
    retry_after.or_else(|| {
        let at = message.split("Retry after ").nth(1)?.split_whitespace().next()?;
        let at = chrono::DateTime::parse_from_rfc3339(at.trim_end_matches(['.', ','])).ok()?;
        let ms = at.timestamp_millis() - crate::util::now_ms();
        Some(Duration::from_millis(ms.clamp(500, i64::MAX) as u64))
    })
}

fn log_throttle(err: &Error, wait: Duration, (budget, spent): (u32, u32), n: usize) {
    if let Error::Api { status, reason, message, .. } = err {
        // Il messaggio è il testo di Google sulla quota, non contenuto della posta.
        let message: String = message.chars().take(120).collect();
        tracing::warn!(
            status,
            reason = reason.as_deref().unwrap_or("-"),
            n,
            wait_ms = wait.as_millis() as u64,
            budget,
            spese_ultimo_minuto = spent,
            %message,
            "Gmail chiede di rallentare"
        );
    }
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

impl GmailClient {
    pub fn new(http: reqwest::Client, tokens: Arc<TokenManager>) -> Self {
        GmailClient { http, tokens, limiter: Limiter::new(limiter::UNITS_PER_MINUTE) }
    }

    /// Esegue una richiesta con quota, token e ritentativi. `build` costruisce la richiesta da capo
    /// a ogni tentativo.
    async fn send(
        &self,
        units: u32,
        prio: Priority,
        build: impl Fn(&reqwest::Client) -> reqwest::RequestBuilder,
    ) -> Result<(reqwest::header::HeaderMap, Vec<u8>)> {
        let mut attempt = 0u32;
        let mut refreshed = false;
        let mut net_attempt = 0u32;
        loop {
            self.limiter.acquire(units, prio).await;
            let token = self.tokens.access_token().await?;
            // Un errore di rete isolato (connessione chiusa a metà risposta) si riprova qui: se risalisse,
            // la prima sync ripartirebbe dall'inizio. Dopo due tentativi vale come rete assente.
            let fetched = async {
                let resp = build(&self.http).bearer_auth(&token).send().await?;
                let status = resp.status().as_u16();
                let headers = resp.headers().clone();
                let body = resp.bytes().await?;
                Ok::<_, Error>((status, headers, body))
            }
            .await;
            let (status, headers, body) = match fetched {
                Ok(r) => r,
                Err(e) if net_attempt < 2 => {
                    net_attempt += 1;
                    tracing::debug!(error = %e, tentativo = net_attempt, "errore di rete: riprovo");
                    tokio::time::sleep(Duration::from_millis(1_000 * u64::from(net_attempt))).await;
                    continue;
                }
                Err(e) => return Err(e),
            };
            if (200..300).contains(&status) {
                return Ok((headers, body.to_vec()));
            }
            if status == 401 && !refreshed {
                self.tokens.invalidate().await;
                refreshed = true;
                continue;
            }
            let ra = retry_after(&headers);
            let err = api_error(status, &body, ra);
            if err.is_transient() && attempt < MAX_RETRIES {
                attempt += 1;
                if err.is_rate_limit() {
                    // La pausa la fa rispettare `acquire` al giro dopo, a tutti.
                    let wait = self.limiter.throttled(retry_hint(&err)).await;
                    log_throttle(&err, wait, self.limiter.usage().await, 1);
                } else {
                    tokio::time::sleep(ra.unwrap_or_else(|| Duration::from_millis(backoff_ms(attempt - 1, 32_000)))).await;
                }
                continue;
            }
            return Err(err);
        }
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)], units: u32, prio: Priority) -> Result<T> {
        let url = format!("{API}{USER}{path}");
        let (_, body) = self.send(units, prio, |h| h.get(&url).query(query)).await?;
        self.limiter.succeeded().await;
        Ok(serde_json::from_slice(&body)?)
    }

    /// Batch HTTP: le richieste interne con errore transitorio tornano come errore al chiamante,
    /// che le rimette in coda.
    ///
    /// Con l'access token appena scaduto Gmail accetta la richiesta esterna ma rifiuta le interne
    /// con 401: si rinnova il token e si ripetono solo quelle, una volta.
    async fn batch(&self, reqs: Vec<batch::SubRequest>, units: u32, prio: Priority) -> Result<Vec<batch::SubResponse>> {
        let mut parts = self.batch_once(&reqs, units, prio).await?;
        let expired: Vec<usize> = parts.iter().enumerate().filter(|(_, p)| p.status == 401).map(|(i, _)| i).collect();
        if !expired.is_empty() {
            tracing::debug!(n = expired.len(), "batch: access token scaduto, lo rinnovo e ripeto");
            self.tokens.invalidate().await;
            let retry: Vec<batch::SubRequest> = expired.iter().map(|&i| reqs[i].clone()).collect();
            let share = (units as usize * retry.len()).div_ceil(reqs.len().max(1)) as u32;
            let again = self.batch_once(&retry, share, prio).await?;
            for (i, part) in expired.into_iter().zip(again) {
                parts[i] = part;
            }
        }
        Ok(parts)
    }

    async fn batch_once(&self, reqs: &[batch::SubRequest], units: u32, prio: Priority) -> Result<Vec<batch::SubResponse>> {
        if reqs.is_empty() {
            return Ok(Vec::new());
        }
        let n = reqs.len();
        let boundary = format!("dimail_{}", b64url(&random_bytes::<12>()));
        let body = batch::build(&boundary, reqs);
        let url = format!("{API}/batch/gmail/v1");
        let ct = format!("multipart/mixed; boundary={boundary}");
        let (headers, resp) = self
            .send(units, prio, |h| {
                h.post(&url).header(reqwest::header::CONTENT_TYPE, ct.as_str()).body(body.clone())
            })
            .await?;
        let resp_ct = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let text = String::from_utf8_lossy(&resp);
        let parts = batch::parse(&resp_ct, &text, n)?;

        // Le richieste rifiutate per quota tornano al chiamante come errore transitorio.
        let limited: Vec<Error> = parts
            .iter()
            .filter(|p| p.status == 429 || p.status == 403)
            .map(|p| api_error(p.status, p.body.as_bytes(), p.retry_after))
            .filter(Error::is_rate_limit)
            .collect();
        if let Some(first) = limited.first() {
            let hint = limited.iter().filter_map(retry_hint).max();
            let wait = self.limiter.throttled(hint).await;
            log_throttle(first, wait, self.limiter.usage().await, limited.len());
        } else {
            self.limiter.succeeded().await;
        }
        Ok(parts)
    }
}

fn list_query(req: ListThreads) -> Vec<(&'static str, String)> {
    let mut q: Vec<(&str, String)> = vec![("maxResults", req.max_results.max(1).to_string())];
    for l in req.label_ids {
        q.push(("labelIds", l));
    }
    if let Some(s) = req.q {
        q.push(("q", s));
    }
    if let Some(t) = req.page_token {
        q.push(("pageToken", t));
    }
    q
}

fn format_query(format: ThreadFormat) -> String {
    match format {
        ThreadFormat::Full => "format=full".to_owned(),
        ThreadFormat::Metadata => {
            let mut s = String::from("format=metadata");
            for h in METADATA_HEADERS {
                s.push_str("&metadataHeaders=");
                s.push_str(h);
            }
            s
        }
    }
}

impl GmailClient {
    /// `threads.get` o `messages.get` in batch HTTP.
    async fn get_many<T: DeserializeOwned>(
        &self,
        kind: &str,
        ids: &[String],
        format: ThreadFormat,
        unit_cost: u32,
        prio: Priority,
    ) -> Result<Vec<Result<T>>> {
        let suffix = format_query(format);
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(FETCH_PER_BATCH) {
            let reqs = chunk
                .iter()
                .map(|id| batch::SubRequest { method: "GET", path: format!("{USER}/{kind}/{id}?{suffix}"), json_body: None })
                .collect();
            let parts = self.batch(reqs, unit_cost * chunk.len() as u32, prio).await?;
            out.extend(parts.iter().map(sub_result::<T>));
        }
        Ok(out)
    }
}

fn sub_result<T: DeserializeOwned>(p: &batch::SubResponse) -> Result<T> {
    if (200..300).contains(&p.status) {
        Ok(serde_json::from_str(&p.body)?)
    } else {
        Err(api_error(p.status, p.body.as_bytes(), p.retry_after))
    }
}

impl GmailApi for GmailClient {
    async fn profile(&self) -> Result<Profile> {
        self.get_json("/profile", &[], cost::PROFILE, Priority::Interactive).await
    }

    async fn labels(&self) -> Result<Vec<Label>> {
        // I contatori dei non letti si calcolano in locale: sono istantanei e seguono le azioni ottimistiche.
        Ok(self
            .get_json::<LabelsResponse>("/labels", &[], cost::LABELS_LIST, Priority::Interactive)
            .await?
            .labels)
    }

    async fn list_threads(&self, req: ListThreads, prio: Priority) -> Result<ThreadsPage> {
        self.get_json("/threads", &list_query(req), cost::THREADS_LIST, prio).await
    }

    async fn list_messages(&self, req: ListThreads, prio: Priority) -> Result<MessagesPage> {
        self.get_json("/messages", &list_query(req), cost::MESSAGES_LIST, prio).await
    }

    async fn history(&self, start_history_id: i64, page_token: Option<String>) -> Result<HistoryPage> {
        let mut q: Vec<(&str, String)> = vec![
            ("startHistoryId", start_history_id.to_string()),
            ("maxResults", "500".into()),
            ("historyTypes", "messageAdded".into()),
            ("historyTypes", "messageDeleted".into()),
            ("historyTypes", "labelAdded".into()),
            ("historyTypes", "labelRemoved".into()),
        ];
        if let Some(t) = page_token {
            q.push(("pageToken", t));
        }
        self.get_json("/history", &q, cost::HISTORY_LIST, Priority::Interactive).await
    }

    async fn get_threads(&self, ids: &[String], format: ThreadFormat, prio: Priority) -> Result<Vec<Result<Thread>>> {
        self.get_many("threads", ids, format, cost::threads_get(format), prio).await
    }

    async fn get_messages(&self, ids: &[String], format: ThreadFormat, prio: Priority) -> Result<Vec<Result<Message>>> {
        self.get_many("messages", ids, format, cost::messages_get(format), prio).await
    }

    async fn send_message(&self, raw: &str, thread_id: Option<&str>) -> Result<String> {
        #[derive(serde::Deserialize)]
        struct Sent {
            id: String,
        }
        // Mai ritentativi automatici qui: un invio ripetuto lo decide la coda, dopo aver controllato gli Inviati.
        let token = self.tokens.access_token().await?;
        self.limiter.acquire(cost::MESSAGES_SEND, Priority::Interactive).await;
        let req = if raw.len() <= UPLOAD_OVER {
            let mut body = json!({ "raw": raw });
            if let Some(t) = thread_id {
                body["threadId"] = json!(t);
            }
            self.http.post(format!("{API}{USER}/messages/send")).json(&body)
        } else {
            // Con allegati grandi: multipart/related, prima i metadati (il thread) e poi il messaggio.
            let message = crate::util::b64url_decode(raw)?;
            let meta = match thread_id {
                Some(t) => json!({ "threadId": t }),
                None => json!({}),
            };
            let boundary = format!("dimail-{}", crate::util::b64url(&crate::util::random_bytes::<12>()));
            let mut body = Vec::with_capacity(message.len() + 400);
            body.extend_from_slice(
                format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: message/rfc822\r\n\r\n")
                    .as_bytes(),
            );
            body.extend_from_slice(&message);
            body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
            self.http
                .post(format!("{API}/upload{USER}/messages/send?uploadType=multipart"))
                .header(reqwest::header::CONTENT_TYPE, format!("multipart/related; boundary={boundary}"))
                .body(body)
        };
        let resp = req.bearer_auth(&token).timeout(Duration::from_secs(600)).send().await?;
        let status = resp.status().as_u16();
        let bytes = resp.bytes().await?;
        if !(200..300).contains(&status) {
            return Err(api_error(status, &bytes, None));
        }
        Ok(serde_json::from_slice::<Sent>(&bytes)?.id)
    }

    async fn has_message_id(&self, rfc_message_id: &str) -> Result<bool> {
        let id = rfc_message_id.trim_matches(['<', '>']);
        let page: MessagesPage = self
            .get_json(
                "/messages",
                &[("q", format!("rfc822msgid:{id}")), ("maxResults", "1".into()), ("includeSpamTrash", "true".into())],
                cost::MESSAGES_LIST,
                Priority::Interactive,
            )
            .await?;
        Ok(!page.messages.is_empty())
    }

    async fn create_label(&self, name: &str) -> Result<Label> {
        let url = format!("{API}{USER}/labels");
        let body = json!({ "name": name, "labelListVisibility": "labelShow", "messageListVisibility": "show" });
        let (_, bytes) = self.send(cost::LABELS_CREATE, Priority::Interactive, |h| h.post(&url).json(&body)).await?;
        self.limiter.succeeded().await;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn get_attachment(&self, message_id: &str, attachment_id: &str) -> Result<Vec<u8>> {
        #[derive(serde::Deserialize)]
        struct Body {
            data: String,
        }
        let b: Body = self
            .get_json(&format!("/messages/{message_id}/attachments/{attachment_id}"), &[], cost::ATTACHMENTS_GET, Priority::Interactive)
            .await?;
        crate::util::b64url_decode(&b.data)
    }

    async fn send_as(&self) -> Result<Vec<SendAs>> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct List {
            #[serde(default)]
            send_as: Vec<SendAs>,
        }
        let l: List = self.get_json("/settings/sendAs", &[], cost::SEND_AS_LIST, Priority::Interactive).await?;
        Ok(l.send_as)
    }

    async fn rename_label(&self, id: &str, name: &str) -> Result<Label> {
        let url = format!("{API}{USER}/labels/{id}");
        let body = json!({ "name": name });
        let (_, bytes) = self.send(cost::LABELS_UPDATE, Priority::Interactive, |h| h.patch(&url).json(&body)).await?;
        self.limiter.succeeded().await;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn batch_modify_messages(&self, ids: &[String], add: &[String], remove: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let url = format!("{API}{USER}/messages/batchModify");
        let body = json!({ "ids": ids, "addLabelIds": add, "removeLabelIds": remove });
        self.send(cost::MESSAGES_BATCH_MODIFY, Priority::Interactive, |h| h.post(&url).json(&body)).await?;
        self.limiter.succeeded().await;
        Ok(())
    }

    async fn apply_ops(&self, ops: &[RemoteOp]) -> Result<Vec<Result<()>>> {
        let mut out = Vec::with_capacity(ops.len());
        for chunk in ops.chunks(batch::MAX_BATCH) {
            let mut units = 0;
            let reqs = chunk
                .iter()
                .map(|op| match op {
                    RemoteOp::Modify { thread_id, add, remove } => {
                        units += cost::THREADS_MODIFY;
                        batch::SubRequest {
                            method: "POST",
                            path: format!("{USER}/threads/{thread_id}/modify"),
                            json_body: Some(json!({ "addLabelIds": add, "removeLabelIds": remove }).to_string()),
                        }
                    }
                    RemoteOp::Trash { thread_id } => {
                        units += cost::THREADS_TRASH;
                        batch::SubRequest { method: "POST", path: format!("{USER}/threads/{thread_id}/trash"), json_body: None }
                    }
                    RemoteOp::Untrash { thread_id } => {
                        units += cost::THREADS_UNTRASH;
                        batch::SubRequest { method: "POST", path: format!("{USER}/threads/{thread_id}/untrash"), json_body: None }
                    }
                })
                .collect();
            let parts = self.batch(reqs, units, Priority::Interactive).await?;
            out.extend(parts.iter().map(|p| sub_result::<serde_json::Value>(p).map(|_| ())));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_reads_reason() {
        let body = br#"{"error":{"code":403,"message":"User-rate limit exceeded","errors":[{"reason":"userRateLimitExceeded"}]}}"#;
        let e = api_error(403, body, None);
        assert!(e.is_rate_limit());
        assert!(e.is_transient());
        let e = api_error(400, br#"{"error":{"code":400,"message":"Invalid label","errors":[{"reason":"invalidArgument"}]}}"#, None);
        assert!(!e.is_transient());
        assert!(matches!(api_error(404, b"{}", None), Error::NotFound));
    }

    #[test]
    fn retry_hint_reads_the_date_in_the_message() {
        let at = chrono::Utc::now() + chrono::Duration::seconds(30);
        let body = format!(
            r#"{{"error":{{"code":429,"message":"User-rate limit exceeded.  Retry after {}","errors":[{{"reason":"rateLimitExceeded"}}]}}}}"#,
            at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        );
        let e = api_error(429, body.as_bytes(), None);
        let h = retry_hint(&e).unwrap();
        assert!(h > Duration::from_secs(28) && h <= Duration::from_secs(30), "{h:?}");
        let e = api_error(429, br#"{"error":{"code":429,"message":"Too many concurrent requests for user"}}"#, None);
        assert_eq!(retry_hint(&e), None);
        let e = api_error(429, b"{}", Some(Duration::from_secs(3)));
        assert_eq!(retry_hint(&e), Some(Duration::from_secs(3)));
    }
}
