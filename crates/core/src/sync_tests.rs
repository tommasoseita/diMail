//! Prove del motore di sync e della coda in uscita contro un Gmail finto, in memoria.
//! Coprono il criterio di M1: le azioni fatte "su Gmail web" compaiono in locale e viceversa.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::broadcast;

use crate::account::AccountRuntime;
use crate::db::Db;
use crate::events::CoreEvent;
use crate::gmail::*;
use crate::outbox::Action;
use crate::store::{self, View, testdata};
use crate::sync::{Ctx, SyncConfig};
use crate::{Error, Result};

const ME: &str = "tommaso@esempio.it";

#[derive(Default)]
pub struct FakeState {
    pub threads: BTreeMap<String, Thread>,
    pub history_id: i64,
    pub history: Vec<HistoryRecord>,
    /// `history.list` con uno start sotto questa soglia risponde 404.
    pub min_history: i64,
    pub fail_ops_on: HashSet<String>,
    pub offline: bool,
    pub calls: Vec<&'static str>,
    /// Messaggi inviati: (raw in base64url, thread).
    pub sent: Vec<(String, Option<String>)>,
    pub fail_send: Option<u16>,
    pub user_labels: Vec<Label>,
    /// Allegati grandi per `attachmentId`; `stale_attachments` simula un ID non più valido.
    pub attachments: BTreeMap<String, Vec<u8>>,
    pub stale_attachments: HashSet<String>,
    /// La firma di Gmail web per il mio indirizzo.
    pub signature: Option<String>,
}

fn mref(m: &Message) -> HistoryMessage {
    HistoryMessage { message: MessageRef { id: m.id.clone(), thread_id: Some(m.thread_id.clone()), label_ids: m.label_ids.clone() } }
}

impl FakeState {
    /// Nuovo record di history: come Gmail, porta l'`historyId` del thread e dei messaggi toccati.
    fn record(&mut self, thread_id: &str, touched: &[String], fill: impl FnOnce(&mut HistoryRecord, &[Message])) {
        self.history_id += 1;
        let h = self.history_id.to_string();
        let mut msgs = Vec::new();
        if let Some(t) = self.threads.get_mut(thread_id) {
            t.history_id = Some(h.clone());
            for m in t.messages.iter_mut().filter(|m| touched.contains(&m.id)) {
                m.history_id = Some(h.clone());
                msgs.push(m.clone());
            }
        }
        let mut rec = HistoryRecord { id: Some(h), ..Default::default() };
        fill(&mut rec, &msgs);
        self.history.push(rec);
    }

    pub fn add_message(&mut self, thread_id: &str, msg_id: &str, date: i64, from: &str, labels: &[&str], body: &str) {
        let m = testdata::msg(msg_id, thread_id, date, from, labels, Some(body));
        self.threads
            .entry(thread_id.to_owned())
            .or_insert_with(|| Thread { id: thread_id.to_owned(), history_id: None, messages: vec![] })
            .messages
            .push(m);
        self.record(thread_id, &[msg_id.to_owned()], |r, msgs| r.messages_added = msgs.iter().map(mref).collect());
    }

    pub fn modify(&mut self, thread_id: &str, add: &[&str], remove: &[&str]) {
        let t = self.threads.get_mut(thread_id).expect("thread");
        let ids: Vec<String> = t.messages.iter().map(|m| m.id.clone()).collect();
        for m in &mut t.messages {
            m.label_ids.retain(|l| !remove.contains(&l.as_str()));
            for a in add {
                if !m.label_ids.iter().any(|l| l == a) {
                    m.label_ids.push(a.to_string());
                }
            }
        }
        let (has_add, has_remove) = (!add.is_empty(), !remove.is_empty());
        self.record(thread_id, &ids, |r, msgs| {
            if has_add {
                r.labels_added = msgs.iter().map(mref).collect();
            }
            if has_remove {
                r.labels_removed = msgs.iter().map(mref).collect();
            }
        });
    }

    pub fn delete_message(&mut self, thread_id: &str, msg_id: &str) {
        let t = self.threads.get_mut(thread_id).expect("thread");
        let i = t.messages.iter().position(|m| m.id == msg_id).expect("messaggio");
        let m = t.messages.remove(i);
        self.record(thread_id, &[], |r, _| r.messages_deleted = vec![mref(&m)]);
    }

    pub fn delete_thread(&mut self, thread_id: &str) {
        let t = self.threads.remove(thread_id).expect("thread");
        self.record(thread_id, &[], |r, _| r.messages_deleted = t.messages.iter().map(mref).collect());
    }

