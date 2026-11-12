//! Un account in esecuzione: database, client Gmail, task di sync e coda in uscita.
//! Più account sono possibili (un database ciascuno); la v1 ne usa uno.

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::ai::{AiConfig, AiStatus, Llm, Provider, WriteRequest};
use crate::auth::{self, OAuthClient, TokenManager, keychain};
use crate::mime::Addr;
use crate::send::{Draft, ReplyMode};
use crate::db::Db;
use crate::events::CoreEvent;
use crate::gmail::{GmailApi, GmailClient};
use crate::outbox::{self, Action};
use crate::store::{self, AccountRow, LabelRow, SyncStatus, ThreadDetail, ThreadSummary, View, prio};
use crate::sync::{self, Ctx, SyncConfig};
use crate::{Error, Result, paths};

/// Elenco degli account collegati, in `Application Support/diMail/app.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Registry {
    pub accounts: Vec<String>,
    /// Il Portachiavi del vecchio identificativo è già stato letto (vedi `migrate`).
    #[serde(default)]
    pub legacy_keychain_done: bool,
}

impl Registry {
    fn path() -> std::path::PathBuf {
        paths::data_dir().join("app.json")
    }

    pub fn load() -> Registry {
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::other(format!("cartella dati: {e}")))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?).map_err(|e| Error::other(e.to_string()))?;
        std::fs::rename(&tmp, &path).map_err(|e| Error::other(e.to_string()))?;
        Ok(())
    }

    pub fn add(&mut self, email: &str) {
        let email = email.to_lowercase();
        if !self.accounts.contains(&email) {
            self.accounts.push(email);
        }
    }
}

pub struct AccountRuntime<G: GmailApi = GmailClient> {
    ctx: Arc<Ctx<G>>,
    llm: Arc<dyn Llm>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

/// Risultato di un posticipo: per l'annulla servono le operazioni e i lavori.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnoozeHandle {
    pub ops: Vec<i64>,
    pub jobs: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Queued {
    pub id: i64,
    pub send_at: i64,
}

impl<G: GmailApi> AccountRuntime<G> {
    /// Avvia i task: prima sync, sync incrementale, due corsie di download, coda in uscita,
    /// rilevatore di risveglio.
    pub fn start(ctx: Arc<Ctx<G>>) -> Arc<Self> {
        Self::start_with_ai(ctx, Arc::new(crate::ai::NoLlm))
    }

    pub fn start_with_ai(ctx: Arc<Ctx<G>>, llm: Arc<dyn Llm>) -> Arc<Self> {
        let tasks = vec![
            tokio::spawn(sync::run_bootstrap(Arc::clone(&ctx))),
            tokio::spawn(sync::run_incremental_loop(Arc::clone(&ctx))),
            tokio::spawn(sync::run_fetcher(Arc::clone(&ctx), sync::URGENT)),
            tokio::spawn(sync::run_fetcher(Arc::clone(&ctx), sync::BACKGROUND)),
            tokio::spawn(outbox::run(Arc::clone(&ctx))),
            tokio::spawn(crate::bulk::run(Arc::clone(&ctx))),
            tokio::spawn(classify_missing(Arc::clone(&ctx))),
            tokio::spawn(sync::run_wake_detector(Arc::clone(&ctx))),
            tokio::spawn(sync::repair_once(Arc::clone(&ctx))),
            tokio::spawn(crate::jobs::run(Arc::clone(&ctx))),
            tokio::spawn(crate::send::run(Arc::clone(&ctx))),
            tokio::spawn(crate::ai::run_sections(Arc::clone(&ctx), Arc::clone(&llm))),
        ];
        Arc::new(AccountRuntime { ctx, llm, tasks: Mutex::new(tasks) })
    }

    pub fn email(&self) -> &str {
        &self.ctx.me
    }

    pub fn ctx(&self) -> &Arc<Ctx<G>> {
        &self.ctx
    }

    pub fn shutdown(&self) {
        for t in self.tasks.lock().unwrap_or_else(|p| p.into_inner()).drain(..) {
            t.abort();
        }
    }

    pub async fn account(&self) -> Result<Option<AccountRow>> {
        self.ctx.db.read_async(|c| store::account(c)).await
    }

    pub async fn list_threads(&self, view: View, limit: i64, offset: i64) -> Result<Vec<ThreadSummary>> {
        self.ctx.db.read_async(move |c| store::list_threads(c, &view, limit, offset)).await
    }

