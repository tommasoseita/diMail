//! Dal vecchio identificativo `app.wibo.dimail` (fino al 9 novembre 2026, quando l'app si chiamava
//! Dimail) al nuovo `crate::APP_ID`, senza rifare il login né la prima sync.
//!
//! - Il database non si sposta: la cartella `~/Library/Application Support/Dimail` prende il nome
//!   `diMail` (su un disco che non distingue maiuscole e minuscole è la stessa cartella). Idem per
//!   `~/Library/Logs`.
//! - I dati della WebView (i recenti della palette) passano da `~/Library/WebKit/app.wibo.dimail`.
//! - Nel Portachiavi il refresh token e la chiave di Mistral si copiano nei servizi nuovi e le voci
//!   vecchie si tolgono. Sono di un'altra app per macOS, quindi la prima lettura chiede il permesso
//!   (una volta sola).

use std::path::Path;

use crate::account::Registry;
use crate::auth::keychain;

pub const LEGACY_APP_ID: &str = "app.wibo.dimail";
const LEGACY_FOLDER: &str = "Dimail";

/// Prima della finestra e del log. Restituisce cosa ha fatto, da scrivere nel log subito dopo.
pub fn files() -> Vec<String> {
    let mut notes = Vec::new();
    if std::env::var_os("DIMAIL_DATA_DIR").is_none()
        && let Some(base) = dirs::data_dir()
    {
        rename_folder(&base, &mut notes);
    }
    if let Some(home) = dirs::home_dir() {
        rename_folder(&home.join("Library/Logs"), &mut notes);
        let webkit = home.join("Library/WebKit");
        let (old, new) = (webkit.join(LEGACY_APP_ID), webkit.join(crate::APP_ID));
        if old.is_dir() && !new.exists() {
            match std::fs::rename(&old, &new) {
                Ok(()) => notes.push("migrazione: dati della WebView spostati".into()),
                Err(e) => notes.push(format!("migrazione: dati della WebView non spostati ({e})")),
            }
        }
    }
    notes
}

/// `<parent>/diMail` → `<parent>/diMail`, se c'è la vecchia e non già la nuova.
fn rename_folder(parent: &Path, notes: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(parent) else { return };
    let names: Vec<String> = entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    if !names.iter().any(|n| n == LEGACY_FOLDER) || names.iter().any(|n| n == crate::APP_NAME) {
        return;
    }
    // Passando da un nome temporaneo il cambio di maiuscole funziona su ogni tipo di disco.
    let tmp = parent.join(format!("{}-migrazione", crate::APP_NAME));
    let result = std::fs::rename(parent.join(LEGACY_FOLDER), &tmp).and_then(|()| std::fs::rename(&tmp, parent.join(crate::APP_NAME)));
    match result {
        Ok(()) => notes.push(format!("migrazione: {}/{LEGACY_FOLDER} ora si chiama {}", parent.display(), crate::APP_NAME)),
        Err(e) => notes.push(format!("migrazione: {}/{LEGACY_FOLDER} non rinominata ({e})", parent.display())),
    }
}

/// Nel thread di avvio, prima di leggere il Portachiavi: macOS può chiedere il permesso e aspettare
/// il clic. Si fa una volta sola, segnata in `app.json`, anche se l'utente nega l'accesso: in quel
/// caso rifà il login (i dati restano) o reinserisce la chiave.
pub fn keychain() {
    let mut reg = Registry::load();
    if reg.legacy_keychain_done {
        return;
    }
    let legacy = |name: &str| format!("{LEGACY_APP_ID}.{name}");
    for email in &reg.accounts {
        move_item(&legacy("google-refresh-token"), email, &keychain::service("google-refresh-token"), email, "refresh token");
    }
    move_item(&legacy("mistral-api-key"), "default", &keychain::service(crate::ai::KEY_SERVICE), "mistral", "chiave di Mistral");
    reg.legacy_keychain_done = true;
    if let Err(e) = reg.save() {
        tracing::warn!(error = %e, "migrazione: app.json non salvato");
    }
}

fn move_item(old_service: &str, old_account: &str, new_service: &str, new_account: &str, what: &str) {
    if matches!(keychain::get(new_service, new_account), Ok(Some(_))) {
        return;
    }
    match keychain::get(old_service, old_account) {
        Ok(Some(value)) => {
            if let Err(e) = keychain::set(new_service, new_account, &value) {
                tracing::warn!(error = %e, cosa = what, "migrazione: voce del Portachiavi non copiata");
                return;
            }
            if let Err(e) = keychain::delete(old_service, old_account) {
                tracing::warn!(error = %e, cosa = what, "migrazione: voce vecchia del Portachiavi rimasta");
            }
            tracing::info!(cosa = what, "migrazione: voce del Portachiavi spostata");
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, cosa = what, "migrazione: voce vecchia del Portachiavi non letta"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_the_old_folder_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Dimail/accounts")).unwrap();
        std::fs::write(dir.path().join("Dimail/app.json"), b"{}").unwrap();
        let mut notes = Vec::new();
        rename_folder(dir.path(), &mut notes);
        let names: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["diMail".to_owned()]);
        assert!(dir.path().join("diMail/accounts").is_dir() && dir.path().join("diMail/app.json").is_file());
        // La seconda volta non fa niente.
        notes.clear();
        rename_folder(dir.path(), &mut notes);
        assert!(notes.is_empty());
    }
}
