//! Motore di sync (sez. 5.2 e 5.3).
//!
//! - **Prima sync:** si salva l'`historyId` *prima* di elencare, poi si mettono in coda gli ID:
//!   prima pagina dell'inbox (metadati), corpi degli ultimi 90 giorni, resto dell'inbox e dei
//!   12 mesi (metadati). I costi di Gmail sono quelli di maggio 2026: vedi `list_window`.
//! - **Coda di download** persistente e a priorità, condivisa da prima sync, sync incrementale e
//!   interfaccia. Due corsie: urgente (utente, incrementale, prima pagina) e storico (al massimo il
//!   70% della quota). I thread con un solo messaggio si scaricano con `messages.get`.
//! - **Sync incrementale:** `history.list` dall'ultimo `historyId`; i cambi di etichette si
//!   applicano in locale, si scaricano solo i messaggi nuovi. Ogni 15 s in primo piano, 60 s in
//!   background, subito al ritorno in primo piano, al risveglio, al ritorno della rete e dopo
//!   un'azione confermata.
//! - **historyId troppo vecchio (404):** sync completa senza svuotare la cache.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use tokio::sync::{Notify, broadcast};

use crate::db::Db;
use crate::events::CoreEvent;
use crate::gmail::{GmailApi, ListThreads, Message, MessageRef, Priority, Thread, ThreadFormat, ThreadRef};
use crate::store::{self, FetchItem, ToFetch, prio};
use crate::util::{backoff_ms, now_ms, parse_i64};
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// Metadati di tutto ciò che non è in inbox (sez. 5.1).
    pub metadata_months: u32,
    /// Corpi di tutto ciò che non è in inbox.
    pub body_days: u32,
    pub foreground_interval: Duration,
    pub background_interval: Duration,
    pub offline_retry: Duration,
    /// Thread per batch di download.
    pub batch_size: usize,
    /// Ogni quanto rileggere le etichette (create o rinominate altrove non compaiono nella history).
    pub labels_refresh: Duration,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            metadata_months: 12,
            body_days: 90,
            foreground_interval: Duration::from_secs(15),
            background_interval: Duration::from_secs(60),
            offline_retry: Duration::from_secs(5),
            batch_size: crate::gmail::FETCH_PER_BATCH,
            labels_refresh: Duration::from_secs(5 * 60),
        }
    }
}

#[derive(Default)]
struct Phase {
    offline: bool,
    auth_required: bool,
    syncing: bool,
    last_labels_refresh: Option<std::time::Instant>,
}

/// Stato condiviso dai task di un account.
pub struct Ctx<G: GmailApi> {
    pub db: Arc<Db>,
    pub gmail: Arc<G>,
    pub me: String,
    pub cfg: SyncConfig,
    pub events: broadcast::Sender<CoreEvent>,
    pub sync_trigger: Notify,
    pub fetch_urgent: Notify,
    pub fetch_background: Notify,
    pub outbox_notify: Notify,
    pub bulk_notify: Notify,
    pub jobs_notify: Notify,
    pub send_notify: Notify,
    pub sections_notify: Notify,
    foreground: AtomicBool,
    phase: Mutex<Phase>,
    claimed: Mutex<HashSet<String>>,
}

impl<G: GmailApi> Ctx<G> {
    pub fn new(db: Arc<Db>, gmail: Arc<G>, me: String, cfg: SyncConfig, events: broadcast::Sender<CoreEvent>) -> Arc<Self> {
        Arc::new(Ctx {
            db,
            gmail,
            me,
            cfg,
            events,
            sync_trigger: Notify::new(),
            fetch_urgent: Notify::new(),
            fetch_background: Notify::new(),
            outbox_notify: Notify::new(),
            bulk_notify: Notify::new(),
            jobs_notify: Notify::new(),
            send_notify: Notify::new(),
            sections_notify: Notify::new(),
            foreground: AtomicBool::new(true),
            phase: Mutex::new(Phase::default()),
            claimed: Mutex::new(HashSet::new()),
        })
    }

    pub fn emit(&self, e: CoreEvent) {
        // Nessun ascoltatore non è un errore.
        let _ = self.events.send(e);
    }

    fn phase(&self) -> std::sync::MutexGuard<'_, Phase> {
        self.phase.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn is_foreground(&self) -> bool {
        self.foreground.load(Ordering::Relaxed)
    }

    pub fn set_foreground(&self, fg: bool) {
        let was = self.foreground.swap(fg, Ordering::Relaxed);
        if fg && !was {
            self.sync_trigger.notify_one();
        }
    }

    pub fn is_offline(&self) -> bool {
        self.phase().offline
    }

    pub fn set_offline(&self, offline: bool) {
        let was = std::mem::replace(&mut self.phase().offline, offline);
        if was && !offline {
            tracing::info!("rete tornata");
            self.wake_all();
        } else if !was && offline {
            tracing::info!("rete assente");
        }
    }

