use std::time::Duration;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),

    /// Rete assente o irraggiungibile. Transitorio: si riprova.
    #[error("rete non disponibile: {0}")]
    Network(String),

    /// Il refresh token non vale più (revoca, cambio password). Serve un nuovo login;
    /// i dati locali restano.
    #[error("l'accesso a Google è scaduto o è stato revocato: rifai il login")]
    AuthRevoked,

    #[error("login: {0}")]
    Auth(String),

    /// Errore restituito da Gmail.
    #[error("Gmail ha risposto {status}: {message}")]
    Api {
        status: u16,
        reason: Option<String>,
        message: String,
        retry_after: Option<Duration>,
    },

    #[error("non trovato su Gmail")]
    NotFound,

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }

    /// Errori per cui ha senso riprovare più tardi.
    pub fn is_transient(&self) -> bool {
        match self {
            Error::Network(_) => true,
            Error::Api { status, reason, .. } => is_retryable_status(*status, reason.as_deref()),
            _ => false,
        }
    }

    pub fn is_rate_limit(&self) -> bool {
        matches!(self, Error::Api { status, reason, .. } if *status == 429 || (*status == 403 && is_rate_reason(reason.as_deref())))
    }
}

pub(crate) fn is_rate_reason(reason: Option<&str>) -> bool {
    matches!(
        reason,
        Some("rateLimitExceeded" | "userRateLimitExceeded" | "quotaExceeded" | "concurrentLimitExceeded")
    )
}

pub(crate) fn is_retryable_status(status: u16, reason: Option<&str>) -> bool {
    match status {
        429 | 500 | 502 | 503 | 504 => true,
        403 => is_rate_reason(reason),
        _ => false,
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        // Mai includere URL con dati o token: reqwest non mette il corpo nell'errore,
        // e togliamo comunque l'URL.
        Error::Network(e.without_url().to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Other(format!("JSON non valido: {e}"))
    }
}

impl From<tokio::task::JoinError> for Error {
    fn from(e: tokio::task::JoinError) -> Self {
        Error::Other(format!("task interrotto: {e}"))
    }
}

impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
