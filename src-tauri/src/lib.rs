//! Guscio Tauri: espone il nucleo all'interfaccia con comandi IPC e inoltra gli eventi.
//! L'interfaccia non chiama mai Gmail direttamente (sez. 3).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use dimail_core::account::{self, AccountRuntime, Queued, Registry, SnoozeHandle};
use dimail_core::ai::{AiConfig, AiStatus, ModelInfo, Provider, WriteRequest};
use dimail_core::jobs::Snoozed;
use dimail_core::mime::Addr;
use dimail_core::sections::Section;
use dimail_core::send::{Draft, OutAttachment, ReplyMode, Signature};
use dimail_core::bulk::BulkPreview;
use dimail_core::people::ContactCard;
use dimail_core::split::{Remember, SplitRule};
use dimail_core::auth::{OAuthClient, keychain};
use dimail_core::events::CoreEvent;
use dimail_core::outbox::Action;
use dimail_core::store::{AccountRow, LabelRow, MyDomains, SyncStatus, ThreadDetail, ThreadSummary, View};
use dimail_core::sync::SyncConfig;
use dimail_core::{Error, paths};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{Mutex, broadcast};

type CmdResult<T> = Result<T, Error>;

struct AppState {
    http: dimail_core::reqwest::Client,
    /// Il client OAuth caricato dall'utente (Portachiavi). Finché manca, l'app chiede il JSON.
    client: RwLock<Option<OAuthClient>>,
    events: broadcast::Sender<CoreEvent>,
    runtime: RwLock<Option<Arc<AccountRuntime>>>,
    /// Account registrato ma senza token nel Portachiavi (o token revocato).
    pending_email: RwLock<Option<String>>,
    login_lock: Mutex<()>,
    /// L'account si sta riprendendo: la lettura dal Portachiavi può aspettare il clic dell'utente.
    starting: AtomicBool,
}

impl AppState {
    fn rt(&self) -> CmdResult<Arc<AccountRuntime>> {
        self.runtime
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| Error::other("nessun account collegato"))
    }

    fn client(&self) -> CmdResult<OAuthClient> {
        self.client
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| Error::other("manca il client Google: caricane il JSON"))
    }

    fn set_runtime(&self, rt: Option<Arc<AccountRuntime>>) {
        let old = std::mem::replace(&mut *self.runtime.write().unwrap_or_else(|p| p.into_inner()), rt);
        if let Some(old) = old {
            old.shutdown();
        }
    }
}

#[derive(Serialize)]
struct AppInfo {
    /// L'account si sta ancora riprendendo: l'interfaccia aspetta l'evento `app_ready`.
    starting: bool,
    account: Option<AccountRow>,
    needs_login: bool,
    login_hint: Option<String>,
    /// Manca il client Google (primo avvio, o dopo l'aggiornamento dalla vecchia Dimail): schermata del JSON.
    needs_client: bool,
}

#[tauri::command]
async fn app_info(state: State<'_, AppState>) -> CmdResult<AppInfo> {
    let starting = state.starting.load(Ordering::SeqCst);
    let rt = state.runtime.read().unwrap_or_else(|p| p.into_inner()).clone();
    let hint = state.pending_email.read().unwrap_or_else(|p| p.into_inner()).clone();
    let needs_client = !starting && state.client.read().unwrap_or_else(|p| p.into_inner()).is_none();
    match rt {
        Some(rt) => Ok(AppInfo {
            starting,
            account: rt.account().await?,
            needs_login: rt.ctx().is_auth_required(),
            login_hint: Some(rt.email().to_owned()),
            needs_client,
        }),
        None => Ok(AppInfo { starting, account: None, needs_login: !starting, login_hint: hint, needs_client }),
    }
}

/// Errori e messaggi dell'interfaccia nel log dell'app: nella build di release la console della
/// WebView non si vede. L'interfaccia non manda mai contenuto delle email.
#[tauri::command]
fn log_frontend(level: String, message: String) {
    let message: String = message.chars().take(2_000).collect();
    match level.as_str() {
        "error" => tracing::error!(target: "interfaccia", "{message}"),
        "warn" => tracing::warn!(target: "interfaccia", "{message}"),
        _ => tracing::info!(target: "interfaccia", "{message}"),
    }
}