    pub fn is_auth_required(&self) -> bool {
        self.phase().auth_required
    }

    pub fn set_auth_required(&self) {
        let was = std::mem::replace(&mut self.phase().auth_required, true);
        if !was {
            tracing::warn!("refresh token non più valido: serve un nuovo login");
            self.emit(CoreEvent::AuthRequired { email: self.me.clone() });
        }
    }

    /// Risveglia tutti i task (risveglio dal sonno, rete tornata, finestra in primo piano).
    pub fn wake_all(&self) {
        self.sync_trigger.notify_one();
        self.fetch_urgent.notify_one();
        self.fetch_background.notify_one();
        self.outbox_notify.notify_one();
        self.bulk_notify.notify_one();
        self.jobs_notify.notify_one();
        self.send_notify.notify_one();
        self.sections_notify.notify_one();
    }

    pub async fn status(&self) -> Result<store::SyncStatus> {
        let mut s = self.db.read_async(|c| store::sync_counts(c)).await?;
        let p = self.phase();
        s.phase = if p.auth_required {
            "auth_required"
        } else if p.offline {
            "offline"
        } else if s.initial_sync_state != "done" {
            "initial"
        } else if p.syncing || s.queued > 0 {
            "syncing"
        } else if s.last_error.is_some() {
            "error"
        } else {
            "idle"
        }
        .into();
        Ok(s)
    }

    pub async fn emit_status(&self) {
        match self.status().await {
            Ok(s) => self.emit(CoreEvent::SyncStatus(s)),
            Err(e) => tracing::warn!(error = %e, "stato della sync non disponibile"),
        }
    }

    /// Classifica un errore di rete/autenticazione e aggiorna lo stato.
    pub fn note_error(&self, e: &Error) {
        match e {
            Error::Network(_) => self.set_offline(true),
            Error::AuthRevoked => self.set_auth_required(),
            _ => {}
        }
    }
}

// ───────────────────────────── etichette e profilo ─────────────────────────────

pub async fn refresh_labels<G: GmailApi>(ctx: &Ctx<G>) -> Result<()> {
    let labels = ctx.gmail.labels().await?;
    ctx.db.write_async(move |c| store::replace_labels(c, &labels)).await?;
    ctx.phase().last_labels_refresh = Some(std::time::Instant::now());
    ctx.emit(CoreEvent::LabelsChanged);
    Ok(())
}

/// Passi 1 e 2 della prima sync: profilo (con l'`historyId` di partenza) ed etichette.
pub async fn save_profile_and_labels<G: GmailApi>(ctx: &Ctx<G>) -> Result<()> {
    let p = ctx.gmail.profile().await?;
    if !p.email_address.eq_ignore_ascii_case(&ctx.me) {
        return Err(Error::Auth(format!("il profilo Gmail è di {}, non di {}", p.email_address, ctx.me)));
    }
    let history = parse_i64(Some(&p.history_id));
    let (mt, tt) = (p.messages_total, p.threads_total);
    ctx.db
        .write_async(move |c| {
            let acc = store::account(c)?;
            // L'historyId di partenza si fissa una sola volta, prima di elencare i thread.
            let first_time = acc.as_ref().is_none_or(|a| a.history_id.is_none());
            store::set_profile(c, if first_time { history } else { None }, mt, tt)?;
            if first_time {
                store::set_initial_sync_state(c, "listing")?;
            }
            Ok(())
        })
        .await?;
    refresh_labels(ctx).await
}

// ───────────────────────────── elenco nella coda ─────────────────────────────

/// Messaggi della finestra, per sapere quali thread ne hanno uno solo: quelli si scaricano con
/// `messages.get`, a metà costo di `threads.get`.
#[derive(Default)]
struct MessageScan {
    /// Thread → messaggi visti, dal più recente.
    seen: HashMap<String, Vec<String>>,
    messages: usize,
}

impl MessageScan {
    fn add(&mut self, refs: &[MessageRef]) {
        for m in refs {
            if let Some(t) = &m.thread_id {
                let seen = self.seen.entry(t.clone()).or_default();
                if !seen.contains(&m.id) {
                    seen.push(m.id.clone());
                    self.messages += 1;
                }
            }
        }
    }

    /// Elenca i messaggi della richiesta, da `req.page_token` in poi, fino all'ultima pagina.
    async fn scan<G: GmailApi>(&mut self, ctx: &Ctx<G>, mut req: ListThreads) -> Result<()> {
        loop {
            let page = ctx.gmail.list_messages(req.clone(), Priority::Interactive).await?;
            self.add(&page.messages);
            match page.next_page_token {
                Some(t) => req.page_token = Some(t),
                None => return Ok(()),
            }
        }
    }

