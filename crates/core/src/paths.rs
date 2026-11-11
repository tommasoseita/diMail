//! Dove stanno i file di diMail.
//!
//! - Dati: `~/Library/Application Support/diMail/` (un database per account, sez. 13)
//! - Log: `~/Library/Logs/diMail/`
//!
//! `DIMAIL_DATA_DIR` sostituisce la cartella dei dati (utile per i test a mano).

use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DIMAIL_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .expect("cartella Application Support non trovata")
        .join(crate::APP_NAME)
}

pub fn logs_dir() -> PathBuf {
    dirs::home_dir()
        .expect("cartella home non trovata")
        .join("Library/Logs").join(crate::APP_NAME)
}

pub fn accounts_dir() -> PathBuf {
    data_dir().join("accounts")
}

pub fn account_dir(email: &str) -> PathBuf {
    accounts_dir().join(email.trim().to_lowercase())
}

pub fn account_db(email: &str) -> PathBuf {
    account_dir(email).join("mail.db")
}

pub fn attachments_cache_dir(email: &str) -> PathBuf {
    account_dir(email).join("attachments")
}