#[tauri::command]
async fn login(app: AppHandle, state: State<'_, AppState>) -> CmdResult<AccountRow> {
    let _guard = state
        .login_lock
        .try_lock()
        .map_err(|_| Error::other("login già in corso: completalo nel browser"))?;
    let hint = state.pending_email.read().unwrap_or_else(|p| p.into_inner()).clone();
    let client = state.client()?;
    let opener = app.clone();
    let rt = account::connect(
        &state.http,
        &client,
        hint.as_deref(),
        move |url| opener.opener().open_url(url, None::<&str>).map_err(|e| Error::other(format!("browser: {e}"))),
        state.events.clone(),
        SyncConfig::default(),
    )
    .await?;
    let acc = rt.account().await?.ok_or_else(|| Error::other("account non salvato"))?;
    state.set_runtime(Some(rt));
    *state.pending_email.write().unwrap_or_else(|p| p.into_inner()) = None;
    Ok(acc)
}

// ───────────── Client Google (modalità personale) ─────────────

#[derive(Serialize)]
struct GoogleClientInfo {
    client_id: String,
    project_id: Option<String>,
}

fn client_info(c: &OAuthClient) -> GoogleClientInfo {
    GoogleClientInfo { client_id: c.client_id.clone(), project_id: c.project_id.clone() }
}

#[tauri::command]
fn google_client(state: State<'_, AppState>) -> Option<GoogleClientInfo> {
    state.client.read().unwrap_or_else(|p| p.into_inner()).as_ref().map(client_info)
}

/// Carica il JSON del client: dal percorso (trascinato sulla finestra) o scelto nel pannello di macOS.
/// `Ok(None)` se l'utente annulla. Se c'era un account in attesa del client, riparte subito.
#[tauri::command]
async fn load_google_client(window: tauri::WebviewWindow, state: State<'_, AppState>, path: Option<String>) -> CmdResult<Option<GoogleClientInfo>> {
    let path = match path {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            use tauri_plugin_dialog::DialogExt;
            let (tx, rx) = tokio::sync::oneshot::channel();
            window
                .dialog()
                .file()
                .set_parent(&window)
                .set_title("Il JSON del client Google")
                .add_filter("JSON", &["json"])
                .pick_file(move |f| {
                    let _ = tx.send(f);
                });
            match rx.await.ok().flatten().and_then(|f| f.into_path().ok()) {
                Some(p) => p,
                None => return Ok(None),
            }
        }
    };
    let meta = std::fs::metadata(&path).map_err(|e| Error::other(format!("il file non si legge: {e}")))?;
    if !meta.is_file() || meta.len() > 64 * 1024 {
        return Err(Error::other("non è il JSON di un client Google (troppo grande o non è un file)"));
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| Error::other(format!("il file non si legge: {e}")))?;
    let client = OAuthClient::from_google_json(&raw)?;
    apply_client(&state, client).await.map(Some)
}

/// Salva il client nel Portachiavi e lo mette in uso. Se c'era un account in attesa del client
/// riparte subito; con un client diverso da prima serve un nuovo accesso.
async fn apply_client(state: &AppState, client: OAuthClient) -> CmdResult<GoogleClientInfo> {
    let previous = state.client.read().unwrap_or_else(|p| p.into_inner()).clone();
    let changed = previous.as_ref().is_some_and(|p| p.client_id != client.client_id);
    keychain::save_client(&client)?;
    tracing::info!(cambiato = changed, "client Google caricato");
    *state.client.write().unwrap_or_else(|p| p.into_inner()) = Some(client.clone());
    let info = client_info(&client);

    // Un account già collegato che aspettava il client (aggiornamento dalla vecchia Dimail): riparte senza login.
    let pending = state.pending_email.read().unwrap_or_else(|p| p.into_inner()).clone();
    if let Some(email) = pending
        && state.runtime.read().unwrap_or_else(|p| p.into_inner()).is_none()
    {
        let (http, events) = (state.http.clone(), state.events.clone());
        let resumed = tauri::async_runtime::spawn_blocking(move || account::resume(&http, &client, &email, events, SyncConfig::default()))
            .await
            .map_err(|e| Error::other(e.to_string()))?;
        match resumed {
            Ok(Some(rt)) => {
                tracing::info!("account ripreso dopo il client");
                state.set_runtime(Some(rt));
                *state.pending_email.write().unwrap_or_else(|p| p.into_inner()) = None;
            }
            Ok(None) => {}
            Err(e) => tracing::error!(error = %e, "account non ripreso dopo il client"),
        }
    } else if changed {
        // Un altro client: il refresh token valeva per quello vecchio, serve un nuovo accesso.
        if let Some(rt) = state.runtime.read().unwrap_or_else(|p| p.into_inner()).as_ref() {
            *state.pending_email.write().unwrap_or_else(|p| p.into_inner()) = Some(rt.email().to_owned());
        }
        state.set_runtime(None);
    }
    Ok(info)
}