    /// Il messaggio da scaricare, se il thread ne ha uno solo. In Gmail l'ID di un thread è quello del
    /// suo primo messaggio, e `messages.list` va dal più recente: quando si è visto il primo messaggio,
    /// gli altri (più nuovi) sono già passati. Se il conto è uno, il thread è tutto lì. Nel dubbio
    /// (primo messaggio fuori finestra, ID diversi) si scarica il thread intero.
    fn single(&self, thread_id: &str) -> Option<String> {
        match self.seen.get(thread_id).map(Vec::as_slice) {
            Some([only]) if only == thread_id => Some(only.clone()),
            _ => None,
        }
    }

    fn singles(&self) -> usize {
        self.seen.iter().filter(|(t, m)| m.len() == 1 && m[0] == **t).count()
    }
}

fn to_fetch(refs: &[ThreadRef], scan: &MessageScan) -> Vec<ToFetch> {
    refs.iter()
        .map(|t| ToFetch {
            thread_id: t.id.clone(),
            listed_history_id: parse_i64(t.history_id.as_deref()),
            message_id: scan.single(&t.id),
        })
        .collect()
}

async fn enqueue_refs<G: GmailApi>(ctx: &Ctx<G>, items: Vec<ToFetch>, p: i64, want_body: bool) -> Result<usize> {
    let n = ctx.db.write_async(move |c| store::enqueue(c, &items, p, want_body)).await?;
    if n > 0 {
        if p < prio::BACKGROUND_FROM { ctx.fetch_urgent.notify_one() } else { ctx.fetch_background.notify_one() }
    }
    Ok(n)
}

/// Elenca i thread (paginando) e li mette in coda pagina per pagina. Restituisce gli ID elencati.
/// Gli elenchi costano poco e vanno a priorità piena: se aspettassero la quota dello storico,
/// i download resterebbero senza lavoro.
async fn list_into_queue<G: GmailApi>(ctx: &Ctx<G>, mut req: ListThreads, scan: &MessageScan, p: i64, want_body: bool) -> Result<Vec<String>> {
    let mut all = Vec::new();
    loop {
        let page = ctx.gmail.list_threads(req.clone(), Priority::Interactive).await?;
        all.extend(page.threads.iter().map(|t| t.id.clone()));
        enqueue_refs(ctx, to_fetch(&page.threads, scan), p, want_body).await?;
        match page.next_page_token {
            Some(t) => req.page_token = Some(t),
            None => break,
        }
    }
    Ok(all)
}

/// Mette in coda la finestra di sync (sez. 5.1, con i costi di Gmail di maggio 2026):
/// 1. prima pagina dell'inbox, solo metadati, subito;
/// 2. tutti i messaggi dei 12 mesi (5 unità ogni 500), per riconoscere i thread con un messaggio solo;
/// 3. corpi degli ultimi 90 giorni, inbox compresa;
/// 4. l'inbox dei 12 mesi, solo metadati (il corpo arriva quando si apre il thread);
/// 5. il resto dei 12 mesi, solo metadati;
/// 6. l'inbox più vecchia, per ultima: può essere enorme (quella di Tommaso ha 57.000 thread). Prima
///    si elencano anche i messaggi più vecchi, così i thread con un messaggio solo costano la metà.
async fn list_window<G: GmailApi>(ctx: &Ctx<G>) -> Result<HashSet<String>> {
    let mut listed = HashSet::new();
    let window = format!("newer_than:{}m", ctx.cfg.metadata_months);
    let mut scan = MessageScan::default();

    let mut msgs_req = ListThreads { q: Some(window.clone()), max_results: 500, ..Default::default() };
    let first_msgs = ctx.gmail.list_messages(msgs_req.clone(), Priority::Interactive).await?;
    scan.add(&first_msgs.messages);
    let first = ctx
        .gmail
        .list_threads(ListThreads { label_ids: vec![store::INBOX.into()], max_results: 100, ..Default::default() }, Priority::Interactive)
        .await?;
    listed.extend(first.threads.iter().map(|t| t.id.clone()));
    enqueue_refs(ctx, to_fetch(&first.threads, &scan), prio::INBOX_FIRST, false).await?;

    if let Some(t) = first_msgs.next_page_token {
        msgs_req.page_token = Some(t);
        scan.scan(ctx, msgs_req).await?;
    }
    tracing::info!(messaggi = scan.messages, thread = scan.seen.len(), con_un_messaggio = scan.singles(), "messaggi della finestra elencati");

    let req = |q: Option<String>, label: Option<&str>| ListThreads {
        q,
        label_ids: label.map(|l| vec![l.to_owned()]).unwrap_or_default(),
        max_results: 500,
        ..Default::default()
    };
    let bodies = list_into_queue(ctx, req(Some(format!("newer_than:{}d", ctx.cfg.body_days)), None), &scan, prio::WINDOW_BODIES, true).await?;
    tracing::info!(n = bodies.len(), giorni = ctx.cfg.body_days, "finestra dei corpi elencata");
    listed.extend(bodies);

    let mut inbox_old: Vec<ThreadRef> = Vec::new();
    let mut inbox_req = req(None, Some(store::INBOX));
    loop {
        let page = ctx.gmail.list_threads(inbox_req.clone(), Priority::Interactive).await?;
        listed.extend(page.threads.iter().map(|t| t.id.clone()));
        let (recent, old): (Vec<ThreadRef>, Vec<ThreadRef>) = page.threads.into_iter().partition(|t| scan.seen.contains_key(&t.id));
        enqueue_refs(ctx, to_fetch(&recent, &scan), prio::INBOX_REST, false).await?;
        inbox_old.extend(old);
        match page.next_page_token {
            Some(t) => inbox_req.page_token = Some(t),
            None => break,
        }
    }
    tracing::info!(oltre_la_finestra = inbox_old.len(), "inbox elencata");

    let meta = list_into_queue(ctx, req(Some(window), None), &scan, prio::WINDOW_METADATA, false).await?;
    tracing::info!(n = meta.len(), mesi = ctx.cfg.metadata_months, "finestra dei metadati elencata");
    listed.extend(meta);

    if !inbox_old.is_empty() {
        let older = format!("older_than:{}m", ctx.cfg.metadata_months);
        scan.scan(ctx, ListThreads { q: Some(older), max_results: 500, ..Default::default() }).await?;
        let items = to_fetch(&inbox_old, &scan);
        let singles = items.iter().filter(|i| i.message_id.is_some()).count();
        enqueue_refs(ctx, items, prio::INBOX_OLD, false).await?;
        tracing::info!(n = inbox_old.len(), con_un_messaggio = singles, "inbox oltre la finestra in coda");
    }
    Ok(listed)
}

