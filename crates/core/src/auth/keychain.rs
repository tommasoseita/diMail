//! Portachiavi di macOS (sez. 3 e 4): refresh token di Google, client OAuth, chiavi dell'AI.
//! L'access token non passa mai di qui: resta solo in memoria.
//!
//! Ogni servizio ha il prefisso dell'identificativo dell'app (`crate::APP_ID`), per esempio
//! `io.github.tommasoseita.dimail.google-refresh-token`.

use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

use super::OAuthClient;
use crate::{Error, Result};

const REFRESH_TOKEN: &str = "google-refresh-token";
const OAUTH_CLIENT: &str = "google-oauth-client";
const OAUTH_CLIENT_ACCOUNT: &str = "default";
/// errSecItemNotFound
const NOT_FOUND: i32 = -25300;

/// Il nome completo del servizio: `<APP_ID>.<nome>`.
pub fn service(name: &str) -> String {
    format!("{}.{name}", crate::APP_ID)
}

/// Legge una voce. `Ok(None)` se non c'è.
pub fn get(service: &str, account: &str) -> Result<Option<Vec<u8>>> {
    match get_generic_password(service, account) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.code() == NOT_FOUND => Ok(None),
        Err(e) => Err(Error::other(format!("Portachiavi: lettura non riuscita ({e})"))),
    }
}

pub fn set(service: &str, account: &str, value: &[u8]) -> Result<()> {
    set_generic_password(service, account, value).map_err(|e| Error::other(format!("Portachiavi: salvataggio non riuscito ({e})")))
}

/// Toglie una voce; se non c'è, va bene lo stesso.
pub fn delete(service: &str, account: &str) -> Result<()> {
    match delete_generic_password(service, account) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == NOT_FOUND => Ok(()),
        Err(e) => Err(Error::other(format!("Portachiavi: eliminazione non riuscita ({e})"))),
    }
}

pub fn save_refresh_token(email: &str, token: &str) -> Result<()> {
    set(&service(REFRESH_TOKEN), &email.to_lowercase(), token.as_bytes())
}

pub fn load_refresh_token(email: &str) -> Result<Option<String>> {
    match get(&service(REFRESH_TOKEN), &email.to_lowercase())? {
        Some(bytes) => String::from_utf8(bytes).map(Some).map_err(|_| Error::other("Portachiavi: token non valido")),
        None => Ok(None),
    }
}

pub fn delete_refresh_token(email: &str) -> Result<()> {
    delete(&service(REFRESH_TOKEN), &email.to_lowercase())
}

/// Il client OAuth caricato dall'utente (modalità personale), come JSON.
pub fn save_client(client: &OAuthClient) -> Result<()> {
    set(&service(OAUTH_CLIENT), OAUTH_CLIENT_ACCOUNT, &serde_json::to_vec(client)?)
}

pub fn load_client() -> Result<Option<OAuthClient>> {
    match get(&service(OAUTH_CLIENT), OAUTH_CLIENT_ACCOUNT)? {
        Some(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|_| Error::other("Portachiavi: client Google non valido")),
        None => Ok(None),
    }
}