/// Riavvio dopo un aggiornamento (12 novembre). Il riavvio di Tauri apre una nuova istanza (`open -n`)
/// mentre questa è ancora viva, e il Dock la mostra come una seconda icona accanto a quella fissata.
/// Qui un piccolo processo aspetta che diMail sia uscita e poi la riapre normalmente: il Dock la
/// riconosce come la stessa app.
#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    let bundle = std::env::current_exe().ok().and_then(|exe| {
        let macos = exe.parent()?;
        let bundle = macos.parent()?.parent()?;
        (macos.file_name()? == "MacOS" && bundle.extension()? == "app").then(|| bundle.to_path_buf())
    });
    let Some(bundle) = bundle else {
        // Fuori da un'app (sviluppo): il riavvio di Tauri.
        app.restart();
    };
    // Al massimo 30 secondi di attesa, poi riapre comunque.
    let script = r#"i=0; while kill -0 "$1" 2>/dev/null && [ "$i" -lt 300 ]; do sleep 0.1; i=$((i+1)); done; exec /usr/bin/open "$2""#;
    let spawned = std::process::Command::new("/bin/sh")
        .args(["-c", script, "sh", &std::process::id().to_string()])
        .arg(&bundle)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        Ok(_) => {
            tracing::info!("riavvio: diMail si chiude e si riapre");
            app.exit(0);
        }
        Err(e) => {
            tracing::warn!(error = %e, "riavvio: non riesco ad aspettare la chiusura, uso quello di Tauri");
            app.restart();
        }
    }
}

/// Il link di configurazione del client in uso, da mandare ai colleghi (Impostazioni → Account Google).
#[tauri::command]
fn client_setup_link(state: State<'_, AppState>) -> CmdResult<String> {
    Ok(state.client()?.setup_link())
}

#[derive(Serialize)]
struct SetupLinkInfo {
    client_id: String,
    project_id: Option<String>,
    /// È lo stesso client già in uso: non c'è niente da fare.
    same: bool,
    /// C'è già un altro client: aprire il link lo sostituisce e richiede un nuovo accesso.
    replaces: bool,
}

/// Cosa c'è in un link di configurazione, per chiedere conferma prima di usarlo. Il link non va mai nei log.
#[tauri::command]
fn setup_link_info(state: State<'_, AppState>, link: String) -> CmdResult<SetupLinkInfo> {
    let client = OAuthClient::from_setup_link(&link)?;
    let current = state.client.read().unwrap_or_else(|p| p.into_inner()).as_ref().map(|c| c.client_id.clone());
    Ok(SetupLinkInfo {
        same: current.as_deref() == Some(client.client_id.as_str()),
        replaces: current.is_some_and(|id| id != client.client_id),
        client_id: client.client_id,
        project_id: client.project_id,
    })
}

#[tauri::command]
async fn apply_setup_link(state: State<'_, AppState>, link: String) -> CmdResult<GoogleClientInfo> {
    let client = OAuthClient::from_setup_link(&link)?;
    apply_client(&state, client).await
}

#[tauri::command]
async fn list_threads(state: State<'_, AppState>, view: View, limit: i64, offset: i64) -> CmdResult<Vec<ThreadSummary>> {
    state.rt()?.list_threads(view, limit, offset).await
}

#[tauri::command]
async fn move_split(state: State<'_, AppState>, thread_ids: Vec<String>, split: String, remember: Remember) -> CmdResult<()> {
    state.rt()?.move_split(thread_ids, split, remember).await
}

#[tauri::command]
async fn split_rules(state: State<'_, AppState>) -> CmdResult<Vec<SplitRule>> {
    state.rt()?.split_rules().await
}

#[tauri::command]
async fn remove_split_rule(state: State<'_, AppState>, kind: String, pattern: String) -> CmdResult<()> {
    state.rt()?.remove_split_rule(kind, pattern).await
}