    fn find_message(&self, id: &str) -> Option<&Message> {
        self.threads.values().flat_map(|t| &t.messages).find(|m| m.id == id)
    }

    pub fn count(&self, call: &str) -> usize {
        self.calls.iter().filter(|c| **c == call).count()
    }

    pub fn labels_of(&self, thread_id: &str) -> Vec<String> {
        let mut all: Vec<String> = self.threads[thread_id].messages.iter().flat_map(|m| m.label_ids.clone()).collect();
        all.sort();
        all.dedup();
        all
    }
}

#[derive(Default)]
pub struct FakeGmail {
    pub state: Mutex<FakeState>,
}

impl FakeGmail {
    fn s(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap()
    }
    fn check_online(&self, call: &'static str) -> Result<()> {
        let mut s = self.s();
        s.calls.push(call);
        if s.offline { Err(Error::Network("offline".into())) } else { Ok(()) }
    }
}

fn label(id: &str, kind: &str) -> Label {
    Label {
        id: id.into(),
        name: id.into(),
        kind: Some(kind.into()),
        message_list_visibility: None,
        label_list_visibility: None,
        messages_total: None,
        messages_unread: None,
        threads_total: None,
        threads_unread: None,
        color: None,
    }
}

impl GmailApi for FakeGmail {
    async fn profile(&self) -> Result<Profile> {
        self.check_online("profile")?;
        let s = self.s();
        Ok(Profile { email_address: ME.into(), messages_total: None, threads_total: Some(s.threads.len() as i64), history_id: s.history_id.to_string() })
    }

    async fn labels(&self) -> Result<Vec<Label>> {
        self.check_online("labels")?;
        let s = self.s();
        Ok(["INBOX", "UNREAD", "STARRED", "TRASH", "SENT", "IMPORTANT"]
            .iter()
            .map(|l| label(l, "system"))
            .chain([label("Label_1", "user")])
            .chain(s.user_labels.iter().cloned())
            .collect())
    }

    async fn list_threads(&self, req: ListThreads, _p: Priority) -> Result<ThreadsPage> {
        self.check_online("threads.list")?;
        let s = self.s();
        let mut ids: Vec<&Thread> = s
            .threads
            .values()
            .filter(|t| {
                let labels: Vec<&String> = t.messages.iter().flat_map(|m| &m.label_ids).collect();
                req.label_ids.iter().all(|l| labels.contains(&l))
                    && !labels.iter().any(|l| *l == "TRASH" || *l == "SPAM")
                    && t.messages.iter().any(|m| matches_q(req.q.as_deref(), m))
            })
            .collect();
        ids.sort_by_key(|t| std::cmp::Reverse(t.messages.last().and_then(|m| m.internal_date.clone())));
        let offset: usize = req.page_token.as_deref().map(|p| p.parse().unwrap()).unwrap_or(0);
        let page: Vec<ThreadRef> = ids
            .iter()
            .skip(offset)
            .take(req.max_results as usize)
            .map(|t| ThreadRef { id: t.id.clone(), history_id: t.history_id.clone() })
            .collect();
        let next = (offset + page.len() < ids.len()).then(|| (offset + page.len()).to_string());
        Ok(ThreadsPage { threads: page, next_page_token: next, result_size_estimate: Some(ids.len() as i64) })
    }

    async fn history(&self, start: i64, _page: Option<String>) -> Result<HistoryPage> {
        self.check_online("history.list")?;
        let s = self.s();
        if start < s.min_history {
            return Err(Error::NotFound);
        }
        let history = s
            .history
            .iter()
            .filter(|r| r.id.as_deref().and_then(|h| h.parse::<i64>().ok()).is_some_and(|h| h > start))
            .cloned()
            .collect();
        Ok(HistoryPage { history, next_page_token: None, history_id: Some(s.history_id.to_string()) })
    }

    async fn list_messages(&self, req: ListThreads, _p: Priority) -> Result<MessagesPage> {
        self.check_online("messages.list")?;
        let s = self.s();
        let mut all: Vec<&Message> = s
            .threads
            .values()
            .flat_map(|t| &t.messages)
            .filter(|m| !m.label_ids.iter().any(|l| l == "TRASH" || l == "SPAM") && matches_q(req.q.as_deref(), m))
            .collect();
        all.sort_by_key(|m| std::cmp::Reverse(m.internal_date.as_deref().and_then(|d| d.parse::<i64>().ok())));
        let offset: usize = req.page_token.as_deref().map(|p| p.parse().unwrap()).unwrap_or(0);
        let page: Vec<MessageRef> = all
            .iter()
            .skip(offset)
            .take(req.max_results as usize)
            .map(|m| MessageRef { id: m.id.clone(), thread_id: Some(m.thread_id.clone()), label_ids: vec![] })
            .collect();
        let next = (offset + page.len() < all.len()).then(|| (offset + page.len()).to_string());
        Ok(MessagesPage { messages: page, next_page_token: next })
    }