/// Prima sync. Riprende da dove era arrivata dopo un riavvio.
pub async fn bootstrap<G: GmailApi>(ctx: &Ctx<G>) -> Result<()> {
    let acc = ctx.db.read_async(|c| store::account(c)).await?.ok_or_else(|| Error::other("account mancante"))?;
    if acc.initial_sync_state == "done" || acc.initial_sync_state == "fetching" {
        return Ok(());
    }
    if acc.history_id.is_none() {
        save_profile_and_labels(ctx).await?;
    }
    // Un elenco interrotto si rifà da capo: la coda della prima sync si ricostruisce con le regole
    // correnti. Restano le richieste dell'utente e della sync incrementale.
    ctx.db
        .write_async(|c| Ok(c.execute("DELETE FROM fetch_queue WHERE priority >= ?1", [prio::INBOX_FIRST])?))
        .await?;
    let started = std::time::Instant::now();
    list_window(ctx).await?;
    ctx.db.write_async(|c| store::set_initial_sync_state(c, "fetching")).await?;
    tracing::info!(ms = started.elapsed().as_millis() as u64, "prima sync: elenco completato");
    ctx.emit_status().await;
    Ok(())
}

/// Con l'`historyId` scaduto: si rielenca la finestra, si scarica solo ciò che è cambiato
/// (confronto degli `historyId` per thread) e si eliminano i thread spariti. La cache resta.
async fn full_resync<G: GmailApi>(ctx: &Ctx<G>) -> Result<usize> {
    tracing::warn!("historyId troppo vecchio: sync completa in background");
    let p = ctx.gmail.profile().await?;
    let new_history = parse_i64(Some(&p.history_id)).ok_or_else(|| Error::other("historyId non valido"))?;
    let listed = list_window(ctx).await?;
    let cutoff = now_ms() - i64::from(ctx.cfg.metadata_months) * 31 * 24 * 3600 * 1000;
    let n = listed.len();
    let removed = ctx
        .db
        .write_async(move |c| {
            let tx = c.transaction()?;
            let local: Vec<String> = tx
                .prepare(
                    "SELECT id FROM threads WHERE (last_date >= ?1 OR in_inbox = 1)
                     AND id NOT IN (SELECT thread_id FROM outbox_ops WHERE state IN ('pending', 'in_flight'))",
                )?
                .query_map([cutoff], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            let mut removed = Vec::new();
            for id in local {
                if !listed.contains(&id) && store::delete_thread(&tx, &id)? {
                    removed.push(id);
                }
            }
            store::set_history_id(&tx, new_history)?;
            tx.commit()?;
            Ok(removed)
        })
        .await?;
    if !removed.is_empty() {
        tracing::info!(n = removed.len(), "thread spariti eliminati in locale");
        ctx.emit(CoreEvent::ThreadsChanged { ids: removed });
    }
    Ok(n)
}

/// Un giro di sync incrementale. Restituisce quanti thread sono cambiati o sono in coda,
/// `None` se la prima sync non ha ancora fissato l'`historyId`.
///
/// I cambi di etichette (archiviato, letto, stella...) arrivano nella history con le etichette
/// nuove del messaggio: si applicano in locale senza scaricare nulla. Si scaricano solo i messaggi
/// nuovi, uno per uno.
pub async fn incremental_once<G: GmailApi>(ctx: &Ctx<G>) -> Result<Option<usize>> {
    let Some(start) = ctx.db.read_async(|c| Ok(store::account(c)?.and_then(|a| a.history_id))).await? else {
        return Ok(None);
    };
    let mut records = Vec::new();
    let mut page_token = None;
    let mut latest: Option<i64> = None;
    loop {
        let page = match ctx.gmail.history(start, page_token.take()).await {
            Err(Error::NotFound) => return full_resync(ctx).await.map(Some),
            Err(e) => return Err(e),
            Ok(p) => p,
        };
        records.extend(page.history);
        if let Some(h) = parse_i64(page.history_id.as_deref()) {
            latest = Some(latest.map_or(h, |l: i64| l.max(h)));
        }
        match page.next_page_token {
            Some(t) => page_token = Some(t),
            None => break,
        }
    }

    let (changed, queued) = ctx
        .db
        .write_async(move |c| {
            let tx = c.transaction()?;
            let mut changed: BTreeSet<String> = BTreeSet::new();
            let mut full: Vec<ToFetch> = Vec::new();
            let mut meta: Vec<ToFetch> = Vec::new();
            for rec in &records {
                let h = parse_i64(rec.id.as_deref()).or(latest).unwrap_or(start);
                for added in &rec.messages_added {
                    let m = &added.message;
                    let Some(t) = &m.thread_id else { continue };
                    // Basta il messaggio se il thread è già in locale o se il messaggio lo apre; una
                    // risposta a un thread che non abbiamo porta il thread intero.
                    let single = m.id == *t || store::has_thread(&tx, t)?;
                    let item = ToFetch { thread_id: t.clone(), listed_history_id: None, message_id: single.then(|| m.id.clone()) };
                    // Le bozze si salvano di continuo: bastano i metadati.
                    if m.label_ids.iter().any(|l| l == "DRAFT") { meta.push(item) } else { full.push(item) }
                }
                for deleted in &rec.messages_deleted {
                    if let Some(t) = store::delete_message(&tx, &deleted.message.id)? {
                        changed.insert(t);
                    }
                }
                for m in rec.labels_added.iter().chain(&rec.labels_removed).map(|x| &x.message) {
                    if let Some((t, did_change)) = store::apply_history_labels(&tx, &m.id, &m.label_ids, h)? {
                        if did_change {
                            changed.insert(t);
                        }
                        continue;
                    }
                    // Messaggio che non abbiamo. Un thread fuori finestra conta solo se torna in inbox
                    // (un'operazione di massa su posta vecchia non deve scaricare migliaia di thread).
                    let Some(t) = &m.thread_id else { continue };
                    let local = store::has_thread(&tx, t)?;
                    if local || m.label_ids.iter().any(|l| l == store::INBOX) {
                        meta.push(ToFetch { thread_id: t.clone(), listed_history_id: None, message_id: local.then(|| m.id.clone()) });
                    }
                }
            }
            let queued = store::enqueue(&tx, &full, prio::INCREMENTAL, true)? + store::enqueue(&tx, &meta, prio::INCREMENTAL, false)?;
            // La coda è persistente: si può avanzare l'historyId nella stessa transazione.
            if let Some(h) = latest
                && h > start
            {
                store::set_history_id(&tx, h)?;
            }
            store::set_sync_result(&tx, None)?;
            tx.commit()?;
            Ok((changed.into_iter().collect::<Vec<_>>(), queued))
        })
        .await?;
    if !changed.is_empty() {
        tracing::debug!(n = changed.len(), "sync incrementale: etichette applicate in locale");
        ctx.emit(CoreEvent::ThreadsChanged { ids: changed.clone() });
    }
    if queued > 0 {
        tracing::debug!(n = queued, "sync incrementale: messaggi da scaricare");
        ctx.fetch_urgent.notify_one();
    }
    Ok(Some(changed.len() + queued))
}

pub async fn run_incremental_loop<G: GmailApi>(ctx: Arc<Ctx<G>>) {
    loop {
        if ctx.is_auth_required() {
            ctx.sync_trigger.notified().await;
            continue;
        }
        ctx.phase().syncing = true;
        let res = incremental_once(&ctx).await;
        ctx.phase().syncing = false;
        match res {
            Ok(None) => {}
            Ok(Some(_)) => {
                ctx.set_offline(false);
                let stale = ctx
                    .phase()
                    .last_labels_refresh
                    .is_none_or(|t| t.elapsed() >= ctx.cfg.labels_refresh);
                if stale {
                    if let Err(e) = refresh_labels(&ctx).await {
                        tracing::debug!(error = %e, "etichette non aggiornate");
                    }
                }
            }
            Err(e) => {
                ctx.note_error(&e);
                if !matches!(e, Error::Network(_) | Error::AuthRevoked) {
                    tracing::warn!(error = %e, "sync incrementale fallita");
                    let msg = e.to_string();
                    let _ = ctx.db.write_async(move |c| store::set_sync_result(c, Some(&msg))).await;
                }
            }
        }
        ctx.emit_status().await;

        let interval = if ctx.is_offline() {
            ctx.cfg.offline_retry
        } else if ctx.is_foreground() {
            ctx.cfg.foreground_interval
        } else {
            ctx.cfg.background_interval
        };
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = ctx.sync_trigger.notified() => {}
        }
    }
}