    /// Lettura locale senza chiedere i corpi mancanti: per precaricare i thread vicini (sez. 6)
    /// senza consumare quota.
    pub async fn peek_thread(&self, id: String) -> Result<Option<ThreadDetail>> {
        self.ctx.db.read_async(move |c| store::get_thread(c, &id)).await
    }

    /// Correzione manuale di Importanti / Altro, ricordata se richiesto per mittente o dominio.
    pub async fn move_split(&self, thread_ids: Vec<String>, split: String, remember: crate::split::Remember) -> Result<()> {
        let ids = self.ctx.db.write_async(move |c| crate::split::correct(c, &thread_ids, &split, remember)).await?;
        // Una regola per mittente o dominio può spostare molti thread: si ricarica tutto.
        let all = remember != crate::split::Remember::Thread;
        self.ctx.emit(CoreEvent::ThreadsChanged { ids: if all { vec![] } else { ids } });
        Ok(())
    }

    pub async fn split_rules(&self) -> Result<Vec<crate::split::SplitRule>> {
        self.ctx.db.read_async(|c| crate::split::rules(c)).await
    }

    pub async fn remove_split_rule(&self, kind: String, pattern: String) -> Result<()> {
        self.ctx.db.write_async(move |c| crate::split::remove_rule(c, &kind, &pattern)).await?;
        self.ctx.emit(CoreEvent::ThreadsChanged { ids: vec![] });
        Ok(())
    }

    pub async fn count_threads(&self, view: View) -> Result<i64> {
        self.ctx.db.read_async(move |c| store::count_threads(c, &view)).await
    }

    pub async fn contact_card(&self, email: String) -> Result<crate::people::ContactCard> {
        self.ctx.db.read_async(move |c| crate::people::contact_card(c, &email)).await
    }

    /// Thread segnati come fatti da `since` (mezzanotte locale, decisa dall'interfaccia).
    pub async fn done_today(&self, since: i64) -> Result<i64> {
        self.ctx.db.read_async(move |c| store::done_today(c, since)).await
    }

    pub async fn get_setting(&self, key: String) -> Result<Option<serde_json::Value>> {
        self.ctx.db.read_async(move |c| store::get_setting(c, &key)).await
    }

    pub async fn set_setting(&self, key: String, value: serde_json::Value) -> Result<()> {
        self.ctx.db.write_async(move |c| store::set_setting(c, &key, &value)).await
    }

    /// Quanti thread toccherebbe uno sgombero con questa data.
    pub async fn bulk_preview(&self, cutoff: i64) -> Result<crate::bulk::BulkPreview> {
        crate::bulk::preview(&self.ctx, cutoff).await
    }

    /// Sgombero: in locale subito, su Gmail in background. Restituisce l'ID per l'annulla.
    pub async fn bulk_done(&self, cutoff: i64) -> Result<i64> {
        let window = crate::bulk::window_start(self.ctx.cfg.metadata_months);
        let (op, n) = self.ctx.db.write_async(move |c| crate::bulk::start_done(c, cutoff, window)).await?;
        tracing::info!(op, thread = n, "sgombero avviato");
        self.ctx.emit(CoreEvent::ThreadsChanged { ids: vec![] });
        self.ctx.bulk_notify.notify_one();
        self.ctx.emit_status().await;
        Ok(op)
    }

    pub async fn bulk_undo(&self, op_id: i64) -> Result<i64> {
        let undo = self.ctx.db.write_async(move |c| crate::bulk::start_undo(c, op_id)).await?;
        tracing::info!(op = op_id, annullo = undo, "sgombero annullato");
        self.ctx.emit(CoreEvent::ThreadsChanged { ids: vec![] });
        self.ctx.bulk_notify.notify_one();
        Ok(undo)
    }

    /// Lettura locale. Se mancano dei corpi (thread oltre la finestra), li chiede con priorità
    /// massima: l'interfaccia mostra subito i metadati e si aggiorna all'arrivo.
    pub async fn get_thread(&self, id: String) -> Result<Option<ThreadDetail>> {
        let id2 = id.clone();
        let detail = self.ctx.db.read_async(move |c| store::get_thread(c, &id2)).await?;
        if let Some(d) = &detail
            && !d.thread.bodies_complete
        {
            // Con un messaggio solo basta `messages.get`, a metà costo.
            self.ctx
                .db
                .write_async(move |c| {
                    let message_id = store::only_message(c, &id)?;
                    store::enqueue(c, &[store::ToFetch { thread_id: id, listed_history_id: None, message_id }], prio::USER, true)
                })
                .await?;
            self.ctx.fetch_urgent.notify_one();
        }
        Ok(detail)
    }

