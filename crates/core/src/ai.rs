//! AI con Mistral o OpenAI, a scelta nelle impostazioni (sez. 12 della spec, anticipata il 9 novembre
//! per due funzioni):
//! - ⌘J nel composer: scrive la risposta, o sistema la bozza che ho scritto;
//! - sezioni con istruzioni: l'AI assegna l'etichetta ai thread che corrispondono.
//! (Le sintesi nella lista, provate il 9 novembre, sono state tolte su richiesta di Tommaso.)
//!
//! Le chiavi (una per provider) stanno nel Portachiavi, mai su disco né nei log. Nei log niente testo delle email né
//! risposte del modello: solo conteggi di token. Un tetto giornaliero di token ferma le chiamate.

use std::collections::HashMap;
#[cfg(test)]
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
#[cfg(test)]
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::auth::keychain;
use crate::events::CoreEvent;
use crate::gmail::GmailApi;
use crate::outbox::{self, Action};
use crate::sync::Ctx;
use crate::util::now_ms;
use crate::{Error, Result};

/// Servizio del Portachiavi per le chiavi (`<APP_ID>.ai-api-key`); l'account è il provider.
pub const KEY_SERVICE: &str = "ai-api-key";
/// Impostazione dell'account con provider e modello scelti.
const CONFIG_SETTING: &str = "ai.config";
/// Tetto di default: circa 1.500 sintesi al giorno.
pub const DEFAULT_DAILY_TOKENS: i64 = 2_000_000;
const CLASSIFY_BATCH: usize = 20;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub system: String,
    pub user: String,
    pub json: bool,
    pub max_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct ChatReply {
    pub text: String,
    pub tokens: i64,
}

/// Il modello. Astratto per provare tutto senza rete.
pub trait Llm: Send + Sync + 'static {
    /// C'è la chiave del provider scelto.
    fn available(&self) -> bool;
    fn config(&self) -> AiConfig;
    fn configure(&self, cfg: AiConfig);
    fn has_key(&self, provider: Provider) -> bool;
    fn chat(&self, req: ChatRequest) -> BoxFut<'_, Result<ChatReply>>;
    /// Prova la chiave sul provider e la salva. Mai nei log.
    fn set_key(&self, provider: Provider, key: String) -> BoxFut<'_, Result<()>>;
    fn clear_key(&self, provider: Provider) -> Result<()>;
}

/// Nessun modello: l'AI è spenta.
pub struct NoLlm;

impl Llm for NoLlm {
    fn available(&self) -> bool {
        false
    }
    fn config(&self) -> AiConfig {
        AiConfig::default()
    }
    fn configure(&self, _cfg: AiConfig) {}
    fn has_key(&self, _provider: Provider) -> bool {
        false
    }
    fn chat(&self, _req: ChatRequest) -> BoxFut<'_, Result<ChatReply>> {
        Box::pin(async { Err(Error::other("AI non configurata")) })
    }
    fn set_key(&self, _provider: Provider, _key: String) -> BoxFut<'_, Result<()>> {
        Box::pin(async { Err(Error::other("AI non disponibile")) })
    }
    fn clear_key(&self, _provider: Provider) -> Result<()> {
        Ok(())
    }
}