    async fn get_threads(&self, ids: &[String], format: ThreadFormat, _p: Priority) -> Result<Vec<Result<Thread>>> {
        self.check_online("threads.get")?;
        let s = self.s();
        Ok(ids
            .iter()
            .map(|id| {
                let mut t = s.threads.get(id).cloned().ok_or(Error::NotFound)?;
                if format == ThreadFormat::Metadata {
                    t.messages.iter_mut().for_each(strip_body);
                }
                Ok(t)
            })
            .collect())
    }

    async fn get_messages(&self, ids: &[String], format: ThreadFormat, _p: Priority) -> Result<Vec<Result<Message>>> {
        self.check_online("messages.get")?;
        let s = self.s();
        Ok(ids
            .iter()
            .map(|id| {
                let mut m = s.find_message(id).cloned().ok_or(Error::NotFound)?;
                if format == ThreadFormat::Metadata {
                    strip_body(&mut m);
                }
                Ok(m)
            })
            .collect())
    }

    async fn batch_modify_messages(&self, ids: &[String], add: &[String], remove: &[String]) -> Result<()> {
        self.check_online("messages.batchModify")?;
        let mut s = self.s();
        let threads: Vec<(String, Vec<String>)> = s
            .threads
            .values()
            .map(|t| (t.id.clone(), t.messages.iter().filter(|m| ids.contains(&m.id)).map(|m| m.id.clone()).collect::<Vec<_>>()))
            .filter(|(_, m)| !m.is_empty())
            .collect();
        for (thread_id, msgs) in threads {
            let t = s.threads.get_mut(&thread_id).unwrap();
            for m in t.messages.iter_mut().filter(|m| msgs.contains(&m.id)) {
                m.label_ids.retain(|l| !remove.contains(l));
                for a in add {
                    if !m.label_ids.contains(a) {
                        m.label_ids.push(a.clone());
                    }
                }
            }
            let (has_add, has_remove) = (!add.is_empty(), !remove.is_empty());
            s.record(&thread_id, &msgs, |r, ms| {
                if has_add {
                    r.labels_added = ms.iter().map(mref).collect();
                }
                if has_remove {
                    r.labels_removed = ms.iter().map(mref).collect();
                }
            });
        }
        Ok(())
    }

    async fn apply_ops(&self, ops: &[RemoteOp]) -> Result<Vec<Result<()>>> {
        self.check_online("batch.modify")?;
        let mut s = self.s();
        Ok(ops
            .iter()
            .map(|op| {
                if s.fail_ops_on.contains(op.thread_id()) {
                    return Err(Error::Api { status: 400, reason: Some("invalidArgument".into()), message: "rifiutato".into(), retry_after: None });
                }
                if !s.threads.contains_key(op.thread_id()) {
                    return Err(Error::NotFound);
                }
                match op {
                    RemoteOp::Modify { thread_id, add, remove } => {
                        let add: Vec<&str> = add.iter().map(String::as_str).collect();
                        let remove: Vec<&str> = remove.iter().map(String::as_str).collect();
                        s.modify(thread_id, &add, &remove);
                    }
                    RemoteOp::Trash { thread_id } => s.modify(thread_id, &["TRASH"], &["INBOX"]),
                    RemoteOp::Untrash { thread_id } => s.modify(thread_id, &[], &["TRASH"]),
                }
                Ok(())
            })
            .collect())
    }

    async fn send_message(&self, raw: &str, thread_id: Option<&str>) -> Result<String> {
        self.check_online("messages.send")?;
        let mut s = self.s();
        if let Some(status) = s.fail_send {
            return Err(Error::Api { status, reason: None, message: "rifiutato".into(), retry_after: None });
        }
        s.sent.push((raw.to_owned(), thread_id.map(str::to_owned)));
        Ok(format!("sent{}", s.sent.len()))
    }