#[tauri::command]
async fn peek_thread(state: State<'_, AppState>, id: String) -> CmdResult<Option<ThreadDetail>> {
    state.rt()?.peek_thread(id).await
}

#[tauri::command]
async fn count_threads(state: State<'_, AppState>, view: View) -> CmdResult<i64> {
    state.rt()?.count_threads(view).await
}

#[tauri::command]
async fn contact_card(state: State<'_, AppState>, email: String) -> CmdResult<ContactCard> {
    state.rt()?.contact_card(email).await
}

#[tauri::command]
async fn done_today(state: State<'_, AppState>, since: i64) -> CmdResult<i64> {
    state.rt()?.done_today(since).await
}

#[tauri::command]
async fn get_setting(state: State<'_, AppState>, key: String) -> CmdResult<Option<serde_json::Value>> {
    state.rt()?.get_setting(key).await
}

#[tauri::command]
async fn set_setting(state: State<'_, AppState>, key: String, value: serde_json::Value) -> CmdResult<()> {
    state.rt()?.set_setting(key, value).await
}

#[tauri::command]
async fn bulk_preview(state: State<'_, AppState>, cutoff: i64) -> CmdResult<BulkPreview> {
    state.rt()?.bulk_preview(cutoff).await
}

#[tauri::command]
async fn bulk_done(state: State<'_, AppState>, cutoff: i64) -> CmdResult<i64> {
    state.rt()?.bulk_done(cutoff).await
}

#[tauri::command]
async fn bulk_undo(state: State<'_, AppState>, op_id: i64) -> CmdResult<i64> {
    state.rt()?.bulk_undo(op_id).await
}

#[tauri::command]
async fn get_thread(state: State<'_, AppState>, id: String) -> CmdResult<Option<ThreadDetail>> {
    state.rt()?.get_thread(id).await
}

#[tauri::command]
async fn list_labels(state: State<'_, AppState>) -> CmdResult<Vec<LabelRow>> {
    state.rt()?.labels().await
}

#[tauri::command]
async fn sync_status(state: State<'_, AppState>) -> CmdResult<SyncStatus> {
    state.rt()?.status().await
}

#[tauri::command]
async fn act(state: State<'_, AppState>, thread_ids: Vec<String>, action: Action) -> CmdResult<Vec<i64>> {
    state.rt()?.act(thread_ids, action).await
}

#[tauri::command]
async fn undo(state: State<'_, AppState>, op_ids: Vec<i64>) -> CmdResult<Vec<i64>> {
    state.rt()?.undo(op_ids).await
}

#[tauri::command]
async fn sync_now(state: State<'_, AppState>) -> CmdResult<()> {
    state.rt()?.sync_now();
    Ok(())
}

// ───────────── Posticipa ─────────────

#[tauri::command]
async fn snooze(state: State<'_, AppState>, thread_ids: Vec<String>, due_at: i64) -> CmdResult<SnoozeHandle> {
    state.rt()?.snooze(thread_ids, due_at).await
}

#[tauri::command]
async fn unsnooze(state: State<'_, AppState>, handle: SnoozeHandle) -> CmdResult<()> {
    state.rt()?.unsnooze(handle).await
}

#[tauri::command]
async fn list_snoozed(state: State<'_, AppState>) -> CmdResult<Vec<Snoozed>> {
    state.rt()?.list_snoozed().await
}

#[tauri::command]
async fn clear_reminder(state: State<'_, AppState>, thread_id: String) -> CmdResult<()> {
    state.rt()?.clear_reminder(thread_id).await
}

// ───────────── Scrivi ─────────────

#[tauri::command]
async fn reply_draft(state: State<'_, AppState>, thread_id: String, mode: ReplyMode) -> CmdResult<Draft> {
    state.rt()?.reply_draft(thread_id, mode).await
}

#[tauri::command]
async fn send_draft(state: State<'_, AppState>, draft: Draft, delay_ms: i64) -> CmdResult<Queued> {
    state.rt()?.send(draft, delay_ms).await
}

#[tauri::command]
async fn cancel_send(state: State<'_, AppState>, id: i64) -> CmdResult<Option<Draft>> {
    state.rt()?.cancel_send(id).await
}