// ───────────────────────────── provider, modelli e chiavi ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Mistral,
    Openai,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Mistral, Provider::Openai];

    pub fn name(self) -> &'static str {
        match self {
            Provider::Mistral => "Mistral",
            Provider::Openai => "OpenAI",
        }
    }

    fn api(self) -> &'static str {
        match self {
            Provider::Mistral => "https://api.mistral.ai/v1",
            Provider::Openai => "https://api.openai.com/v1",
        }
    }

    /// L'account della voce nel Portachiavi.
    fn key_account(self) -> &'static str {
        match self {
            Provider::Mistral => "mistral",
            Provider::Openai => "openai",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Mistral => "mistral-small-2603",
            Provider::Openai => "gpt-5.4-mini",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiConfig {
    pub provider: Provider,
    pub model: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        AiConfig { provider: Provider::Mistral, model: Provider::Mistral.default_model().into() }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub provider: Provider,
    pub id: &'static str,
    pub name: &'static str,
    pub recommended: bool,
}

/// I modelli proposti nelle impostazioni (gli stessi del catalogo di DiVoice). Se ne può scrivere
/// un altro a mano: «Prova» dice se funziona.
pub const MODELS: &[ModelInfo] = &[
    ModelInfo { provider: Provider::Mistral, id: "mistral-small-2603", name: "Mistral Small 4", recommended: true },
    ModelInfo { provider: Provider::Mistral, id: "mistral-medium-2604", name: "Mistral Medium 3.5", recommended: false },
    ModelInfo { provider: Provider::Mistral, id: "mistral-large-4-0", name: "Mistral Large 4", recommended: false },
    ModelInfo { provider: Provider::Openai, id: "gpt-5.4-mini", name: "GPT-5.4 mini", recommended: true },
    ModelInfo { provider: Provider::Openai, id: "gpt-6-luna", name: "GPT-6 Luna", recommended: false },
    ModelInfo { provider: Provider::Openai, id: "gpt-6.1-terra", name: "GPT-6.1 Terra", recommended: false },
    ModelInfo { provider: Provider::Openai, id: "gpt-6.1-sol", name: "GPT-6.1 Sol", recommended: false },
];

/// Un nome di modello accettabile: lettere, cifre e `._:/-`, fino a 100 caratteri.
pub fn valid_model(model: &str) -> bool {
    let mut chars = model.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric())
        && model.len() <= 100
        && chars.all(|c| c.is_ascii_alphanumeric() || "._:/-".contains(c))
}

/// Il corpo della richiesta per provider e modello. OpenAI: niente `temperature` (i modelli che
/// ragionano non la accettano), `max_completion_tokens` e ragionamento al minimo; il ragionamento
/// conta nei token, quindi se c'è serve margine.
pub fn request_body(cfg: &AiConfig, req: &ChatRequest) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": cfg.model,
        "messages": [
            { "role": "system", "content": req.system },
            { "role": "user", "content": req.user },
        ],
    });
    match cfg.provider {
        Provider::Mistral => {
            body["temperature"] = serde_json::json!(0.2);
            body["max_tokens"] = serde_json::json!(req.max_tokens);
        }
        Provider::Openai => {
            // gpt-6.1-sol non scende sotto "low"; gli altri accettano "none".
            let effort = if cfg.model.starts_with("gpt-6.1-sol") { "low" } else { "none" };
            let room = if effort == "none" { 0 } else { 4_000 };
            body["reasoning_effort"] = serde_json::json!(effort);
            body["max_completion_tokens"] = serde_json::json!(req.max_tokens + room);
        }
    }
    if req.json {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }
    body
}

/// Il testo della risposta: una stringa o, con alcuni modelli, una lista di blocchi.
fn reply_text(content: Option<serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(s)) => s,
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()).is_none_or(|t| t == "text"))
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect(),
        _ => String::new(),
    }
}

pub fn load_key(provider: Provider) -> Option<String> {
    match keychain::get(&keychain::service(KEY_SERVICE), provider.key_account()) {
        Ok(Some(b)) => String::from_utf8(b).ok().filter(|k| !k.trim().is_empty()),
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = %e, provider = provider.name(), "Portachiavi: chiave dell'AI non leggibile");
            None
        }
    }
}

/// Le API di chat di Mistral e OpenAI (stesso formato: `/v1/chat/completions`).
pub struct ChatApi {
    http: reqwest::Client,
    keys: RwLock<HashMap<Provider, String>>,
    cfg: RwLock<AiConfig>,
}

impl ChatApi {
    pub fn from_keychain(http: reqwest::Client) -> Self {
        let keys = Provider::ALL.into_iter().filter_map(|p| load_key(p).map(|k| (p, k))).collect();
        ChatApi { http, keys: RwLock::new(keys), cfg: RwLock::new(AiConfig::default()) }
    }

    fn key(&self, provider: Provider) -> Option<String> {
        self.keys.read().unwrap_or_else(|p| p.into_inner()).get(&provider).cloned()
    }

    /// Prova la chiave sul provider e, se va, la salva nel Portachiavi.
    async fn store_key(&self, provider: Provider, key: &str) -> Result<()> {
        let key = key.trim().to_owned();
        if key.is_empty() {
            return Err(Error::other("chiave vuota"));
        }
        let resp = self.http.get(format!("{}/models", provider.api())).bearer_auth(&key).send().await?;
        let name = provider.name();
        match resp.status().as_u16() {
            200..=299 => {}
            401 | 403 => return Err(Error::other(format!("{name} non accetta questa chiave"))),
            s => return Err(Error::other(format!("{name} ha risposto {s}: riprova tra poco"))),
        }
        keychain::set(&keychain::service(KEY_SERVICE), provider.key_account(), key.as_bytes())?;
        self.keys.write().unwrap_or_else(|p| p.into_inner()).insert(provider, key);
        tracing::info!(provider = name, "chiave dell'AI salvata");
        Ok(())
    }

