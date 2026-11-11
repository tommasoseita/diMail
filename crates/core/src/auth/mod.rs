//! OAuth 2.0 per app installate, con PKCE e redirect su loopback (sez. 4).
//!
//! Il login si apre nel browser di sistema. Il "client secret" di un'app desktop non è
//! davvero segreto: la protezione la dà PKCE.

pub mod keychain;

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use url::Url;

use crate::util::{b64url, b64url_decode, random_bytes};
use crate::{Error, Result};

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

pub const SCOPE_GMAIL_MODIFY: &str = "https://www.googleapis.com/auth/gmail.modify";

/// Solo questi. `gmail.settings.basic` non serve: `gmail.modify` basta per alias e firme.
pub const SCOPES: &[&str] = &["openid", "email", "profile", SCOPE_GMAIL_MODIFY];

/// Quanto aspettare che l'utente completi il login nel browser.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Rinnova l'access token se scade entro questo margine.
const REFRESH_MARGIN: Duration = Duration::from_secs(120);

/// Il client OAuth di tipo "App desktop" con cui diMail parla con Google. In modalità personale lo
/// crea l'utente nella console di Google Cloud (docs/it/google-client.md) e ne carica il JSON.
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: String,
    /// Il progetto di Google Cloud, solo per mostrarlo nelle impostazioni.
    #[serde(default)]
    pub project_id: Option<String>,
}

impl OAuthClient {
    /// Legge il JSON scaricato dalla console di Google Cloud ("client_secret_….json").
    pub fn from_google_json(raw: &str) -> Result<OAuthClient> {
        let v: serde_json::Value = serde_json::from_str(raw.trim_start_matches('\u{feff}'))
            .map_err(|_| Error::other("il file non è un JSON: scarica quello del client dalla console di Google Cloud"))?;
        if v.get("web").is_some() {
            return Err(Error::other(
                "questo è il JSON di un client «Applicazione web»: diMail ha bisogno di un client di tipo «App desktop»",
            ));
        }
        let installed = v
            .get("installed")
            .ok_or_else(|| Error::other("nel JSON manca il client: serve quello di un client OAuth di tipo «App desktop»"))?;
        let field = |k: &str| installed.get(k).and_then(|x| x.as_str()).map(str::trim).filter(|x| !x.is_empty());
        let client_id = field("client_id").ok_or_else(|| Error::other("nel JSON manca client_id"))?;
        if !client_id.ends_with(".apps.googleusercontent.com") || client_id.len() > 200 {
            return Err(Error::other("client_id non sembra di Google (finisce con .apps.googleusercontent.com)"));
        }
        let client_secret = field("client_secret").ok_or_else(|| Error::other("nel JSON manca client_secret"))?;
        Ok(OAuthClient {
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
            project_id: field("project_id").map(str::to_owned),
        })
    }
}

/// Link di configurazione (11 novembre): `dimail://client?c=…`, con dentro il client. Chi ha creato
/// il client lo copia dalle Impostazioni e lo manda ai colleghi; aprirlo configura diMail senza
/// scaricare né scegliere il file JSON. Contiene il `client_secret`, come il file: per un'app desktop
/// Google non lo considera un segreto, ma il link si condivide solo con chi deve usarlo.
const SETUP_LINK: &str = "dimail://client";

impl OAuthClient {
    pub fn setup_link(&self) -> String {
        let payload = serde_json::json!({ "i": self.client_id, "s": self.client_secret, "p": self.project_id });
        format!("{SETUP_LINK}?c={}", b64url(payload.to_string().as_bytes()))
    }

    pub fn from_setup_link(link: &str) -> Result<OAuthClient> {
        let bad = || Error::other("non è un link di configurazione di diMail");
        let link = link.trim();
        if link.len() > 4000 {
            return Err(bad());
        }
        let rest = link.strip_prefix(SETUP_LINK).ok_or_else(bad)?;
        let query = rest.trim_start_matches('/').strip_prefix('?').ok_or_else(bad)?;
        let code = query.split('&').find_map(|kv| kv.strip_prefix("c=")).ok_or_else(bad)?;
        let raw = b64url_decode(code).map_err(|_| bad())?;
        let v: serde_json::Value = serde_json::from_slice(&raw).map_err(|_| bad())?;
        // Gli stessi controlli del file JSON.
        let as_file = serde_json::json!({ "installed": { "client_id": v.get("i"), "client_secret": v.get("s"), "project_id": v.get("p") } });
        OAuthClient::from_google_json(&as_file.to_string())
    }
}

impl std::fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthClient").field("client_id", &self.client_id).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct Identity {
    pub email: String,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub hosted_domain: Option<String>,
}