/// "Invia ora": senza aspettare la fine dell'annulla.
#[tauri::command]
async fn send_now(state: State<'_, AppState>, id: i64) -> CmdResult<bool> {
    state.rt()?.send_now(id).await
}

#[tauri::command]
async fn suggest_contacts(state: State<'_, AppState>, q: String) -> CmdResult<Vec<Addr>> {
    state.rt()?.suggest_contacts(q).await
}

// ───────────── AI ─────────────

#[tauri::command]
async fn ai_status(state: State<'_, AppState>) -> CmdResult<AiStatus> {
    state.rt()?.ai_status().await
}

/// La chiave arriva dal campo password dell'interfaccia e va dritta nel Portachiavi. Mai nei log.
#[tauri::command]
async fn ai_set_key(state: State<'_, AppState>, provider: Provider, key: String) -> CmdResult<()> {
    state.rt()?.ai_set_key(provider, key).await
}

#[tauri::command]
async fn ai_clear_key(state: State<'_, AppState>, provider: Provider) -> CmdResult<()> {
    state.rt()?.ai_clear_key(provider)
}

#[tauri::command]
fn ai_models() -> Vec<ModelInfo> {
    dimail_core::ai::MODELS.to_vec()
}

#[tauri::command]
async fn ai_configure(state: State<'_, AppState>, provider: Provider, model: String) -> CmdResult<()> {
    state.rt()?.ai_configure(AiConfig { provider, model: model.trim().to_owned() }).await
}

/// «Prova»: una richiesta minima al modello scelto. Restituisce i millisecondi.
#[tauri::command]
async fn ai_check(state: State<'_, AppState>) -> CmdResult<u64> {
    state.rt()?.ai_check().await
}

// ───────────── I miei domini ─────────────

#[tauri::command]
async fn my_domains(state: State<'_, AppState>) -> CmdResult<MyDomains> {
    state.rt()?.my_domains().await
}

#[tauri::command]
async fn set_my_domains(state: State<'_, AppState>, domains: Vec<String>) -> CmdResult<MyDomains> {
    state.rt()?.set_my_domains(domains).await
}

/// ⌘J: scrive la risposta o sistema la bozza. Il testo non va nei log.
#[tauri::command]
async fn ai_write(state: State<'_, AppState>, req: WriteRequest) -> CmdResult<String> {
    state.rt()?.ai_write(req).await
}

// ───────────── Allegati ─────────────

/// Scarica l'allegato (la prima volta) e lo apre con l'app predefinita del Mac.
#[tauri::command]
async fn open_attachment(app: AppHandle, state: State<'_, AppState>, message_id: String, part_id: String) -> CmdResult<()> {
    let file = state.rt()?.attachment(message_id, part_id).await?;
    app.opener()
        .open_path(file.path.to_string_lossy(), None::<&str>)
        .map_err(|e| Error::other(format!("non riesco ad aprire l'allegato: {e}")))
}

/// La firma di Gmail web, per mostrarla nel composer (nessuna se su Gmail non c'è).
#[tauri::command]
async fn get_signature(state: State<'_, AppState>) -> CmdResult<Option<Signature>> {
    state.rt()?.signature().await
}

fn local_files(paths: Vec<std::path::PathBuf>) -> CmdResult<Vec<OutAttachment>> {
    paths.iter().map(|p| dimail_core::attachments::local_file(p)).collect()
}

/// File da allegare, scelti nel pannello di macOS (attaccato alla finestra).
#[tauri::command]
async fn pick_attachments(window: tauri::WebviewWindow) -> CmdResult<Vec<OutAttachment>> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    window.dialog().file().set_parent(&window).set_title("Allega file").pick_files(move |files| {
        let _ = tx.send(files);
    });
    let files = rx.await.ok().flatten().unwrap_or_default();
    local_files(files.into_iter().filter_map(|f| f.into_path().ok()).collect())
}

/// File trascinati sulla finestra: nome, dimensione e tipo, per allegarli.
#[tauri::command]
fn file_info(paths: Vec<String>) -> CmdResult<Vec<OutAttachment>> {
    local_files(paths.into_iter().map(std::path::PathBuf::from).collect())
}

/// Copia l'allegato in Download e restituisce il percorso.
#[tauri::command]
async fn save_attachment(state: State<'_, AppState>, message_id: String, part_id: String) -> CmdResult<String> {
    Ok(state.rt()?.save_attachment(message_id, part_id).await?.to_string_lossy().into_owned())
}