    fn remove_key(&self, provider: Provider) -> Result<()> {
        keychain::delete(&keychain::service(KEY_SERVICE), provider.key_account())?;
        self.keys.write().unwrap_or_else(|p| p.into_inner()).remove(&provider);
        Ok(())
    }
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}
#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}
#[derive(Deserialize)]
struct ChoiceMessage {
    content: Option<serde_json::Value>,
}
#[derive(Deserialize)]
struct Usage {
    total_tokens: Option<i64>,
}

impl Llm for ChatApi {
    fn available(&self) -> bool {
        self.key(self.config().provider).is_some()
    }

    fn config(&self) -> AiConfig {
        self.cfg.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn configure(&self, cfg: AiConfig) {
        *self.cfg.write().unwrap_or_else(|p| p.into_inner()) = cfg;
    }

    fn has_key(&self, provider: Provider) -> bool {
        self.key(provider).is_some()
    }

    fn chat(&self, req: ChatRequest) -> BoxFut<'_, Result<ChatReply>> {
        Box::pin(async move {
            let cfg = self.config();
            let name = cfg.provider.name();
            let key = self.key(cfg.provider).ok_or_else(|| Error::other(format!("manca la chiave di {name}")))?;
            let resp = self
                .http
                .post(format!("{}/chat/completions", cfg.provider.api()))
                .bearer_auth(&key)
                .timeout(Duration::from_secs(60))
                .json(&request_body(&cfg, &req))
                .send()
                .await?;
            let status = resp.status().as_u16();
            let bytes = resp.bytes().await?;
            if !(200..300).contains(&status) {
                // Il corpo dell'errore non contiene la posta: è il motivo del rifiuto.
                let message: String = String::from_utf8_lossy(&bytes).chars().take(200).collect();
                return Err(Error::Api { status, reason: None, message, retry_after: None });
            }
            let r: ChatResponse = serde_json::from_slice(&bytes)?;
            let text = reply_text(r.choices.into_iter().next().and_then(|c| c.message.content));
            Ok(ChatReply { text, tokens: r.usage.and_then(|u| u.total_tokens).unwrap_or(0) })
        })
    }

    fn set_key(&self, provider: Provider, key: String) -> BoxFut<'_, Result<()>> {
        Box::pin(async move { self.store_key(provider, &key).await })
    }

    fn clear_key(&self, provider: Provider) -> Result<()> {
        self.remove_key(provider)
    }
}

/// Provider e modello salvati nelle impostazioni dell'account (Mistral Small se non c'è niente).
pub fn load_config(c: &Connection) -> Result<AiConfig> {
    Ok(crate::store::get_setting(c, CONFIG_SETTING)?
        .and_then(|v| serde_json::from_value::<AiConfig>(v).ok())
        .filter(|cfg| valid_model(&cfg.model))
        .unwrap_or_default())
}

pub fn save_config(c: &Connection, cfg: &AiConfig) -> Result<()> {
    if !valid_model(&cfg.model) {
        return Err(Error::other("nome del modello non valido"));
    }
    crate::store::set_setting(c, CONFIG_SETTING, &serde_json::to_value(cfg)?)
}

/// «Prova»: una richiesta minima al modello scelto. Restituisce i millisecondi.
pub async fn check(llm: &dyn Llm) -> Result<u64> {
    let cfg = llm.config();
    if !llm.available() {
        return Err(Error::other(format!("manca la chiave di {}", cfg.provider.name())));
    }
    let started = std::time::Instant::now();
    let req = ChatRequest { system: "Rispondi solo: ok".into(), user: "Prova".into(), json: false, max_tokens: 16 };
    match llm.chat(req).await {
        Ok(_) => Ok(started.elapsed().as_millis() as u64),
        Err(Error::Api { status, message, .. }) => {
            Err(Error::other(format!("{} ha risposto {status} con {}: {}", cfg.provider.name(), cfg.model, message.trim())))
        }
        Err(e) => Err(e),
    }
}

