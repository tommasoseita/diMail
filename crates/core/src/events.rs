use serde::Serialize;

use crate::store::SyncStatus;

/// Eventi dal nucleo verso l'interfaccia. L'interfaccia rilegge da SQLite quando li riceve.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreEvent {
    /// Thread scritti, modificati o eliminati in locale.
    ThreadsChanged { ids: Vec<String> },
    LabelsChanged,
    SyncStatus(SyncStatus),
    /// Gmail ha rifiutato un'azione in modo definitivo: lo stato locale torna a quello di Gmail.
    ActionFailed { thread_id: Option<String>, message: String },
    /// Serve un nuovo login. I dati locali restano.
    AuthRequired { email: String },
    /// Avanzamento di uno sgombero in blocco su Gmail.
    BulkStatus { op_id: i64, kind: String, state: String, done: i64, total: i64 },
    /// Stato di un messaggio nella coda di invio: sent, retrying, failed.
    SendStatus { id: i64, state: String, error: Option<String> },
    /// Stato dell'AI: budget (tetto giornaliero raggiunto).
    AiStatus { state: String },
}
