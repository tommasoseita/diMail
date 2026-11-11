//! Nucleo di diMail.
//!
//! Regola della spec (sez. 3): l'interfaccia legge solo da SQLite, tramite il nucleo.
//! Ogni azione si scrive prima nel database locale e poi parte verso Gmail in background.
//!
//! Questo crate non dipende da Tauri, così si può riusare sul futuro backend (sez. 15).

pub mod account;
pub mod ai;
pub mod attachments;
pub mod auth;
pub mod bulk;
pub mod db;
pub mod error;
pub mod events;
pub mod gmail;
pub mod jobs;
pub mod migrate;
pub mod mime;
pub mod outbox;
pub mod people;
pub mod send;
pub mod paths;
pub mod sanitize;
pub mod sections;
pub mod split;
pub mod store;
pub mod sync;
#[cfg(test)]
mod sync_tests;
pub mod util;

/// Il nome dell'app, come lo vede l'utente.
pub const APP_NAME: &str = "diMail";

/// Identificativo del bundle (dal 9 novembre 2026; prima era `app.wibo.dimail`, vedi `migrate`).
/// Fa da prefisso ai servizi del Portachiavi. Non va più cambiato: macOS lega a questo il
/// Portachiavi, i permessi e le preferenze.
pub const APP_ID: &str = "io.github.tommasoseita.dimail";

pub use account::AccountRuntime;
pub use reqwest;
pub use error::{Error, Result};
pub use events::CoreEvent;