pub struct LoginResult {
    pub identity: Identity,
    pub refresh_token: String,
    pub access_token: String,
    pub expires_at: Instant,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    id_token: Option<String>,
    scope: Option<String>,
}

#[derive(Deserialize)]
struct TokenError {
    error: String,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct IdClaims {
    email: String,
    name: Option<String>,
    picture: Option<String>,
    hd: Option<String>,
}

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn new() -> Self {
        // 32 byte casuali → 43 caratteri, dentro i limiti di RFC 7636 (43-128).
        let verifier = b64url(&random_bytes::<32>());
        let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
        Pkce { verifier, challenge }
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}

pub fn authorization_url(client: &OAuthClient, redirect_uri: &str, pkce: &Pkce, state: &str, login_hint: Option<&str>) -> Url {
    let scope = SCOPES.join(" ");
    let mut url = Url::parse_with_params(
        AUTH_URL,
        &[
            ("client_id", client.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", scope.as_str()),
            ("code_challenge", pkce.challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state),
            ("access_type", "offline"),
            // Garantisce un refresh token anche se l'utente aveva già dato il consenso.
            ("prompt", "consent"),
        ],
    )
    .expect("URL di autorizzazione valido");
    if let Some(hint) = login_hint {
        url.query_pairs_mut().append_pair("login_hint", hint);
    }
    url
}

/// Esegue il login completo: apre il browser, aspetta il redirect su loopback, scambia il codice.
/// `open_url` apre l'URL nel browser di sistema.
pub async fn login(
    http: &reqwest::Client,
    client: &OAuthClient,
    login_hint: Option<&str>,
    open_url: impl FnOnce(&str) -> Result<()>,
) -> Result<LoginResult> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| Error::Auth(format!("impossibile aprire la porta locale: {e}")))?;
    let port = listener.local_addr().map_err(|e| Error::Auth(e.to_string()))?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let pkce = Pkce::new();
    let state = b64url(&random_bytes::<16>());
    let url = authorization_url(client, &redirect_uri, &pkce, &state, login_hint);
    open_url(url.as_str())?;

    let code = tokio::time::timeout(LOGIN_TIMEOUT, wait_for_code(&listener, &state))
        .await
        .map_err(|_| Error::Auth("tempo scaduto: il login nel browser non è stato completato".into()))??;
    drop(listener);

    let resp = exchange_code(http, client, &code, &redirect_uri, &pkce.verifier).await?;

    let granted = resp.scope.as_deref().unwrap_or_default();
    if !granted.split_whitespace().any(|s| s == SCOPE_GMAIL_MODIFY) {
        return Err(Error::Auth(
            "non è stato concesso l'accesso a Gmail: rifai il login e lascia selezionata la casella di Gmail".into(),
        ));
    }
    let refresh_token = resp
        .refresh_token
        .ok_or_else(|| Error::Auth("Google non ha restituito un refresh token".into()))?;
    let id_token = resp
        .id_token
        .ok_or_else(|| Error::Auth("Google non ha restituito l'identità dell'account".into()))?;
    let identity = decode_id_token(&id_token)?;

    Ok(LoginResult {
        identity,
        refresh_token,
        access_token: resp.access_token,
        expires_at: Instant::now() + Duration::from_secs(resp.expires_in),
    })
}

/// Il token arriva direttamente dall'endpoint di Google su TLS, quindi basta leggerne il payload
/// (OpenID Connect Core, 3.1.3.7).
pub fn decode_id_token(id_token: &str) -> Result<Identity> {
    let payload = id_token
        .split('.')
        .nth(1)
        .ok_or_else(|| Error::Auth("id_token malformato".into()))?;
    let claims: IdClaims = serde_json::from_slice(&b64url_decode(payload)?)?;
    Ok(Identity {
        email: claims.email.to_lowercase(),
        name: claims.name,
        picture: claims.picture,
        hosted_domain: claims.hd,
    })
}

const PAGE_OK: &str = "<!doctype html><meta charset=utf-8><title>diMail</title>\
<body style=\"font:16px -apple-system,system-ui;margin:20vh auto;max-width:28em;text-align:center\">\
<h2>Accesso completato</h2><p>Puoi chiudere questa scheda e tornare a diMail.</p></body>";
const PAGE_ERR: &str = "<!doctype html><meta charset=utf-8><title>diMail</title>\
<body style=\"font:16px -apple-system,system-ui;margin:20vh auto;max-width:28em;text-align:center\">\
<h2>Accesso non completato</h2><p>Torna a diMail e riprova.</p></body>";