// ───────────────────────────── download dei thread ─────────────────────────────

#[derive(Clone, Copy)]
pub struct Lane {
    pub name: &'static str,
    pub min_prio: i64,
    pub max_prio: i64,
    pub gmail_prio: Priority,
}

pub const URGENT: Lane = Lane { name: "urgente", min_prio: 0, max_prio: prio::BACKGROUND_FROM - 1, gmail_prio: Priority::Interactive };
pub const BACKGROUND: Lane = Lane { name: "storico", min_prio: prio::BACKGROUND_FROM, max_prio: i64::MAX, gmail_prio: Priority::Background };

struct Claim<'a, G: GmailApi> {
    ctx: &'a Ctx<G>,
    ids: Vec<String>,
}

impl<G: GmailApi> Drop for Claim<'_, G> {
    fn drop(&mut self) {
        let mut set = self.ctx.claimed.lock().unwrap_or_else(|p| p.into_inner());
        for id in &self.ids {
            set.remove(id);
        }
    }
}

enum Fetched {
    Thread(Thread),
    Message(Message),
}

/// Scarica un batch dalla corsia. Restituisce quanti thread ha preso dalla coda.
pub async fn fetch_once<G: GmailApi>(ctx: &Ctx<G>, lane: Lane) -> Result<usize> {
    let claimed: HashSet<String> = ctx.claimed.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let (min, max, size) = (lane.min_prio, lane.max_prio, ctx.cfg.batch_size);
    let items = ctx.db.read_async(move |c| store::next_fetch_batch(c, min, max, size, &claimed)).await?;
    if items.is_empty() {
        return Ok(0);
    }
    let _claim = {
        let mut set = ctx.claimed.lock().unwrap_or_else(|p| p.into_inner());
        let ids: Vec<String> = items.iter().filter(|i| set.insert(i.thread_id.clone())).map(|i| i.thread_id.clone()).collect();
        Claim { ctx, ids }
    };

    let n = items.len();
    // Quattro gruppi: thread interi o singoli messaggi, con o senza corpo.
    for (by_message, format) in [(false, ThreadFormat::Full), (true, ThreadFormat::Full), (false, ThreadFormat::Metadata), (true, ThreadFormat::Metadata)] {
        let group: Vec<FetchItem> = items
            .iter()
            .filter(|i| i.message_id.is_some() == by_message && i.want_body == (format == ThreadFormat::Full))
            .cloned()
            .collect();
        if group.is_empty() {
            continue;
        }
        let results: Result<Vec<Result<Fetched>>> = if by_message {
            let ids: Vec<String> = group.iter().filter_map(|i| i.message_id.clone()).collect();
            ctx.gmail
                .get_messages(&ids, format, lane.gmail_prio)
                .await
                .map(|v| v.into_iter().map(|r| r.map(Fetched::Message)).collect())
        } else {
            let ids: Vec<String> = group.iter().map(|i| i.thread_id.clone()).collect();
            ctx.gmail
                .get_threads(&ids, format, lane.gmail_prio)
                .await
                .map(|v| v.into_iter().map(|r| r.map(Fetched::Thread)).collect())
        };
        let results = match results {
            Ok(r) => r,
            Err(e) => {
                ctx.note_error(&e);
                let transient = e.is_transient() || matches!(e, Error::AuthRevoked);
                let limited = e.is_rate_limit();
                let group2 = group.clone();
                ctx.db
                    .write_async(move |c| {
                        let tx = c.transaction()?;
                        for item in &group2 {
                            let wait = if limited {
                                1_000
                            } else if transient {
                                backoff_ms(item.attempts as u32, 60_000)
                            } else {
                                60_000
                            };
                            store::defer_fetch(&tx, item, now_ms() + wait as i64, !limited)?;
                        }
                        tx.commit()?;
                        Ok(())
                    })
                    .await?;
                return Err(e);
            }
        };
        ctx.set_offline(false);

        let me = ctx.me.clone();
        let with_body = format == ThreadFormat::Full;
        let changed = ctx
            .db
            .write_async(move |c| {
                let mut tx = c.transaction()?;
                let mut changed = Vec::new();
                for (item, res) in group.iter().zip(results) {
                    match res {
                        Ok(fetched) => {
                            // Ogni thread in un savepoint, con i panic presi: un messaggio che il parsing
                            // non regge non deve fermare la sync né far riscaricare il batch all'infinito.
                            let sp = tx.savepoint()?;
                            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<bool> {
                                match &fetched {
                                    Fetched::Thread(thread) => {
                                        // Una risposta più vecchia di quanto c'è già in locale non lo sovrascrive.
                                        let incoming = parse_i64(thread.history_id.as_deref()).unwrap_or(0);
                                        let local = store::thread_history_id(&sp, &thread.id)?;
                                        if local.is_none_or(|l| incoming >= l) {
                                            store::write_thread(&sp, thread, with_body, &me)?;
                                            return Ok(true);
                                        }
                                        Ok(false)
                                    }
                                    Fetched::Message(msg) => {
                                        store::write_messages(&sp, &item.thread_id, std::slice::from_ref(msg), with_body, &me)?;
                                        Ok(true)
                                    }
                                }
                            }));
                            match outcome {
                                Ok(Ok(written)) => {
                                    sp.commit()?;
                                    if written {
                                        changed.push(item.thread_id.clone());
                                    }
                                    store::dequeue_fetch(&tx, item)?;
                                }
                                failure => {
                                    drop(sp); // rollback del solo thread
                                    match failure {
                                        Ok(Err(e)) => tracing::warn!(error = %e, tentativi = item.attempts, "thread non salvabile"),
                                        // Il messaggio del panic può contenere testo della mail: nel log non va.
                                        _ => tracing::error!(tentativi = item.attempts, "errore interno salvando un thread"),
                                    }
                                    if item.attempts < 2 {
                                        store::defer_fetch(&tx, item, now_ms() + 60_000, true)?;
                                    } else {
                                        tracing::warn!("thread saltato dopo tre errori");
                                        store::dequeue_fetch(&tx, item)?;
                                    }
                                }
                            }
                        }
                        Err(Error::NotFound) => {
                            // Sparito su Gmail: il messaggio (se era uno solo) o il thread intero.
                            let gone = match &item.message_id {
                                Some(m) => store::delete_message(&tx, m)?.is_some(),
                                None => store::delete_thread(&tx, &item.thread_id)?,
                            };
                            if gone {
                                changed.push(item.thread_id.clone());
                            }
                            store::dequeue_fetch(&tx, item)?;
                        }
                        // La pausa la impone il limitatore: qui basta non riprenderlo subito.
                        Err(e) if e.is_rate_limit() => store::defer_fetch(&tx, item, now_ms() + 1_000, false)?,
                        // Token scaduto anche dopo il rinnovo: si riprova più tardi, non si rinuncia.
                        Err(Error::Api { status: 401, .. }) if item.attempts < 8 => {
                            store::defer_fetch(&tx, item, now_ms() + 5_000, true)?;
                        }
                        Err(e) if e.is_transient() && item.attempts < 8 => {
                            store::defer_fetch(&tx, item, now_ms() + backoff_ms(item.attempts as u32, 120_000) as i64, true)?;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "thread non scaricabile: rinuncio");
                            store::dequeue_fetch(&tx, item)?;
                        }
                    }
                }
                tx.commit()?;
                Ok(changed)
            })
            .await?;
        if !changed.is_empty() {
            ctx.emit(CoreEvent::ThreadsChanged { ids: changed });
            // Posta nuova: le sezioni con istruzioni la valutano subito, non al giro dopo.
            ctx.sections_notify.notify_one();
        }
    }
    Ok(n)
}