    pub async fn labels(&self) -> Result<Vec<LabelRow>> {
        self.ctx.db.read_async(|c| store::list_labels(c)).await
    }

    pub async fn status(&self) -> Result<SyncStatus> {
        self.ctx.status().await
    }

    /// Azione ottimistica: stato locale e coda in uscita nella stessa transazione.
    pub async fn act(&self, thread_ids: Vec<String>, action: Action) -> Result<Vec<i64>> {
        let ids = thread_ids.clone();
        let ops = self.ctx.db.write_async(move |c| outbox::enqueue_action(c, &ids, &action)).await?;
        self.ctx.emit(CoreEvent::ThreadsChanged { ids: thread_ids });
        self.ctx.outbox_notify.notify_one();
        Ok(ops)
    }

    pub async fn undo(&self, op_ids: Vec<i64>) -> Result<Vec<i64>> {
        let ops2 = op_ids.clone();
        let threads: Vec<String> = self
            .ctx
            .db
            .read_async(move |c| {
                let mut stmt = c.prepare("SELECT DISTINCT thread_id FROM outbox_ops WHERE id = ?1")?;
                let mut out = Vec::new();
                for id in &ops2 {
                    out.extend(stmt.query_map([id], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?);
                }
                Ok(out)
            })
            .await?;
        let ops = self.ctx.db.write_async(move |c| outbox::enqueue_undo(c, &op_ids)).await?;
        self.ctx.emit(CoreEvent::ThreadsChanged { ids: threads });
        self.ctx.outbox_notify.notify_one();
        Ok(ops)
    }

    // ───────────── Posticipa ─────────────

    /// Posticipa fino a `due_at` (ms). Esce dall'inbox subito, torna in cima all'ora scelta.
    pub async fn snooze(&self, thread_ids: Vec<String>, due_at: i64) -> Result<SnoozeHandle> {
        let label = crate::jobs::ensure_snooze_label(&self.ctx).await;
        let ids = thread_ids.clone();
        let (ops, jobs) = self.ctx.db.write_async(move |c| crate::jobs::snooze(c, &ids, due_at, label.as_deref())).await?;
        self.ctx.emit(CoreEvent::ThreadsChanged { ids: thread_ids });
        self.ctx.outbox_notify.notify_one();
        self.ctx.jobs_notify.notify_one();
        Ok(SnoozeHandle { ops, jobs })
    }

    pub async fn unsnooze(&self, handle: SnoozeHandle) -> Result<()> {
        let jobs = handle.jobs.clone();
        self.ctx.db.write_async(move |c| crate::jobs::cancel(c, &jobs)).await?;
        self.undo(handle.ops).await?;
        Ok(())
    }

    pub async fn list_snoozed(&self) -> Result<Vec<crate::jobs::Snoozed>> {
        self.ctx.db.read_async(|c| crate::jobs::list_snoozed(c)).await
    }

    /// Il thread è stato visto: il segno "Promemoria" sparisce e torna al suo posto.
    pub async fn clear_reminder(&self, thread_id: String) -> Result<()> {
        let id = thread_id.clone();
        let n = self
            .ctx
            .db
            .write_async(move |c| Ok(c.execute("UPDATE threads SET reminded_at = NULL WHERE id = ?1 AND reminded_at IS NOT NULL", [&id])?))
            .await?;
        if n > 0 {
            self.ctx.emit(CoreEvent::ThreadsChanged { ids: vec![thread_id] });
        }
        Ok(())
    }

    // ───────────── Scrivi e rispondi ─────────────

    pub async fn reply_draft(&self, thread_id: String, mode: ReplyMode) -> Result<Draft> {
        let me = self.ctx.me.clone();
        self.ctx.db.read_async(move |c| crate::send::reply_draft(c, &thread_id, mode, &me)).await
    }

    /// In coda per l'invio, tra `delay_ms` (il tempo dell'annulla). Gli allegati si leggono adesso,
    /// quelli inoltrati si scaricano da Gmail se non sono già in cache.
    pub async fn send(&self, draft: Draft, delay_ms: i64) -> Result<Queued> {
        let files = self.attachment_parts(&draft).await?;
        let me = self.ctx.me.clone();
        let (id, send_at) = self
            .ctx
            .db
            .write_async(move |c| {
                let name = store::account(c)?.and_then(|a| a.name);
                let sig = crate::send::cached_signature(c)?.map(|(s, _)| s).filter(|s| !s.html.trim().is_empty());
                crate::send::enqueue(c, &draft, &Addr { name, email: me }, delay_ms.clamp(0, 60_000), sig.as_ref(), &files)
            })
            .await?;
        self.ctx.send_notify.notify_one();
        Ok(Queued { id, send_at })
    }

    async fn attachment_parts(&self, d: &Draft) -> Result<Vec<crate::send::FilePart>> {
        let mut out = Vec::new();
        let mut total = 0usize;
        for a in &d.attachments {
            let path = match (&a.path, &a.message_id, &a.part_id) {
                (Some(p), _, _) => std::path::PathBuf::from(p),
                (None, Some(m), Some(part)) => self.attachment(m.clone(), part.clone()).await?.path,
                _ => continue,
            };
            let data = tokio::task::spawn_blocking(move || std::fs::read(path))
                .await
                .map_err(|e| Error::other(e.to_string()))?
                .map_err(|e| Error::other(format!("«{}» non si legge: {e}", a.name)))?;
            total += data.len();
            if total > crate::send::MAX_ATTACHMENTS_BYTES {
                return Err(Error::other("gli allegati superano 25 MB, il limite di Gmail: mandali con un link di Drive"));
            }
            let mime_type = if a.mime_type.is_empty() { crate::attachments::guess_mime(&a.name).to_owned() } else { a.mime_type.clone() };
            out.push(crate::send::FilePart { name: a.name.clone(), mime_type, data });
        }
        Ok(out)
    }

    /// La firma di Gmail web, per mostrarla nel composer (riletta da Gmail se è vecchia di un'ora).
    pub async fn signature(&self) -> Result<Option<crate::send::Signature>> {
        crate::send::signature(&self.ctx, false).await
    }

    /// "Invia ora": parte subito invece che alla fine dell'annulla. Falso se è già partito o annullato.
    pub async fn send_now(&self, id: i64) -> Result<bool> {
        let moved = self.ctx.db.write_async(move |c| crate::send::send_now(c, id)).await?;
        if moved {
            self.ctx.send_notify.notify_one();
        }
        Ok(moved)
    }

    /// Annulla l'invio, se non è ancora partito. Restituisce la bozza per riaprirla.
    pub async fn cancel_send(&self, id: i64) -> Result<Option<Draft>> {
        self.ctx.db.write_async(move |c| crate::send::cancel(c, id)).await
    }

    pub async fn suggest_contacts(&self, q: String) -> Result<Vec<Addr>> {
        self.ctx.db.read_async(move |c| crate::send::suggest_contacts(c, &q, 8)).await
    }

    // ───────────── AI ─────────────

    pub async fn ai_status(&self) -> Result<AiStatus> {
        let llm = Arc::clone(&self.llm);
        self.ctx.db.read_async(move |c| crate::ai::status(c, llm.as_ref())).await
    }

    pub async fn ai_set_key(&self, provider: Provider, key: String) -> Result<()> {
        self.llm.set_key(provider, key).await?;
        self.ctx.sections_notify.notify_one();
        Ok(())
    }

    pub fn ai_clear_key(&self, provider: Provider) -> Result<()> {
        self.llm.clear_key(provider)
    }

    /// Provider e modello: salvati nell'account e usati da subito.
    pub async fn ai_configure(&self, cfg: AiConfig) -> Result<()> {
        let saved = cfg.clone();
        self.ctx.db.write_async(move |c| crate::ai::save_config(c, &saved)).await?;
        tracing::info!(provider = cfg.provider.name(), modello = %cfg.model, "AI: modello scelto");
        self.llm.configure(cfg);
        self.ctx.sections_notify.notify_one();
        Ok(())
    }

    /// «Prova»: una richiesta minima con il modello scelto, in millisecondi.
    pub async fn ai_check(&self) -> Result<u64> {
        crate::ai::check(self.llm.as_ref()).await
    }

    // ───────────── I miei domini ─────────────

    pub async fn my_domains(&self) -> Result<crate::store::MyDomains> {
        self.ctx.db.read_async(crate::store::my_domains_view).await
    }

    /// Salva i domini e ridivide l'inbox per i mittenti dei domini che entrano o escono.
    pub async fn set_my_domains(&self, domains: Vec<String>) -> Result<crate::store::MyDomains> {
        let view = self.ctx.db.write_async(move |c| crate::store::set_my_domains(c, &domains)).await?;
        self.ctx.emit(CoreEvent::ThreadsChanged { ids: vec![] });
        Ok(view)
    }

    /// ⌘J nel composer: scrive la risposta o sistema la bozza.
    pub async fn ai_write(&self, req: WriteRequest) -> Result<String> {
        crate::ai::write(&self.ctx, self.llm.as_ref(), req).await
    }

    // ───────────── Allegati ─────────────

    /// L'allegato su disco (scaricato da Gmail la prima volta), per aprirlo.
    pub async fn attachment(&self, message_id: String, part_id: String) -> Result<crate::attachments::AttachmentFile> {
        let dir = paths::attachments_cache_dir(&self.ctx.me);
        crate::attachments::fetch(&self.ctx, &dir, &message_id, &part_id).await
    }

    /// Un'immagine nel corpo dell'email (`cid:`), come indirizzo `data:`: scaricata la prima volta.
    pub async fn inline_image(&self, message_id: String, part_id: String) -> Result<String> {
        let file = self.attachment(message_id, part_id).await?;
        crate::attachments::image_data_url(&file)
    }

    /// Una copia in Download. Restituisce dove.
    pub async fn save_attachment(&self, message_id: String, part_id: String) -> Result<std::path::PathBuf> {
        let file = self.attachment(message_id, part_id).await?;
        let downloads = dirs::download_dir().ok_or_else(|| Error::other("cartella Download non trovata"))?;
        crate::attachments::copy_to(&file, &downloads)
    }

    // ───────────── Sezioni ─────────────

    pub async fn sections(&self) -> Result<Vec<crate::sections::Section>> {
        self.ctx.db.write_async(|c| crate::sections::list(c)).await
    }

    /// Nuova sezione: etichetta su Gmail, poi la configurazione locale. Con istruzioni, l'AI
    /// comincia a valutare i thread in inbox.
    pub async fn create_section(&self, name: String, show_as_tab: bool, instructions: Option<String>) -> Result<crate::sections::Section> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(Error::other("serve un nome"));
        }
        let label = self.ctx.gmail.create_label(&name).await?;
        sync::refresh_labels(&self.ctx).await?;
        let id = label.id.clone();
        let section = self
            .ctx
            .db
            .write_async(move |c| {
                crate::sections::insert(c, &id, show_as_tab, instructions.as_deref())?;
                Ok(crate::sections::list(c)?.into_iter().find(|s| s.label_id == id))
            })
            .await?
            .ok_or_else(|| Error::other("sezione non trovata dopo la creazione"))?;
        tracing::info!(tab = show_as_tab, ai = section.instructions.is_some(), "sezione creata");
        self.ctx.emit(CoreEvent::LabelsChanged);
        self.ctx.sections_notify.notify_one();
        Ok(section)
    }