/// Un'immagine incollata nel corpo dell'email (`cid:`), come indirizzo `data:` da mostrare al suo posto.
#[tauri::command]
async fn inline_image(state: State<'_, AppState>, message_id: String, part_id: String) -> CmdResult<String> {
    state.rt()?.inline_image(message_id, part_id).await
}

/// Mostra nel Finder un file salvato in Download (solo lì).
#[tauri::command]
fn reveal_download(app: AppHandle, path: String) -> CmdResult<()> {
    let downloads = dirs::download_dir().ok_or_else(|| Error::other("cartella Download non trovata"))?;
    let p = std::path::PathBuf::from(&path);
    if p.parent() != Some(downloads.as_path()) {
        return Err(Error::other("percorso non valido"));
    }
    app.opener().reveal_item_in_dir(p).map_err(|e| Error::other(format!("Finder: {e}")))
}

// ───────────── Sezioni ─────────────

#[tauri::command]
async fn list_sections(state: State<'_, AppState>) -> CmdResult<Vec<Section>> {
    state.rt()?.sections().await
}

#[tauri::command]
async fn create_section(state: State<'_, AppState>, name: String, show_as_tab: bool, instructions: Option<String>) -> CmdResult<Section> {
    state.rt()?.create_section(name, show_as_tab, instructions).await
}

#[tauri::command]
async fn rename_section(state: State<'_, AppState>, label_id: String, name: String) -> CmdResult<()> {
    state.rt()?.rename_section(label_id, name).await
}

#[tauri::command]
async fn configure_section(state: State<'_, AppState>, label_id: String, show_as_tab: bool, instructions: Option<String>) -> CmdResult<()> {
    state.rt()?.configure_section(label_id, show_as_tab, instructions).await
}

#[tauri::command]
async fn archive_section(state: State<'_, AppState>, label_id: String, archived: bool) -> CmdResult<()> {
    state.rt()?.archive_section(label_id, archived).await
}

#[tauri::command]
async fn reorder_sections(state: State<'_, AppState>, ordered: Vec<String>) -> CmdResult<()> {
    state.rt()?.reorder_sections(ordered).await
}

static LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