async fn maybe_finish_initial_sync<G: GmailApi>(ctx: &Ctx<G>) -> Result<()> {
    let finished = ctx
        .db
        .write_async(|c| {
            let state: Option<String> = c.query_row("SELECT initial_sync_state FROM account WHERE id = 1", [], |r| r.get(0)).ok();
            if state.as_deref() != Some("fetching") {
                return Ok(false);
            }
            let left: i64 = c.query_row("SELECT count(*) FROM fetch_queue", [], |r| r.get(0))?;
            if left > 0 {
                return Ok(false);
            }
            store::set_initial_sync_state(c, "done")?;
            Ok(true)
        })
        .await?;
    if finished {
        tracing::info!("prima sync completata");
        ctx.emit_status().await;
    }
    Ok(())
}

pub async fn run_fetcher<G: GmailApi>(ctx: Arc<Ctx<G>>, lane: Lane) {
    let notify = if lane.gmail_prio == Priority::Interactive { &ctx.fetch_urgent } else { &ctx.fetch_background };
    let mut since_status = std::time::Instant::now();
    loop {
        if ctx.is_auth_required() {
            notify.notified().await;
            continue;
        }
        match fetch_once(&ctx, lane).await {
            Ok(0) => {
                let _ = maybe_finish_initial_sync(&ctx).await;
                ctx.emit_status().await;
                let next = ctx.db.read_async(|c| store::earliest_deferred_fetch(c)).await.ok().flatten();
                let wait = next
                    .map(|t| Duration::from_millis((t - now_ms()).clamp(100, 60_000) as u64))
                    .unwrap_or(Duration::from_secs(60));
                tokio::select! {
                    _ = notify.notified() => {}
                    _ = tokio::time::sleep(wait) => {}
                }
            }
            Ok(_) => {
                if since_status.elapsed() >= Duration::from_millis(700) {
                    ctx.emit_status().await;
                    since_status = std::time::Instant::now();
                }
            }
            Err(e) => {
                if !matches!(e, Error::Network(_) | Error::AuthRevoked) {
                    tracing::warn!(lane = lane.name, error = %e, "download dei thread fallito");
                }
                ctx.emit_status().await;
                let wait = if ctx.is_offline() { ctx.cfg.offline_retry } else { Duration::from_secs(2) };
                tokio::select! {
                    _ = notify.notified() => {}
                    _ = tokio::time::sleep(wait) => {}
                }
            }
        }
    }
}