    pub async fn rename_section(&self, label_id: String, name: String) -> Result<()> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(Error::other("serve un nome"));
        }
        self.ctx.gmail.rename_label(&label_id, &name).await?;
        sync::refresh_labels(&self.ctx).await?;
        self.ctx.emit(CoreEvent::LabelsChanged);
        Ok(())
    }

    pub async fn configure_section(&self, label_id: String, show_as_tab: bool, instructions: Option<String>) -> Result<()> {
        let changed = self
            .ctx
            .db
            .write_async(move |c| crate::sections::configure(c, &label_id, show_as_tab, instructions.as_deref()))
            .await?;
        self.ctx.emit(CoreEvent::LabelsChanged);
        self.ctx.emit(CoreEvent::ThreadsChanged { ids: vec![] });
        if changed {
            self.ctx.sections_notify.notify_one();
        }
        Ok(())
    }

    pub async fn archive_section(&self, label_id: String, archived: bool) -> Result<()> {
        self.ctx.db.write_async(move |c| crate::sections::set_archived(c, &label_id, archived)).await?;
        self.ctx.emit(CoreEvent::LabelsChanged);
        self.ctx.emit(CoreEvent::ThreadsChanged { ids: vec![] });
        Ok(())
    }

    pub async fn reorder_sections(&self, ordered: Vec<String>) -> Result<()> {
        self.ctx.db.write_async(move |c| crate::sections::reorder(c, &ordered)).await?;
        self.ctx.emit(CoreEvent::LabelsChanged);
        Ok(())
    }

    pub fn set_foreground(&self, fg: bool) {
        self.ctx.set_foreground(fg);
    }

    pub fn sync_now(&self) {
        self.ctx.wake_all();
    }
}