/// Log locali, a rotazione giornaliera, 7 file. Mai contenuto delle email né token (sez. 14).
fn init_logging() {
    use tracing_subscriber::fmt::writer::MakeWriterExt;
    let dir = paths::logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("dimail")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&dir)
        .expect("cartella dei log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let _ = LOG_GUARD.set(guard);
    let filter = tracing_subscriber::EnvFilter::try_from_env("DIMAIL_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,html5ever=error,dimail_core=debug,dimail_lib=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(writer.and(std::io::stderr))
        .init();
}


/// I link cliccati in un'email si aprono nel browser (o nell'app di posta, per mailto) e la finestra
/// resta dov'è (11 novembre). Le email stanno in iframe isolati senza script: lì WebKit non chiama i
/// gestori di clic messi dall'interfaccia, quindi il clic diventa una navigazione del riquadro, che
/// qui si ferma prima che parta.
fn external_links<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("external-links")
        .on_navigation(|webview, url| {
            let internal = matches!(url.scheme(), "tauri" | "about" | "data" | "blob" | "asset" | "ipc")
                || matches!(url.host_str(), Some("localhost" | "tauri.localhost" | "ipc.localhost" | "asset.localhost"));
            if internal {
                return true;
            }
            if matches!(url.scheme(), "http" | "https" | "mailto" | "tel") {
                if let Err(e) = webview.opener().open_url(url.as_str(), None::<&str>) {
                    tracing::warn!(error = %e, "link esterno: apertura non riuscita");
                }
            }
            false
        })
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Prima del log e della finestra: cartelle e dati della WebView del vecchio identificativo.
    let moved = dimail_core::migrate::files();
    init_logging();
    tracing::info!(versione = env!("CARGO_PKG_VERSION"), "diMail avviato");
    for note in moved {
        tracing::info!("{note}");
    }

    let (events, _) = broadcast::channel::<CoreEvent>(1024);
    let state = AppState {
        http: dimail_core::gmail::http_client(),
        client: RwLock::new(None),
        events: events.clone(),
        runtime: RwLock::new(None),
        pending_email: RwLock::new(None),
        login_lock: Mutex::new(()),
        starting: AtomicBool::new(false),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(external_links())
        .manage(state)
        .setup(move |app| {
            let handle = app.handle().clone();

            // Portachiavi (migrazione, client, account) fuori dal thread principale: la lettura può
            // aspettare minuti il clic dell'utente, e bloccare il setup lasciava la finestra bianca.
            handle.state::<AppState>().starting.store(true, Ordering::SeqCst);
            let h = handle.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let state = h.state::<AppState>();
                dimail_core::migrate::keychain();
                let client = match keychain::load_client() {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!(error = %e, "client Google non letto");
                        None
                    }
                };
                *state.client.write().unwrap_or_else(|p| p.into_inner()) = client.clone();
                if let Some(email) = Registry::load().accounts.first().cloned() {
                    match client {
                        None => {
                            tracing::info!("account in attesa del client Google");
                            *state.pending_email.write().unwrap_or_else(|p| p.into_inner()) = Some(email);
                        }
                        Some(client) => match account::resume(&state.http, &client, &email, state.events.clone(), SyncConfig::default()) {
                            Ok(Some(rt)) => {
                                tracing::info!("account ripreso");
                                state.set_runtime(Some(rt));
                            }
                            Ok(None) => *state.pending_email.write().unwrap_or_else(|p| p.into_inner()) = Some(email),
                            Err(e) => {
                                tracing::error!(error = %e, "account non ripreso");
                                *state.pending_email.write().unwrap_or_else(|p| p.into_inner()) = Some(email);
                            }
                        },
                    }
                }
                state.starting.store(false, Ordering::SeqCst);
                let _ = h.emit("app_ready", ());
            });

            // Eventi del nucleo → interfaccia.
            let mut rx = events.subscribe();
            let emitter = handle.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(e) => {
                            let _ = emitter.emit("core", &e);
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::debug!(n, "eventi persi: l'interfaccia ricarica");
                            let _ = emitter.emit("core", &CoreEvent::ThreadsChanged { ids: vec![] });
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
            Ok(())
        })
        .on_page_load(|webview, payload| {
            tracing::info!(evento = ?payload.event(), url = %payload.url(), finestra = webview.label(), "pagina dell'interfaccia");
        })
        .on_window_event(|window, event| match event {
            WindowEvent::Focused(focused) => {
                let state = window.state::<AppState>();
                if let Some(rt) = state.runtime.read().unwrap_or_else(|p| p.into_inner()).as_ref() {
                    // In primo piano: sync ogni 15 s e subito; in background ogni 60 s.
                    rt.set_foreground(*focused);
                }
            }
            // Il pulsante rosso (e Cmd+W) nasconde la finestra, come Mail: l'app resta aperta, la sync e i
            // lavori a tempo continuano. Si riapre dal Dock; Cmd+Q chiude davvero.
            WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                api.prevent_close();
                let _ = window.hide();
                tracing::debug!("finestra nascosta");
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            log_frontend,
            login,
            google_client,
            load_google_client,
            client_setup_link,
            setup_link_info,
            apply_setup_link,
            restart_app,
            list_threads,
            count_threads,
            peek_thread,
            move_split,
            split_rules,
            remove_split_rule,
            contact_card,
            done_today,
            get_setting,
            set_setting,
            bulk_preview,
            bulk_done,
            bulk_undo,
            get_thread,
            list_labels,
            sync_status,
            act,
            undo,
            sync_now,
            snooze,
            unsnooze,
            list_snoozed,
            clear_reminder,
            reply_draft,
            send_draft,
            cancel_send,
            suggest_contacts,
            ai_status,
            ai_set_key,
            ai_clear_key,
            ai_models,
            ai_configure,
            ai_check,
            ai_write,
            my_domains,
            set_my_domains,
            list_sections,
            create_section,
            rename_section,
            configure_section,
            archive_section,
            reorder_sections,
            open_attachment,
            save_attachment,
            inline_image,
            reveal_download,
            get_signature,
            pick_attachments,
            file_info,
            send_now
        ])
        .build(tauri::generate_context!())
        .expect("errore all'avvio di diMail")
        .run(|app, event| {
            // Clic sull'icona nel Dock con la finestra nascosta: torna.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { has_visible_windows: false, .. } = event
                && let Some(w) = app.get_webview_window("main")
            {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}