    async fn has_message_id(&self, rfc_message_id: &str) -> Result<bool> {
        self.check_online("messages.list")?;
        use base64::Engine;
        let s = self.s();
        Ok(s.sent.iter().any(|(raw, _)| {
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw.trim_end_matches('=')).unwrap_or_default();
            String::from_utf8_lossy(&bytes).contains(rfc_message_id)
        }))
    }

    async fn create_label(&self, name: &str) -> Result<Label> {
        self.check_online("labels.create")?;
        let mut s = self.s();
        let mut l = label(&format!("Label_{}", 100 + s.user_labels.len()), "user");
        l.name = name.into();
        s.user_labels.push(l.clone());
        Ok(l)
    }

    async fn get_attachment(&self, _message_id: &str, attachment_id: &str) -> Result<Vec<u8>> {
        self.check_online("attachments.get")?;
        let s = self.s();
        if s.stale_attachments.contains(attachment_id) {
            return Err(Error::Api { status: 400, reason: Some("invalidArgument".into()), message: "Invalid attachment token".into(), retry_after: None });
        }
        s.attachments.get(attachment_id).cloned().ok_or(Error::NotFound)
    }

    async fn send_as(&self) -> Result<Vec<SendAs>> {
        self.check_online("settings.sendAs.list")?;
        let s = self.s();
        Ok(vec![
            SendAs { send_as_email: "alias@esempio.it".into(), signature: Some("<b>Firma dell'alias</b>".into()), ..Default::default() },
            SendAs { send_as_email: ME.into(), signature: s.signature.clone(), is_primary: true, is_default: true, ..Default::default() },
        ])
    }

    async fn rename_label(&self, id: &str, name: &str) -> Result<Label> {
        self.check_online("labels.update")?;
        let mut s = self.s();
        let l = s.user_labels.iter_mut().find(|l| l.id == id).ok_or(Error::NotFound)?;
        l.name = name.into();
        Ok(l.clone())
    }
}

/// `newer_than:N[dm]` e `older_than:N[dm]` di Gmail, quanto basta per le prove.
fn matches_q(q: Option<&str>, m: &Message) -> bool {
    let Some(q) = q else { return true };
    let date: i64 = m.internal_date.as_deref().and_then(|d| d.parse().ok()).unwrap_or(0);
    q.split_whitespace().all(|term| {
        let Some((op, n)) = term.split_once(':') else { return true };
        if op == "in" {
            return m.label_ids.iter().any(|l| l.eq_ignore_ascii_case(n));
        }
        if op == "before" {
            use chrono::TimeZone;
            let d = chrono::NaiveDate::parse_from_str(n, "%Y/%m/%d").expect("data");
            let midnight = chrono::Local.from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap()).single().unwrap();
            return date < midnight.timestamp_millis();
        }
        let (num, unit) = n.split_at(n.len() - 1);
        let days: i64 = num.parse::<i64>().unwrap_or(0) * if unit == "m" { 31 } else { 1 };
        let cutoff = crate::util::now_ms() - days * 86_400_000;
        match op {
            "newer_than" => date >= cutoff,
            "older_than" => date < cutoff,
            _ => true,
        }
    })
}

fn strip_body(m: &mut Message) {
    if let Some(p) = &mut m.payload {
        p.body = None;
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    fake: Arc<FakeGmail>,
    rt: Arc<AccountRuntime<FakeGmail>>,
    events: broadcast::Receiver<CoreEvent>,
}

fn fast_cfg() -> SyncConfig {
    SyncConfig {
        foreground_interval: Duration::from_millis(40),
        background_interval: Duration::from_millis(40),
        offline_retry: Duration::from_millis(40),
        batch_size: 2, // batch piccoli: si provano più giri
        ..SyncConfig::default()
    }
}

async fn start(fake: FakeGmail) -> Harness {
    start_with(fake, Arc::new(crate::ai::NoLlm)).await
}

async fn start_with(fake: FakeGmail, llm: Arc<dyn crate::ai::Llm>) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("mail.db")).unwrap();
    db.write(|c| store::ensure_account(c, ME, Some("Tommaso"), None)).unwrap();
    let fake = Arc::new(fake);
    let (tx, rx) = broadcast::channel(1024);
    let ctx = Ctx::new(db, Arc::clone(&fake), ME.into(), fast_cfg(), tx);
    let rt = AccountRuntime::start_with_ai(ctx, llm);
    Harness { _dir: dir, fake, rt, events: rx }
}

async fn eventually<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("tempo scaduto: {what}");
}

impl Harness {
    async fn inbox_ids(&self) -> Vec<String> {
        self.rt.list_threads(View::Inbox, 100, 0).await.unwrap().into_iter().map(|t| t.id).collect()
    }
    async fn local(&self, id: &str) -> Option<store::ThreadSummary> {
        let id = id.to_owned();
        self.rt.ctx().db.read_async(move |c| store::thread_summary(c, &id)).await.unwrap()
    }
}