impl<G: GmailApi> Drop for AccountRuntime<G> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn build_ctx(
    http: &reqwest::Client,
    client: &OAuthClient,
    email: &str,
    refresh_token: String,
    events: broadcast::Sender<CoreEvent>,
    cfg: SyncConfig,
) -> Result<(Arc<Ctx<GmailClient>>, Arc<TokenManager>)> {
    let db = Db::open(&paths::account_db(email))?;
    let tokens = TokenManager::new(http.clone(), client.clone(), refresh_token);
    let gmail = Arc::new(GmailClient::new(http.clone(), Arc::clone(&tokens)));
    Ok((Ctx::new(db, gmail, email.to_lowercase(), cfg, events), tokens))
}

/// L'AI con le chiavi del Portachiavi e il provider e il modello scelti per l'account.
fn chat_api(http: &reqwest::Client, ctx: &Ctx<GmailClient>) -> Arc<dyn Llm> {
    let api = crate::ai::ChatApi::from_keychain(http.clone());
    match ctx.db.read(crate::ai::load_config) {
        Ok(cfg) => api.configure(cfg),
        Err(e) => tracing::warn!(error = %e, "AI: impostazioni non lette"),
    }
    Arc::new(api)
}

/// Login nel browser, refresh token nel Portachiavi, profilo ed etichette salvati (M0),
/// poi avvio della sync.
/// Classifica Importanti / Altro i thread che ancora non lo sono (dopo la migrazione 004).
async fn classify_missing<G: GmailApi>(ctx: Arc<Ctx<G>>) {
    let started = std::time::Instant::now();
    match ctx.db.write_async(|c| crate::split::recompute_missing(c)).await {
        Ok(0) => {}
        Ok(n) => {
            tracing::info!(thread = n, ms = started.elapsed().as_millis() as u64, "inbox divisa calcolata");
            ctx.emit(CoreEvent::ThreadsChanged { ids: vec![] });
        }
        Err(e) => tracing::error!(error = %e, "inbox divisa non calcolata"),
    }
}