// ───────────────────────────── consumo ─────────────────────────────

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub fn record_usage(c: &Connection, tokens: i64) -> Result<()> {
    c.execute(
        "INSERT INTO ai_usage (day, tokens, requests) VALUES (?1, ?2, 1)
         ON CONFLICT (day) DO UPDATE SET tokens = tokens + excluded.tokens, requests = requests + 1",
        params![today(), tokens],
    )?;
    Ok(())
}

pub fn tokens_today(c: &Connection) -> Result<i64> {
    Ok(c.query_row("SELECT coalesce(sum(tokens), 0) FROM ai_usage WHERE day = ?1", [today()], |r| r.get(0))?)
}

fn daily_cap(c: &Connection) -> Result<i64> {
    Ok(crate::store::get_setting(c, "ai.daily_tokens")?.and_then(|v| v.as_i64()).unwrap_or(DEFAULT_DAILY_TOKENS))
}

pub fn over_budget(c: &Connection) -> Result<bool> {
    Ok(tokens_today(c)? >= daily_cap(c)?)
}

#[derive(Debug, Clone, Serialize)]
pub struct AiStatus {
    /// C'è la chiave del provider scelto: l'AI funziona.
    pub has_key: bool,
    pub provider: Provider,
    pub model: String,
    /// I provider con una chiave nel Portachiavi.
    pub keys: Vec<Provider>,
    pub tokens_today: i64,
    pub daily_cap: i64,
}

pub fn status(c: &Connection, llm: &dyn Llm) -> Result<AiStatus> {
    let cfg = llm.config();
    Ok(AiStatus {
        has_key: llm.available(),
        provider: cfg.provider,
        model: cfg.model,
        keys: Provider::ALL.into_iter().filter(|p| llm.has_key(*p)).collect(),
        tokens_today: tokens_today(c)?,
        daily_cap: daily_cap(c)?,
    })
}

// ───────────────────────────── testo per il modello ─────────────────────────────

fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Il thread come testo per il modello: oggetto, partecipanti, gli ultimi messaggi (puliti da citazioni
/// e firme, dal più vecchio al più recente), i nomi degli allegati. Mai gli allegati stessi.
pub fn thread_context(c: &Connection, thread_id: &str, budget_chars: usize) -> Result<Option<String>> {
    let subject: Option<String> = c
        .query_row("SELECT subject FROM threads WHERE id = ?1", [thread_id], |r| r.get(0))
        .ok();
    let Some(subject) = subject else { return Ok(None) };
    let mut msgs: Vec<(String, i64, String, String)> = c
        .prepare(
            "SELECT coalesce(m.from_name, m.from_email, '?') || ' <' || coalesce(m.from_email, '') || '>', m.internal_date,
                    coalesce(b.text_unquoted, b.text_plain, m.snippet),
                    coalesce((SELECT group_concat(a.filename, ', ') FROM attachments a WHERE a.message_rowid = m.rowid AND a.is_inline = 0), '')
             FROM messages m LEFT JOIN message_bodies b ON b.message_rowid = m.rowid
             WHERE m.thread_id = ?1 AND m.label_ids NOT LIKE '%\"DRAFT\"%'
             ORDER BY m.internal_date DESC LIMIT 4",
        )?
        .query_map([thread_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;
    msgs.reverse();
    let mut out = format!("Oggetto: {subject}\n");
    let per = budget_chars / msgs.len().max(1);
    for (i, (from, date, text, files)) in msgs.iter().enumerate() {
        use chrono::TimeZone;
        let when = chrono::Local.timestamp_millis_opt(*date).single().map(|d| d.format("%d/%m/%Y %H:%M").to_string()).unwrap_or_default();
        let last = i + 1 == msgs.len();
        // L'ultimo messaggio pesa di più.
        let room = if last { per * 2 } else { per / 2 };
        out.push_str(&format!("\n--- {}Da: {from} — {when}\n{}\n", if last { "ULTIMO MESSAGGIO. " } else { "" }, clip(text, room)));
        if !files.is_empty() {
            out.push_str(&format!("Allegati: {files}\n"));
        }
    }
    Ok(Some(out))
}

// ───────────────────────────── scrittura (⌘J) ─────────────────────────────

/// Richiesta di scrittura dal composer: la bozza (o niente) e il thread a cui si risponde.
#[derive(Debug, Clone, Deserialize)]
pub struct WriteRequest {
    /// new | reply | reply_all | forward
    pub mode: String,
    pub thread_id: Option<String>,
    #[serde(default)]
    pub to: Vec<crate::mime::Addr>,
    #[serde(default)]
    pub subject: String,
    /// Quello che ho scritto: appunti, una bozza grezza, o vuoto.
    #[serde(default)]
    pub notes: String,
}

pub fn write_request(me_name: &str, me_email: &str, req: &WriteRequest, context: Option<&str>) -> ChatRequest {
    let first = me_name.split_whitespace().next().unwrap_or(me_name);
    let kind = match req.mode.as_str() {
        "reply" => "risposta",
        "reply_all" => "risposta a tutti",
        "forward" => "inoltro (il testo che accompagna il messaggio inoltrato)",
        _ => "nuova email",
    };
    let to = req.to.iter().map(|a| a.name.clone().unwrap_or_else(|| a.email.clone())).collect::<Vec<_>>().join(", ");
    let mut user = format!("Tipo: {kind}\nDestinatari: {}\nOggetto: {}\n", if to.is_empty() { "-" } else { &to }, req.subject.trim());
    if let Some(ctx) = context {
        user.push_str(&format!("\nTHREAD (dal più vecchio al più recente):\n{ctx}\n"));
    }
    let notes = req.notes.trim();
    if notes.is_empty() {
        user.push_str("\nBOZZA: (vuota) Scrivi tu la risposta più adatta all'ultimo messaggio, breve.\n");
    } else {
        user.push_str(&format!("\nBOZZA DI {first}:\n{notes}\n"));
    }
    ChatRequest {
        system: format!(
            "Sei l'assistente di scrittura di {me_name} <{me_email}>. Scrivi email chiare, cordiali e dirette, nella lingua \
             della bozza (se è vuota, in quella dell'ultimo messaggio del thread) e con il registro adatto al destinatario \
             (tu o lei come nel thread). Se ricevi una bozza o degli appunti, trasformali in un'email completa e scorrevole: \
             mantieni tutti i fatti, i nomi, le cifre, le date e le richieste, correggi grammatica e tono, non aggiungere \
             informazioni o impegni che non ci sono. Inizia con un saluto breve e chiudi con un saluto e il nome {first}, \
             senza firma completa. I link scritti come [testo](indirizzo) restano in quella forma, con lo stesso indirizzo. \
             Niente oggetto, niente segnaposto tra parentesi, niente commenti: rispondi solo con il testo dell'email."
        ),
        user,
        json: false,
        max_tokens: 900,
    }
}

/// Toglie quello che il modello a volte aggiunge attorno al testo: blocchi di codice, virgolette, "Oggetto:".
fn tidy_draft(text: &str) -> String {
    let mut t = text.trim().trim_matches('`').trim().to_owned();
    if let Some(rest) = t.strip_prefix("Oggetto:").or_else(|| t.strip_prefix("Subject:")) {
        t = rest.split_once('\n').map(|(_, b)| b.trim().to_owned()).unwrap_or_default();
    }
    t.trim_matches(['"', '“', '”']).trim().to_owned()
}

/// ⌘J: scrive o sistema la bozza. Il testo delle email va al provider, nei log solo i token.
pub async fn write<G: GmailApi>(ctx: &Ctx<G>, llm: &dyn Llm, req: WriteRequest) -> Result<String> {
    if !llm.available() {
        return Err(Error::other(format!("manca la chiave di {}", llm.config().provider.name())));
    }
    if ctx.db.read_async(|c| over_budget(c)).await? {
        return Err(Error::other("raggiunto il tetto giornaliero dell'AI: riprova domani"));
    }
    let thread = req.thread_id.clone();
    let (context, name) = ctx
        .db
        .read_async(move |c| {
            let context = match &thread {
                Some(id) => thread_context(c, id, 8_000)?,
                None => None,
            };
            Ok((context, crate::store::account(c)?.and_then(|a| a.name)))
        })
        .await?;
    let name = name.unwrap_or_else(|| ctx.me.split('@').next().unwrap_or("").to_owned());
    let reply = llm.chat(write_request(&name, &ctx.me, &req, context.as_deref())).await?;
    let tokens = reply.tokens;
    ctx.db.write_async(move |c| record_usage(c, tokens)).await?;
    tracing::info!(tokens, modo = %req.mode, bozza = !req.notes.trim().is_empty(), "AI: bozza scritta");
    let text = tidy_draft(&reply.text);
    if text.is_empty() {
        return Err(Error::other("l'AI non ha scritto niente: riprova"));
    }
    Ok(text)
}

// ───────────────────────────── sezioni ─────────────────────────────

#[derive(Debug, Clone)]
struct AiSection {
    label_id: String,
    name: String,
    instructions: String,
}

#[derive(Debug, Clone)]
struct Candidate {
    id: String,
    text: String,
}

fn ai_sections(c: &Connection) -> Result<Vec<AiSection>> {
    Ok(c
        .prepare(
            "SELECT s.label_id, l.name, s.instructions FROM sections s JOIN labels l ON l.id = s.label_id
             WHERE s.archived = 0 AND trim(coalesce(s.instructions, '')) <> '' ORDER BY s.position",
        )?
        .query_map([], |r| Ok(AiSection { label_id: r.get(0)?, name: r.get(1)?, instructions: r.get(2)? }))?
        .collect::<Result<_, _>>()?)
}

/// Thread in inbox non ancora valutati per almeno una sezione con istruzioni, dal più recente.
fn unchecked(c: &Connection, limit: usize) -> Result<Vec<Candidate>> {
    let ids: Vec<String> = c
        .prepare(
            "SELECT t.id FROM threads t
             WHERE t.in_inbox = 1 AND EXISTS (
                SELECT 1 FROM sections s WHERE s.archived = 0 AND trim(coalesce(s.instructions, '')) <> ''
                  AND NOT EXISTS (SELECT 1 FROM section_checks k WHERE k.label_id = s.label_id AND k.thread_id = t.id)
                  AND NOT EXISTS (SELECT 1 FROM thread_labels x WHERE x.thread_id = t.id AND x.label_id = s.label_id))
             ORDER BY t.last_date DESC LIMIT ?1",
        )?
        .query_map([limit as i64], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for id in ids {
        let row: Option<(String, String, String, Option<String>)> = c
            .query_row(
                "SELECT t.subject, t.participants, t.snippet, (SELECT summary FROM ai_summaries s WHERE s.thread_id = t.id)
                 FROM threads t WHERE t.id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .ok();
        let Some((subject, participants, snippet, summary)) = row else { continue };
        let people: Vec<serde_json::Value> = serde_json::from_str(&participants).unwrap_or_default();
        let who = people
            .iter()
            .map(|p| format!("{} <{}>", p["name"].as_str().unwrap_or(""), p["email"].as_str().unwrap_or("")))
            .collect::<Vec<_>>()
            .join(", ");
        let body = summary.unwrap_or(snippet);
        out.push(Candidate { text: format!("Da: {who}\nOggetto: {subject}\nContenuto: {}", clip(&body, 400)), id });
    }
    Ok(out)
}

pub fn classify_request(sections: &[(String, String, String)], threads: &[(String, String)]) -> ChatRequest {
    let mut user = String::from("SEZIONI:\n");
    for (id, name, instr) in sections {
        user.push_str(&format!("- id \"{id}\", nome \"{name}\": {instr}\n"));
    }
    user.push_str("\nTHREAD:\n");
    for (id, text) in threads {
        user.push_str(&format!("\n[thread \"{id}\"]\n{text}\n"));
    }
    ChatRequest {
        system: "Assegni thread email alle sezioni definite dall'utente, seguendo le sue istruzioni alla lettera. Un thread \
                 va in una sezione solo se corrisponde chiaramente; nel dubbio, in nessuna. Rispondi solo con un oggetto JSON: \
                 {\"risultati\": [{\"thread\": \"<id>\", \"sezioni\": [\"<id sezione>\", ...]}]}, un elemento per ogni thread."
            .into(),
        user,
        json: true,
        max_tokens: 1200,
    }
}

#[derive(Deserialize)]
struct ClassifyResponse {
    #[serde(default)]
    risultati: Vec<ClassifyItem>,
}
#[derive(Deserialize)]
struct ClassifyItem {
    thread: String,
    #[serde(default)]
    sezioni: Vec<String>,
}

/// Dalla risposta del modello alle etichette da mettere. Le sezioni e i thread sconosciuti si ignorano.
pub fn parse_classification(text: &str, sections: &[String], threads: &[String]) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let Ok(r) = serde_json::from_str::<ClassifyResponse>(text.trim()) else { return out };
    for item in r.risultati {
        if !threads.contains(&item.thread) {
            continue;
        }
        let ok: Vec<String> = item.sezioni.into_iter().filter(|s| sections.contains(s)).collect();
        if !ok.is_empty() {
            out.insert(item.thread, ok);
        }
    }
    out
}

/// Un giro di classificazione: un batch di thread, tutte le sezioni con istruzioni. `Ok(true)` se ha lavorato.
pub async fn classify_once<G: GmailApi>(ctx: &Ctx<G>, llm: &dyn Llm) -> Result<bool> {
    let (sections, candidates) = ctx.db.read_async(|c| Ok((ai_sections(c)?, unchecked(c, CLASSIFY_BATCH)?))).await?;
    if sections.is_empty() || candidates.is_empty() {
        return Ok(false);
    }
    if ctx.db.read_async(|c| over_budget(c)).await? {
        ctx.emit(CoreEvent::AiStatus { state: "budget".into() });
        return Ok(false);
    }
    let sec: Vec<(String, String, String)> = sections.iter().map(|s| (s.label_id.clone(), s.name.clone(), s.instructions.clone())).collect();
    let thr: Vec<(String, String)> = candidates.iter().map(|t| (t.id.clone(), t.text.clone())).collect();
    let reply = llm.chat(classify_request(&sec, &thr)).await?;
    let section_ids: Vec<String> = sections.iter().map(|s| s.label_id.clone()).collect();
    let thread_ids: Vec<String> = candidates.iter().map(|t| t.id.clone()).collect();
    let matches = parse_classification(&reply.text, &section_ids, &thread_ids);
    let n_matched = matches.len();
    let tokens = reply.tokens;
    let changed = ctx
        .db
        .write_async(move |c| {
            record_usage(c, tokens)?;
            let mut changed = Vec::new();
            for t in &thread_ids {
                let got = matches.get(t).cloned().unwrap_or_default();
                for s in &section_ids {
                    c.execute(
                        "INSERT OR REPLACE INTO section_checks (label_id, thread_id, matched, checked_at) VALUES (?1, ?2, ?3, ?4)",
                        params![s, t, got.contains(s), now_ms()],
                    )?;
                }
                for s in got {
                    outbox::enqueue_action(c, std::slice::from_ref(t), &Action::AddLabel { label_id: s })?;
                    changed.push(t.clone());
                }
            }
            Ok(changed)
        })
        .await?;
    tracing::info!(thread = n_matched, valutati = CLASSIFY_BATCH.min(candidates.len()), tokens, "sezioni AI: thread assegnati");
    if !changed.is_empty() {
        ctx.emit(CoreEvent::ThreadsChanged { ids: changed });
        ctx.outbox_notify.notify_one();
    }
    Ok(true)
}

/// Worker delle sezioni: valuta i thread nuovi e, dopo una sezione nuova, quelli già in inbox.
pub async fn run_sections<G: GmailApi>(ctx: Arc<Ctx<G>>, llm: Arc<dyn Llm>) {
    let mut attempt = 0u32;
    loop {
        if !llm.available() {
            ctx.sections_notify.notified().await;
            continue;
        }
        match classify_once(&ctx, llm.as_ref()).await {
            Ok(true) => {
                attempt = 0;
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                attempt += 1;
                tracing::warn!(error = %e, "sezioni AI: classificazione non riuscita");
                tokio::time::sleep(Duration::from_millis(crate::util::backoff_ms(attempt, 300_000))).await;
                continue;
            }
        }
        tokio::select! {
            _ = ctx.sections_notify.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(60)) => {}
        }
    }
}

/// Modello finto per le prove: risposte scritte a mano.
#[cfg(test)]
#[derive(Default)]
pub struct FakeLlm {
    pub replies: Mutex<VecDeque<String>>,
    pub seen: Mutex<Vec<ChatRequest>>,
}

#[cfg(test)]
impl Llm for FakeLlm {
    fn available(&self) -> bool {
        true
    }
    fn config(&self) -> AiConfig {
        AiConfig { provider: Provider::Mistral, model: "finto".into() }
    }
    fn configure(&self, _cfg: AiConfig) {}
    fn has_key(&self, _provider: Provider) -> bool {
        true
    }
    fn chat(&self, req: ChatRequest) -> BoxFut<'_, Result<ChatReply>> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(req);
            let text = self.replies.lock().unwrap().pop_front().unwrap_or_default();
            Ok(ChatReply { text, tokens: 100 })
        })
    }
    fn set_key(&self, _provider: Provider, _key: String) -> BoxFut<'_, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn clear_key(&self, _provider: Provider) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_body_follows_the_provider() {
        let req = ChatRequest { system: "s".into(), user: "u".into(), json: true, max_tokens: 500 };
        let m = request_body(&AiConfig::default(), &req);
        assert_eq!(m["model"], "mistral-small-2603");
        assert_eq!(m["max_tokens"], 500);
        assert_eq!(m["temperature"], 0.2);
        assert_eq!(m["response_format"]["type"], "json_object");
        assert!(m.get("reasoning_effort").is_none());

        let o = request_body(&AiConfig { provider: Provider::Openai, model: "gpt-5.4-mini".into() }, &req);
        assert_eq!(o["max_completion_tokens"], 500);
        assert_eq!(o["reasoning_effort"], "none");
        assert!(o.get("temperature").is_none() && o.get("max_tokens").is_none());

        let sol = request_body(&AiConfig { provider: Provider::Openai, model: "gpt-6.1-sol".into() }, &req);
        assert_eq!(sol["reasoning_effort"], "low");
        assert_eq!(sol["max_completion_tokens"], 4_500);
    }

    #[test]
    fn models_and_replies_are_read_safely() {
        assert!(valid_model("mistral-small-2603") && valid_model("ft:gpt-5.4-mini:acme/x"));
        assert!(!valid_model("") && !valid_model("-x") && !valid_model("a b") && !valid_model(&"a".repeat(101)));
        assert_eq!(reply_text(Some(serde_json::json!("ciao"))), "ciao");
        let blocks = serde_json::json!([{ "type": "thinking", "thinking": [] }, { "type": "text", "text": "ci" }, { "type": "text", "text": "ao" }]);
        assert_eq!(reply_text(Some(blocks)), "ciao");
        assert_eq!(reply_text(None), "");
    }

    #[test]
    fn classification_ignores_unknown_ids_and_bad_json() {
        let secs = vec!["L1".to_owned(), "L2".to_owned()];
        let thr = vec!["t1".to_owned(), "t2".to_owned()];
        let text = r#"{"risultati":[{"thread":"t1","sezioni":["L1","Lx"]},{"thread":"t9","sezioni":["L2"]},{"thread":"t2","sezioni":[]}]}"#;
        let m = parse_classification(text, &secs, &thr);
        assert_eq!(m.len(), 1);
        assert_eq!(m["t1"], vec!["L1".to_owned()]);
        assert!(parse_classification("non è json", &secs, &thr).is_empty());
    }

    #[test]
    fn context_puts_the_last_message_first_in_weight_and_names_attachments() {
        let (_d, db) = crate::db::test_db();
        db.write(|c| {
            crate::store::ensure_account(c, "tommaso@esempio.it", None, None)?;
            let t = crate::store::testdata::thread(
                "t1",
                1,
                vec![
                    crate::store::testdata::msg("m1", "t1", 1_000, "Anna <anna@x.it>", &["INBOX"], Some("Primo messaggio")),
                    crate::store::testdata::msg("m2", "t1", 2_000, "Luca <luca@x.it>", &["INBOX"], Some("Secondo, il più recente")),
                ],
            );
            crate::store::write_thread(c, &t, true, "tommaso@esempio.it")?;
            let ctx = thread_context(c, "t1", 2_000)?.unwrap();
            assert!(ctx.starts_with("Oggetto: Oggetto t1"));
            let first = ctx.find("Primo messaggio").unwrap();
            let last = ctx.find("ULTIMO MESSAGGIO. Da: Luca").unwrap();
            assert!(first < last, "dal più vecchio al più recente");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn write_request_keeps_the_draft_and_cleans_the_answer() {
        let req = WriteRequest {
            mode: "reply".into(),
            thread_id: Some("t1".into()),
            to: vec![crate::mime::Addr { name: Some("Giulio".into()), email: "g@x.it".into() }],
            subject: "Re: Sinistro".into(),
            notes: "servono certificato inail, cid e testimoni".into(),
        };
        let r = write_request("Tommaso Seita", "tommaso@esempio.it", &req, Some("Oggetto: Sinistro\n--- Da: Giulio"));
        assert!(r.user.contains("BOZZA DI Tommaso:\nservono certificato inail"));
        assert!(r.user.contains("THREAD") && r.system.contains("il nome Tommaso"));
        assert_eq!(tidy_draft("```\nOggetto: Re: x\nCiao Giulio,\n\ngrazie\n```"), "Ciao Giulio,\n\ngrazie");
        let empty = WriteRequest { notes: "  ".into(), ..req };
        assert!(write_request("Tommaso Seita", "t@x", &empty, None).user.contains("(vuota)"));
    }
}