/// Il Mac che si risveglia: l'orologio di sistema salta in avanti rispetto al timer.
pub async fn run_wake_detector<G: GmailApi>(ctx: Arc<Ctx<G>>) {
    let tick = Duration::from_secs(5);
    loop {
        let before = SystemTime::now();
        tokio::time::sleep(tick).await;
        let elapsed = SystemTime::now().duration_since(before).unwrap_or_default();
        if elapsed > tick + Duration::from_secs(15) {
            tracing::info!(secondi = elapsed.as_secs(), "risveglio rilevato: sync immediata");
            ctx.wake_all();
        }
    }
}

/// Ripasso, una volta sola: fino al 9 novembre un access token scaduto a metà di un batch faceva
/// rinunciare ai thread (401 nelle risposte interne). Si rielenca la finestra e si scaricano solo i
/// thread che in locale mancano: elencare costa circa mille unità, i download solo i thread persi.
pub async fn repair_once<G: GmailApi>(ctx: Arc<Ctx<G>>) {
    const KEY: &str = "sync.repair_401";
    tokio::time::sleep(Duration::from_secs(20)).await;
    loop {
        let todo = ctx
            .db
            .read_async(|c| {
                let done = store::get_setting(c, KEY)?.is_some();
                let ready = store::account(c)?.is_some_and(|a| a.initial_sync_state == "done");
                Ok((done, ready))
            })
            .await;
        match todo {
            Ok((true, _)) => return,
            Ok((false, true)) => {}
            // Prima sync ancora in corso: il ripasso aspetta.
            _ => {
                tokio::time::sleep(Duration::from_secs(300)).await;
                continue;
            }
        }
        let started = now_ms();
        match list_window(&ctx).await {
            Ok(listed) => {
                let n = listed.len();
                // Restano in coda solo i thread che in locale non ci sono. Gli altri risultano cambiati solo
                // perché su Gmail l'historyId sale anche con le etichette, già applicate in locale dalla
                // history: riscaricarli costerebbe mezzo milione di unità per niente.
                let _ = ctx
                    .db
                    .write_async(move |c| {
                        let dropped = c.execute(
                            "DELETE FROM fetch_queue WHERE enqueued_at >= ?1 AND priority >= ?2
                               AND thread_id IN (SELECT id FROM threads)",
                            rusqlite::params![started, prio::INBOX_FIRST],
                        )?;
                        tracing::debug!(n = dropped, "ripasso: tolti dalla coda i thread già presenti");
                        store::set_setting(c, KEY, &serde_json::json!(now_ms()))
                    })
                    .await;
                let queued = ctx.db.read_async(|c| Ok(c.query_row("SELECT count(*) FROM fetch_queue", [], |r| r.get::<_, i64>(0))?)).await.unwrap_or(0);
                tracing::info!(elencati = n, in_coda = queued, "ripasso della finestra completato");
                ctx.emit_status().await;
                return;
            }
            Err(e) => {
                ctx.note_error(&e);
                if matches!(e, Error::AuthRevoked) {
                    return;
                }
                tracing::warn!(error = %e, "ripasso della finestra: riprovo più tardi");
                tokio::time::sleep(Duration::from_secs(120)).await;
            }
        }
    }
}

/// Avvia la prima sync (se serve) e poi la tiene viva: si riprova se la rete manca.
pub async fn run_bootstrap<G: GmailApi>(ctx: Arc<Ctx<G>>) {
    let mut attempt = 0u32;
    loop {
        match bootstrap(&ctx).await {
            Ok(()) => return,
            Err(e) => {
                ctx.note_error(&e);
                if matches!(e, Error::AuthRevoked) {
                    return;
                }
                tracing::warn!(error = %e, "prima sync: riprovo");
                ctx.emit_status().await;
                tokio::time::sleep(Duration::from_millis(backoff_ms(attempt, 60_000))).await;
                attempt += 1;
            }
        }
    }
}