async fn wait_for_code(listener: &TcpListener, expected_state: &str) -> Result<String> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| Error::Auth(format!("connessione locale fallita: {e}")))?;

        let mut buf = Vec::with_capacity(2048);
        let mut chunk = [0u8; 2048];
        // Basta la riga della richiesta; limitiamo la lettura per sicurezza.
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
            let n = match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            buf.extend_from_slice(&chunk[..n]);
        }
        let request = String::from_utf8_lossy(&buf);
        let target = request
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("GET "))
            .and_then(|l| l.split_whitespace().next())
            .unwrap_or("/");
        let parsed = Url::parse(&format!("http://127.0.0.1{target}")).ok();
        let param = |k: &str| {
            parsed
                .as_ref()
                .and_then(|u| u.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned()))
        };

        let (code, error, state) = (param("code"), param("error"), param("state"));
        if code.is_none() && error.is_none() {
            // Es. /favicon.ico: non è il redirect.
            let _ = respond(&mut stream, "404 Not Found", "").await;
            continue;
        }
        if state.as_deref() != Some(expected_state) {
            let _ = respond(&mut stream, "400 Bad Request", PAGE_ERR).await;
            return Err(Error::Auth("risposta di login non valida (state diverso)".into()));
        }
        if let Some(err) = error {
            let _ = respond(&mut stream, "200 OK", PAGE_ERR).await;
            return Err(Error::Auth(if err == "access_denied" {
                "accesso negato nel browser".into()
            } else {
                format!("Google ha risposto: {err}")
            }));
        }
        let _ = respond(&mut stream, "200 OK", PAGE_OK).await;
        return Ok(code.expect("controllato sopra"));
    }
}

async fn respond(stream: &mut tokio::net::TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.shutdown().await
}

async fn exchange_code(
    http: &reqwest::Client,
    client: &OAuthClient,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<TokenResponse> {
    let resp = http
        .post(TOKEN_URL)
        .form(&[
            ("code", code),
            ("client_id", client.client_id.as_str()),
            ("client_secret", client.client_secret.as_str()),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier),
        ])
        .send()
        .await?;
    parse_token_response(resp).await
}

async fn parse_token_response(resp: reqwest::Response) -> Result<TokenResponse> {
    let status = resp.status();
    let bytes = resp.bytes().await?;
    if status.is_success() {
        return Ok(serde_json::from_slice(&bytes)?);
    }
    let err: Option<TokenError> = serde_json::from_slice(&bytes).ok();
    match err {
        Some(e) if e.error == "invalid_grant" => Err(Error::AuthRevoked),
        Some(e) => Err(Error::Auth(format!(
            "{} {}",
            e.error,
            e.error_description.unwrap_or_default()
        ))),
        None if status.is_server_error() => Err(Error::Network(format!("token endpoint {status}"))),
        None => Err(Error::Auth(format!("token endpoint {status}"))),
    }
}

struct CachedToken {
    token: String,
    expires_at: Instant,
}

/// Tiene l'access token in memoria e lo rinnova da solo prima della scadenza.
pub struct TokenManager {
    http: reqwest::Client,
    client: OAuthClient,
    refresh_token: String,
    cached: Mutex<Option<CachedToken>>,
}

impl TokenManager {
    pub fn new(http: reqwest::Client, client: OAuthClient, refresh_token: String) -> Arc<Self> {
        Arc::new(TokenManager { http, client, refresh_token, cached: Mutex::new(None) })
    }

    pub async fn seed(&self, access_token: String, expires_at: Instant) {
        *self.cached.lock().await = Some(CachedToken { token: access_token, expires_at });
    }