/// Come in Gmail, l'ID di un thread è quello del suo primo messaggio. `t2` ha anche una risposta.
fn mailbox() -> FakeGmail {
    let f = FakeGmail::default();
    {
        let mut s = f.s();
        let now = crate::util::now_ms();
        s.add_message("t1", "t1", now - 5000, "Anna <anna@cliente.it>", &["INBOX", "UNREAD"], "Preventivo per giovedì");
        s.add_message("t2", "t2", now - 4500, "Luca <luca@esempio.it>", &["INBOX"], "Riunione");
        s.add_message("t3", "t3", now - 3000, "news@shop.it", &["INBOX", "CATEGORY_PROMOTIONS"], "Saldi");
        s.add_message("t4", "t4", now - 2000, "Marco <marco@x.it>", &[], "Archiviato");
        s.add_message("t5", "t5", now - 1000, "Tommaso <tommaso@esempio.it>", &["SENT"], "Inviato da me");
        s.add_message("t2", "t2r", now - 500, "Tommaso <tommaso@esempio.it>", &["SENT"], "Ci sono");
        // Inbox mai svuotata: un thread di due anni fa, ancora in arrivo.
        s.add_message("t8", "t8", now - 2 * 365 * 86_400_000, "Vecchio <v@x.it>", &["INBOX"], "Di due anni fa");
    }
    f
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn first_sync_then_changes_flow_both_ways() {
    let mut h = start(mailbox()).await;

    // Prima sync completa: inbox e finestra storica.
    eventually("prima sync", || async {
        let s = h.rt.status().await.unwrap();
        s.initial_sync_state == "done" && s.threads_local == 6
    })
    .await;
    let mut inbox = h.inbox_ids().await;
    inbox.sort();
    assert_eq!(inbox, vec!["t1", "t2", "t3", "t8"], "anche l'inbox più vecchia della finestra");
    assert!(h.local("t1").await.unwrap().unread);
    let calls = h.fake.s().calls.clone();
    let pos = |c: &str| calls.iter().position(|x| *x == c).unwrap();
    assert!(pos("profile") < pos("threads.list"), "l'historyId si legge prima di elencare: {calls:?}");
    // I thread con un messaggio solo arrivano con messages.get; t2, con la risposta, intero.
    assert!(calls.contains(&"messages.get") && calls.contains(&"threads.get"), "{calls:?}");
    assert_eq!(h.local("t2").await.unwrap().message_count, 2);
    assert_eq!(h.local("t1").await.unwrap().message_count, 1);

    // Da "Gmail web" all'app: archiviato e letto altrove. Le etichette arrivano dalla history,
    // senza scaricare nulla.
    let gets = |h: &Harness| {
        let s = h.fake.s();
        s.count("threads.get") + s.count("messages.get")
    };
    let before = gets(&h);
    h.fake.s().modify("t1", &[], &["INBOX", "UNREAD"]);
    eventually("archiviazione da Gmail web", || async { h.local("t1").await.is_some_and(|t| !t.in_inbox && !t.unread) }).await;
    assert_eq!(gets(&h), before, "un cambio di etichette non costa download");

    // Nuova email in arrivo, poi una risposta nello stesso thread: si scarica solo il messaggio nuovo.
    h.fake.s().add_message("t6", "t6", crate::util::now_ms(), "Sara <sara@cliente.it>", &["INBOX", "UNREAD"], "Nuova richiesta");
    eventually("nuova email", || async { h.inbox_ids().await.contains(&"t6".to_string()) }).await;
    let threads_get = h.fake.s().count("threads.get");
    h.fake.s().add_message("t6", "t6b", crate::util::now_ms(), "Sara <sara@cliente.it>", &["INBOX", "UNREAD"], "Dimenticavo");
    eventually("risposta nel thread", || async { h.local("t6").await.is_some_and(|t| t.message_count == 2) }).await;
    assert_eq!(h.fake.s().count("threads.get"), threads_get, "la risposta arriva con messages.get");

    // Messaggio cancellato su Gmail: sparisce anche in locale.
    h.fake.s().delete_message("t6", "t6b");
    eventually("messaggio cancellato", || async { h.local("t6").await.is_some_and(|t| t.message_count == 1) }).await;

    // Dall'app a Gmail: l'interfaccia cambia subito, Gmail poco dopo.
    let ops = h.rt.act(vec!["t2".into()], Action::Archive).await.unwrap();
    assert!(!h.local("t2").await.unwrap().in_inbox, "aggiornamento ottimistico immediato");
    eventually("archiviazione su Gmail", || async { !h.fake.s().labels_of("t2").contains(&"INBOX".to_string()) }).await;

    // Annulla: torna in inbox, anche su Gmail.
    h.rt.undo(ops).await.unwrap();
    assert!(h.local("t2").await.unwrap().in_inbox);
    eventually("annulla su Gmail", || async { h.fake.s().labels_of("t2").contains(&"INBOX".to_string()) }).await;

    // Gmail rifiuta: lo stato locale torna indietro e arriva un avviso.
    h.fake.s().fail_ops_on.insert("t3".into());
    h.rt.act(vec!["t3".into()], Action::Archive).await.unwrap();
    assert!(!h.local("t3").await.unwrap().in_inbox);
    eventually("ripristino dopo rifiuto", || async { h.local("t3").await.is_some_and(|t| t.in_inbox) }).await;
    let mut got_failure = false;
    while let Ok(e) = h.events.try_recv() {
        got_failure |= matches!(e, CoreEvent::ActionFailed { .. });
    }
    assert!(got_failure);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_actions_wait_and_stale_history_triggers_full_resync() {
    let h = start(mailbox()).await;
    eventually("prima sync", || async { h.rt.status().await.unwrap().initial_sync_state == "done" }).await;

    // Senza rete: l'azione vale subito in locale e parte al ritorno della rete.
    h.fake.s().offline = true;
    h.rt.act(vec!["t1".into()], Action::Star).await.unwrap();
    assert!(h.local("t1").await.unwrap().starred);
    eventually("stato offline", || async { h.rt.status().await.unwrap().phase == "offline" }).await;
    // Una sync arrivata nel frattempo non annulla l'azione in coda.
    assert!(h.local("t1").await.unwrap().starred);
    h.fake.s().offline = false;
    eventually("stella su Gmail dopo il ritorno della rete", || async { h.fake.s().labels_of("t1").contains(&"STARRED".to_string()) }).await;

    // historyId scaduto (giorni offline): sync completa senza svuotare la cache.
    {
        let mut s = h.fake.s();
        s.delete_thread("t4");
        s.add_message("t7", "t7", crate::util::now_ms(), "Nuovo <n@x.it>", &["INBOX"], "Dopo giorni");
        s.min_history = s.history_id + 1;
    }
    eventually("sync completa", || async {
        let ids = h.inbox_ids().await;
        ids.contains(&"t7".to_string()) && h.local("t4").await.is_none()
    })
    .await;
    assert!(h.local("t2").await.is_some(), "la cache non si svuota");
    // Il nuovo punto di partenza è quello attuale: niente altri 404.
    let hist = h.rt.account().await.unwrap().unwrap().history_id.unwrap();
    assert!(hist >= h.fake.s().min_history - 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_done_clears_old_inbox_here_and_on_gmail_and_undo_brings_it_back() {
    let h = start(mailbox()).await;
    eventually("prima sync", || async { h.rt.status().await.unwrap().initial_sync_state == "done" }).await;

    // Più vecchi di 10 giorni: in inbox c'è solo t8, di due anni fa.
    let cutoff = crate::util::now_ms() - 10 * 86_400_000;
    let p = h.rt.bulk_preview(cutoff).await.unwrap();
    assert_eq!((p.local, p.estimate), (1, Some(1)), "{p:?}");

    let op = h.rt.bulk_done(cutoff).await.unwrap();
    assert!(!h.local("t8").await.unwrap().in_inbox, "in locale vale subito");
    assert!(h.local("t1").await.unwrap().in_inbox, "i recenti restano");
    eventually("sgombero su Gmail", || async { !h.fake.s().labels_of("t8").contains(&"INBOX".to_string()) }).await;
    assert!(h.fake.s().calls.contains(&"messages.batchModify"));
    // La history riporta il cambio, che in locale c'è già: niente ritorni in inbox.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!h.local("t8").await.unwrap().in_inbox);

    h.rt.bulk_undo(op).await.unwrap();
    assert!(h.local("t8").await.unwrap().in_inbox, "l'annulla vale subito in locale");
    eventually("annulla su Gmail", || async { h.fake.s().labels_of("t8").contains(&"INBOX".to_string()) }).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(h.local("t8").await.unwrap().in_inbox);
}

async fn first_sync(h: &Harness) {
    eventually("prima sync", || async {
        let s = h.rt.status().await.unwrap();
        s.initial_sync_state == "done" && s.queued == 0
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reply_leaves_after_the_undo_window_and_can_be_taken_back() {
    let h = start(mailbox()).await;
    first_sync(&h).await;
    let mut d = h.rt.reply_draft("t1".into(), crate::send::ReplyMode::Reply).await.unwrap();
    assert_eq!(d.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["anna@cliente.it"]);
    d.body = "Va bene, ci sentiamo giovedì.".into();

    // Annullato nei secondi dell'annulla: non parte e la bozza torna indietro.
    let q = h.rt.send(d.clone(), 5_000).await.unwrap();
    let back = h.rt.cancel_send(q.id).await.unwrap().expect("bozza");
    assert_eq!(back.body, d.body);

    let q = h.rt.send(d, 0).await.unwrap();
    eventually("inviata", || async { h.fake.s().sent.len() == 1 }).await;
    assert_eq!(h.fake.s().sent[0].1.as_deref(), Some("t1"), "nello stesso thread");
    assert!(h.rt.cancel_send(q.id).await.unwrap().is_none(), "partita: niente più annulla");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(h.fake.s().sent.len(), 1, "una volta sola");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_section_with_instructions_pulls_in_matching_threads() {
    let llm = Arc::new(crate::ai::FakeLlm::default());
    llm.replies.lock().unwrap().push_back(r#"{"risultati":[{"thread":"t1","sezioni":["Label_100"]},{"thread":"t3","sezioni":[]}]}"#.into());
    let h = start_with(mailbox(), llm.clone()).await;
    first_sync(&h).await;

    let s = h.rt.create_section("Clienti".into(), true, Some("Le email dei clienti".into())).await.unwrap();
    assert_eq!((s.label_id.as_str(), s.name.as_str(), s.show_as_tab), ("Label_100", "Clienti", true));
    eventually("etichetta su Gmail", || async { h.fake.s().labels_of("t1").contains(&"Label_100".to_owned()) }).await;

    let tab: Vec<String> = h.rt.list_threads(View::Section { id: "Label_100".into() }, 50, 0).await.unwrap().into_iter().map(|t| t.id).collect();
    assert_eq!(tab, ["t1"]);
    let important: Vec<String> = h.rt.list_threads(View::Important, 50, 0).await.unwrap().into_iter().map(|t| t.id).collect();
    assert!(!important.contains(&"t1".to_owned()), "sta nella sua scheda, non in Importanti");
    let req = llm.seen.lock().unwrap()[0].clone();
    assert!(req.user.contains("Le email dei clienti") && req.json);

    // Ogni thread in inbox valutato una volta sola.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(llm.seen.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cmd_j_writes_a_reply_from_the_draft_and_the_thread() {
    let llm = Arc::new(crate::ai::FakeLlm::default());
    llm.replies.lock().unwrap().push_back("```\nCiao Anna,\n\ngiovedì va benissimo.\n\nTommaso\n```".into());
    let h = start_with(mailbox(), llm.clone()).await;
    first_sync(&h).await;
    let req = crate::ai::WriteRequest {
        mode: "reply".into(),
        thread_id: Some("t1".into()),
        to: vec![],
        subject: "Re: Preventivo".into(),
        notes: "ok giovedi".into(),
    };
    let text = h.rt.ai_write(req).await.unwrap();
    assert_eq!(text, "Ciao Anna,\n\ngiovedì va benissimo.\n\nTommaso");
    let seen = llm.seen.lock().unwrap()[0].clone();
    assert!(seen.user.contains("ok giovedi") && seen.user.contains("THREAD"), "bozza e thread arrivano al modello");
    let used = h.rt.ai_status().await.unwrap().tokens_today;
    assert_eq!(used, 100, "il consumo si conta");
}

/// Un messaggio con due allegati: uno grande (per `attachmentId`) e uno piccolo (dentro il messaggio).
fn with_attachments(f: &FakeGmail) {
    use crate::util::b64url;
    let mut s = f.s();
    let now = crate::util::now_ms();
    s.add_message("t9", "t9", now - 100, "Vincenzo <v@x.it>", &["INBOX"], "In allegato la fattura");
    let m = s.threads.get_mut("t9").unwrap().messages.get_mut(0).unwrap();
    m.payload = Some(
        serde_json::from_value(serde_json::json!({
            "partId": "", "mimeType": "multipart/mixed",
            "headers": [{"name": "From", "value": "Vincenzo <v@x.it>"}, {"name": "Subject", "value": "Fattura"}],
            "parts": [
                {"partId": "0", "mimeType": "text/plain", "headers": [], "body": {"size": 22, "data": b64url(b"In allegato la fattura")}},
                {"partId": "1", "mimeType": "application/pdf", "filename": "Fattura 20-2026.pdf",
                 "headers": [{"name": "Content-Disposition", "value": "attachment; filename=\"Fattura 20-2026.pdf\""}],
                 "body": {"size": 9, "attachmentId": "ATT_NEW"}},
                {"partId": "2", "mimeType": "text/plain", "filename": "note.txt",
                 "headers": [{"name": "Content-Disposition", "value": "attachment"}],
                 "body": {"size": 5, "data": b64url(b"ciao!")}}
            ]
        }))
        .unwrap(),
    );
    s.attachments.insert("ATT_NEW".into(), b"%PDF-1.7\n".to_vec());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attachments_download_once_and_survive_a_stale_id() {
    let f = mailbox();
    with_attachments(&f);
    let h = start(f).await;
    first_sync(&h).await;
    let cache = tempfile::tempdir().unwrap();
    let ctx = h.rt.ctx();

    // Il database ha l'ID vecchio: Gmail lo rifiuta, si rilegge il messaggio e si usa quello nuovo.
    ctx.db
        .write(|c| Ok(c.execute("UPDATE attachments SET gmail_attachment_id = 'ATT_OLD' WHERE part_id = '1'", [])?))
        .unwrap();
    h.fake.s().stale_attachments.insert("ATT_OLD".into());

    let pdf = crate::attachments::fetch(ctx, cache.path(), "t9", "1").await.unwrap();
    assert_eq!(std::fs::read(&pdf.path).unwrap(), b"%PDF-1.7\n");
    assert_eq!(pdf.path.file_name().unwrap(), "Fattura 20-2026.pdf");
    let small = crate::attachments::fetch(ctx, cache.path(), "t9", "2").await.unwrap();
    assert_eq!(std::fs::read(&small.path).unwrap(), b"ciao!");

    // La seconda volta viene dalla cache: nessuna chiamata a Gmail.
    let calls = h.fake.s().count("attachments.get");
    crate::attachments::fetch(ctx, cache.path(), "t9", "1").await.unwrap();
    assert_eq!(h.fake.s().count("attachments.get"), calls);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_forward_carries_the_attachments_and_the_gmail_signature() {
    let f = mailbox();
    with_attachments(&f);
    f.s().signature = Some("<div><b>Tommaso Seita</b></div><script>x()</script>".into());
    let h = start(f).await;
    first_sync(&h).await;
    // Allegati già in cache, in una cartella temporanea: l'invio non tocca la cartella vera dell'account.
    let cache = tempfile::tempdir().unwrap();
    for part in ["1", "2"] {
        crate::attachments::fetch(h.rt.ctx(), cache.path(), "t9", part).await.unwrap();
    }

    let sig = h.rt.signature().await.unwrap().expect("firma");
    assert_eq!(sig.html, "<div><b>Tommaso Seita</b></div>", "ripulita");
    let mut d = h.rt.reply_draft("t9".into(), crate::send::ReplyMode::Forward).await.unwrap();
    assert_eq!(d.attachments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Fattura 20-2026.pdf", "note.txt"]);
    let csv = cache.path().join("preventivo.csv");
    std::fs::write(&csv, "a;b\n1;2\n").unwrap();
    let local = crate::attachments::local_file(&csv).unwrap();
    assert_eq!((local.mime_type.as_str(), local.size), ("text/csv", 8));
    assert!(crate::attachments::local_file(cache.path()).is_err(), "una cartella non si allega");
    d.attachments.push(local);
    d.to = vec![crate::mime::Addr { name: None, email: "luca@x.it".into() }];
    d.body = "Ti giro la fattura.".into();
    h.rt.send(d, 0).await.unwrap();
    eventually("inviata", || async { h.fake.s().sent.len() == 1 }).await;

    let raw = String::from_utf8(crate::util::b64url_decode(&h.fake.s().sent[0].0).unwrap()).unwrap();
    let mail = mailparse::parse_mail(raw.as_bytes()).unwrap();
    let mut files = Vec::new();
    let mut html = String::new();
    let mut stack = vec![&mail];
    while let Some(p) = stack.pop() {
        stack.extend(p.subparts.iter().rev());
        if let Some(name) = p.get_content_disposition().params.get("filename") {
            files.push((name.clone(), p.get_body_raw().unwrap()));
        } else if p.ctype.mimetype == "text/html" {
            html = p.get_body().unwrap();
        }
    }
    assert_eq!(
        files,
        [
            ("Fattura 20-2026.pdf".to_owned(), b"%PDF-1.7\n".to_vec()),
            ("note.txt".to_owned(), b"ciao!".to_vec()),
            ("preventivo.csv".to_owned(), b"a;b\n1;2\n".to_vec())
        ]
    );
    assert!(html.contains("<div>Ti giro la fattura.</div><br><div dir=\"ltr\" class=\"gmail_signature\""), "{html}");
    assert!(html.contains("Messaggio inoltrato"));

    // Partito il messaggio, la sua copia nella coda (con gli allegati) si svuota.
    eventually("coda svuotata", || async {
        h.rt.ctx().db.read(|c| Ok(c.query_row("SELECT raw FROM send_queue", [], |r| r.get::<_, String>(0))?)).unwrap().is_empty()
    })
    .await;
}