pub async fn connect(
    http: &reqwest::Client,
    client: &OAuthClient,
    login_hint: Option<&str>,
    open_url: impl FnOnce(&str) -> Result<()>,
    events: broadcast::Sender<CoreEvent>,
    cfg: SyncConfig,
) -> Result<Arc<AccountRuntime>> {
    let login = auth::login(http, client, login_hint, open_url).await?;
    let email = login.identity.email.clone();
    tracing::info!(dominio = login.identity.hosted_domain.as_deref().unwrap_or("-"), "login completato");
    keychain::save_refresh_token(&email, &login.refresh_token)?;

    let (ctx, tokens) = build_ctx(http, client, &email, login.refresh_token, events, cfg)?;
    tokens.seed(login.access_token, login.expires_at).await;
    let llm = chat_api(http, &ctx);
    let (name, picture) = (login.identity.name.clone(), login.identity.picture.clone());
    let me = email.clone();
    ctx.db
        .write_async(move |c| store::ensure_account(c, &me, name.as_deref(), picture.as_deref()))
        .await?;
    sync::save_profile_and_labels(&ctx).await?;

    let mut reg = Registry::load();
    reg.add(&email);
    reg.save()?;
    Ok(AccountRuntime::start_with_ai(ctx, llm))
}

/// Riprende un account già collegato. `Ok(None)` se nel Portachiavi non c'è il token.
pub fn resume(
    http: &reqwest::Client,
    client: &OAuthClient,
    email: &str,
    events: broadcast::Sender<CoreEvent>,
    cfg: SyncConfig,
) -> Result<Option<Arc<AccountRuntime>>> {
    let Some(refresh) = keychain::load_refresh_token(email)? else {
        return Ok(None);
    };
    let (ctx, _) = build_ctx(http, client, email, refresh, events, cfg)?;
    let llm = chat_api(http, &ctx);
    Ok(Some(AccountRuntime::start_with_ai(ctx, llm)))
}

/// Toglie il token dal Portachiavi. I dati locali restano.
pub fn disconnect(email: &str) -> Result<()> {
    keychain::delete_refresh_token(email)
}
