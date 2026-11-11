//! Il client OAuth non sta più nel binario: ognuno carica il proprio JSON al primo avvio
//! (docs/it/google-client.md), o lo riceve dalla sua organizzazione (progetto delle organizzazioni).

fn main() {
    tauri_build::build()
}