    pub async fn access_token(&self) -> Result<String> {
        let mut guard = self.cached.lock().await;
        if let Some(c) = guard.as_ref()
            && c.expires_at > Instant::now() + REFRESH_MARGIN
        {
            return Ok(c.token.clone());
        }
        let resp = self
            .http
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", self.refresh_token.as_str()),
                ("client_id", self.client.client_id.as_str()),
                ("client_secret", self.client.client_secret.as_str()),
            ])
            .send()
            .await?;
        let tok = parse_token_response(resp).await?;
        let token = tok.access_token.clone();
        *guard = Some(CachedToken { token: tok.access_token, expires_at: Instant::now() + Duration::from_secs(tok.expires_in) });
        tracing::debug!("access token rinnovato");
        Ok(token)
    }

    /// Dopo un 401: il prossimo `access_token()` ne chiede uno nuovo.
    pub async fn invalidate(&self) {
        *self.cached.lock().await = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_link_round_trips_and_refuses_other_links() {
        let c = OAuthClient { client_id: "123-abc.apps.googleusercontent.com".into(), client_secret: "GOCSPX-prova".into(), project_id: Some("posta-mia".into()) };
        let link = c.setup_link();
        assert!(link.starts_with("dimail://client?c="));
        assert!(!link.contains("GOCSPX"), "il segreto non è leggibile a occhio nel link");
        let back = OAuthClient::from_setup_link(&format!("  {link}\n")).unwrap();
        assert_eq!((back.client_id, back.client_secret, back.project_id), (c.client_id.clone(), c.client_secret.clone(), c.project_id.clone()));
        // macOS può aggiungere una barra prima della query.
        assert!(OAuthClient::from_setup_link(&link.replace("client?", "client/?")).is_ok());
        let no_project = OAuthClient { project_id: None, ..c.clone() };
        assert_eq!(OAuthClient::from_setup_link(&no_project.setup_link()).unwrap().project_id, None);
        for bad in ["https://esempio.it", "dimail://client", "dimail://client?c=", "dimail://client?c=%%%", "dimail://altro?c=abc"] {
            assert!(OAuthClient::from_setup_link(bad).is_err(), "{bad}");
        }
        // Un client che non è di Google non passa.
        let fake = OAuthClient { client_id: "123.example.com".into(), ..c };
        assert!(OAuthClient::from_setup_link(&fake.setup_link()).is_err());
    }

    #[test]
    fn reads_the_desktop_client_json_and_refuses_the_others() {
        let raw = r#"{"installed":{"client_id":"123-abc.apps.googleusercontent.com","project_id":"posta-mia","auth_uri":"https://accounts.google.com/o/oauth2/auth","client_secret":"GOCSPX-prova","redirect_uris":["http://localhost"]}}"#;
        let c = OAuthClient::from_google_json(raw).unwrap();
        assert_eq!(c.client_id, "123-abc.apps.googleusercontent.com");
        assert_eq!(c.client_secret, "GOCSPX-prova");
        assert_eq!(c.project_id.as_deref(), Some("posta-mia"));
        let web = r#"{"web":{"client_id":"1-a.apps.googleusercontent.com","client_secret":"x"}}"#;
        assert!(OAuthClient::from_google_json(web).unwrap_err().to_string().contains("App desktop"));
        assert!(OAuthClient::from_google_json("non json").is_err());
        assert!(OAuthClient::from_google_json(r#"{"installed":{"client_id":"abc","client_secret":"x"}}"#).is_err());
        assert!(OAuthClient::from_google_json(r#"{"installed":{"client_id":"1-a.apps.googleusercontent.com"}}"#).is_err());
        // Il segreto non finisce nei log.
        assert!(!format!("{c:?}").contains("GOCSPX"));
    }

    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let p = Pkce::new();
        assert_eq!(p.verifier.len(), 43);
        assert_eq!(p.challenge, b64url(&Sha256::digest(p.verifier.as_bytes())));
        // Vettore di RFC 7636, appendice B.
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(b64url(&Sha256::digest(v.as_bytes())), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn auth_url_has_only_the_allowed_scopes() {
        let c = OAuthClient { client_id: "id".into(), client_secret: "s".into(), project_id: None };
        let url = authorization_url(&c, "http://127.0.0.1:1234", &Pkce::new(), "st", None);
        let scope = url.query_pairs().find(|(k, _)| k == "scope").unwrap().1.into_owned();
        assert_eq!(scope, "openid email profile https://www.googleapis.com/auth/gmail.modify");
        assert!(!url.as_str().contains("settings"));
        assert!(url.query_pairs().any(|(k, v)| k == "code_challenge_method" && v == "S256"));
    }

    #[test]
    fn decodes_id_token_payload() {
        let payload = b64url(br#"{"email":"Tommaso@Esempio.it","name":"Tommaso","hd":"esempio.it"}"#);
        let id = decode_id_token(&format!("xx.{payload}.yy")).unwrap();
        assert_eq!(id.email, "tommaso@esempio.it");
        assert_eq!(id.hosted_domain.as_deref(), Some("esempio.it"));
    }

    #[tokio::test]
    async fn loopback_ignores_favicon_and_checks_state() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move { wait_for_code(&listener, "abc").await });

        let send = |path: &'static str| async move {
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).await.unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).await.unwrap();
            out
        };
        assert!(send("/favicon.ico").await.starts_with("HTTP/1.1 404"));
        assert!(send("/?code=4%2Fxyz&state=abc&scope=email").await.contains("Accesso completato"));
        assert_eq!(task.await.unwrap().unwrap(), "4/xyz");
    }
}
