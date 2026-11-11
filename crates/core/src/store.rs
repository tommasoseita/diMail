//! Letture e scritture su SQLite. Le funzioni prendono una `Connection` (o una `Transaction`,
//! che si comporta come tale): chi chiama decide i confini della transazione.

use std::collections::{BTreeSet, HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::gmail::{Label, Message, Thread};
use crate::mime::{Addr, ParsedBody, ParsedMessage, parse_message};
use crate::sanitize::{SANITIZER_VERSION, clean_html, html_to_text, strip_quotes};
use crate::util::{now_ms, parse_i64};
use crate::Result;

pub const INBOX: &str = "INBOX";
pub const UNREAD: &str = "UNREAD";
pub const STARRED: &str = "STARRED";
pub const IMPORTANT: &str = "IMPORTANT";
pub const TRASH: &str = "TRASH";
pub const SPAM: &str = "SPAM";
pub const DRAFT: &str = "DRAFT";
pub const SENT: &str = "SENT";

// ───────────────────────────── account ─────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct AccountRow {
    pub email: String,
    pub name: Option<String>,
    pub picture_url: Option<String>,
    pub history_id: Option<i64>,
    pub initial_sync_state: String,
    pub last_sync_at: Option<i64>,
    pub last_sync_error: Option<String>,
    pub messages_total: Option<i64>,
    pub threads_total: Option<i64>,
}

pub fn ensure_account(c: &Connection, email: &str, name: Option<&str>, picture: Option<&str>) -> Result<()> {
    c.execute(
        "INSERT INTO account (id, email, name, picture_url, created_at) VALUES (1, ?1, ?2, ?3, ?4)
         ON CONFLICT (id) DO UPDATE SET name = coalesce(excluded.name, name), picture_url = coalesce(excluded.picture_url, picture_url)",
        params![email, name, picture, now_ms()],
    )?;
    Ok(())
}

pub fn account(c: &Connection) -> Result<Option<AccountRow>> {
    Ok(c
        .query_row(
            "SELECT email, name, picture_url, history_id, initial_sync_state, last_sync_at, last_sync_error,
                    messages_total, threads_total FROM account WHERE id = 1",
            [],
            |r| {
                Ok(AccountRow {
                    email: r.get(0)?,
                    name: r.get(1)?,
                    picture_url: r.get(2)?,
                    history_id: r.get(3)?,
                    initial_sync_state: r.get(4)?,
                    last_sync_at: r.get(5)?,
                    last_sync_error: r.get(6)?,
                    messages_total: r.get(7)?,
                    threads_total: r.get(8)?,
                })
            },
        )
        .optional()?)
}

pub fn set_profile(c: &Connection, history_id: Option<i64>, messages_total: Option<i64>, threads_total: Option<i64>) -> Result<()> {
    c.execute(
        "UPDATE account SET history_id = coalesce(?1, history_id), messages_total = ?2, threads_total = ?3 WHERE id = 1",
        params![history_id, messages_total, threads_total],
    )?;
    Ok(())
}

pub fn set_history_id(c: &Connection, history_id: i64) -> Result<()> {
    c.execute("UPDATE account SET history_id = ?1 WHERE id = 1", [history_id])?;
    Ok(())
}

pub fn set_initial_sync_state(c: &Connection, state: &str) -> Result<()> {
    let now = now_ms();
    c.execute(
        "UPDATE account SET initial_sync_state = ?1,
            initial_sync_started_at = CASE WHEN ?1 = 'listing' THEN coalesce(initial_sync_started_at, ?2) ELSE initial_sync_started_at END,
            initial_sync_done_at = CASE WHEN ?1 = 'done' THEN ?2 ELSE initial_sync_done_at END
         WHERE id = 1",
        params![state, now],
    )?;
    Ok(())
}

pub fn set_sync_result(c: &Connection, error: Option<&str>) -> Result<()> {
    if error.is_none() {
        c.execute("UPDATE account SET last_sync_at = ?1, last_sync_error = NULL WHERE id = 1", [now_ms()])?;
    } else {
        c.execute("UPDATE account SET last_sync_error = ?1 WHERE id = 1", [error])?;
    }
    Ok(())
}

// ───────────────────────────── etichette ─────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct LabelRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub color_bg: Option<String>,
    pub color_text: Option<String>,
    pub label_list_visibility: Option<String>,
    pub threads_total: Option<i64>,
    pub threads_unread: Option<i64>,
    pub messages_unread: Option<i64>,
    /// Thread non letti in locale (per l'inbox: solo quelli ancora in inbox).
    pub unread_local: i64,
}

fn label_kind(l: &Label) -> &'static str {
    if l.id.starts_with("CATEGORY_") {
        "category"
    } else if l.kind.as_deref() == Some("system") {
        "system"
    } else {
        "user"
    }
}

pub fn replace_labels(c: &Connection, labels: &[Label]) -> Result<()> {
    let keep: HashSet<&str> = labels.iter().map(|l| l.id.as_str()).collect();
    let existing: Vec<String> = c.prepare("SELECT id FROM labels")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    for id in existing.iter().filter(|id| !keep.contains(id.as_str())) {
        c.execute("DELETE FROM labels WHERE id = ?1", [id])?;
    }
    let mut stmt = c.prepare(
        "INSERT INTO labels (id, name, type, color_bg, color_text, label_list_visibility, message_list_visibility,
                             messages_total, messages_unread, threads_total, threads_unread)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT (id) DO UPDATE SET name = excluded.name, type = excluded.type, color_bg = excluded.color_bg,
            color_text = excluded.color_text, label_list_visibility = excluded.label_list_visibility,
            message_list_visibility = excluded.message_list_visibility,
            messages_total = coalesce(excluded.messages_total, messages_total),
            messages_unread = coalesce(excluded.messages_unread, messages_unread),
            threads_total = coalesce(excluded.threads_total, threads_total),
            threads_unread = coalesce(excluded.threads_unread, threads_unread)",
    )?;
    for l in labels {
        let color = l.color.as_ref();
        stmt.execute(params![
            l.id,
            l.name,
            label_kind(l),
            color.and_then(|c| c.background_color.clone()),
            color.and_then(|c| c.text_color.clone()),
            l.label_list_visibility,
            l.message_list_visibility,
            l.messages_total,
            l.messages_unread,
            l.threads_total,
            l.threads_unread,
        ])?;
    }
    Ok(())
}

pub fn list_labels(c: &Connection) -> Result<Vec<LabelRow>> {
    let mut stmt = c.prepare(
        "SELECT l.id, l.name, l.type, l.color_bg, l.color_text, l.label_list_visibility, l.threads_total, l.threads_unread,
                l.messages_unread,
                CASE WHEN l.id = 'INBOX' THEN (SELECT count(*) FROM threads WHERE in_inbox = 1 AND unread = 1)
                     ELSE (SELECT count(*) FROM thread_labels tl JOIN threads t ON t.id = tl.thread_id
                           WHERE tl.label_id = l.id AND t.unread = 1) END
         FROM labels l ORDER BY CASE l.type WHEN 'system' THEN 0 WHEN 'category' THEN 1 ELSE 2 END, l.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(LabelRow {
            id: r.get(0)?,
            name: r.get(1)?,
            kind: r.get(2)?,
            color_bg: r.get(3)?,
            color_text: r.get(4)?,
            label_list_visibility: r.get(5)?,
            threads_total: r.get(6)?,
            threads_unread: r.get(7)?,
            messages_unread: r.get(8)?,
            unread_local: r.get(9)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// ───────────────────────────── scrittura dei thread ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    Written,
    Deleted,
}

struct ProcessedBody {
    text_plain: Option<String>,
    html_clean: Option<String>,
    text_unquoted: Option<String>,
}

fn process_body(b: &ParsedBody) -> ProcessedBody {
    let text_plain = b
        .text_plain
        .clone()
        .or_else(|| b.html.as_deref().map(html_to_text))
        .filter(|t| !t.trim().is_empty());
    let text_unquoted = text_plain.as_deref().map(strip_quotes);
    ProcessedBody { text_plain, html_clean: b.html.as_deref().map(clean_html), text_unquoted }
}

fn addrs_text(list: &[Addr]) -> String {
    list.iter()
        .map(|a| match &a.name {
            Some(n) => format!("{n} {}", a.email),
            None => a.email.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn index_message(c: &Connection, rowid: i64, pm: &ParsedMessage, body: Option<&ProcessedBody>) -> Result<()> {
    c.execute("DELETE FROM search_index WHERE rowid = ?1", [rowid])?;
    let to: Vec<Addr> = pm.to.iter().chain(&pm.cc).chain(&pm.bcc).cloned().collect();
    let body_text = body.and_then(|b| b.text_unquoted.as_deref().or(b.text_plain.as_deref())).unwrap_or("");
    let att_names = pm.attachments.iter().filter(|a| !a.is_inline).map(|a| a.filename.as_str()).collect::<Vec<_>>().join(" ");
    c.execute(
        "INSERT INTO search_index (rowid, subject, from_text, to_text, body, attachment_names) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![rowid, pm.subject, addrs_text(pm.from.as_slice()), addrs_text(&to), body_text, att_names],
    )?;
    Ok(())
}

fn write_body(c: &Connection, rowid: i64, pm: &ParsedMessage, body: &ProcessedBody) -> Result<()> {
    c.execute(
        "INSERT INTO message_bodies (message_rowid, text_plain, html_clean, text_unquoted, sanitizer_version)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (message_rowid) DO UPDATE SET text_plain = excluded.text_plain, html_clean = excluded.html_clean,
            text_unquoted = excluded.text_unquoted, sanitizer_version = excluded.sanitizer_version",
        params![rowid, body.text_plain, body.html_clean, body.text_unquoted, SANITIZER_VERSION],
    )?;
    c.execute("DELETE FROM attachments WHERE message_rowid = ?1", [rowid])?;
    let mut stmt = c.prepare(
        "INSERT OR REPLACE INTO attachments (message_rowid, part_id, gmail_attachment_id, filename, mime_type, size, content_id, is_inline)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for a in &pm.attachments {
        stmt.execute(params![rowid, a.part_id, a.gmail_attachment_id, a.filename, a.mime_type, a.size, a.content_id, a.is_inline])?;
    }
    c.execute("UPDATE messages SET has_body = 1 WHERE rowid = ?1", [rowid])?;
    Ok(())
}

fn update_contacts(c: &Connection, pm: &ParsedMessage, me: &str) -> Result<()> {
    if pm.has_label(DRAFT) {
        return Ok(());
    }
    let from_me = pm.has_label(SENT) || pm.from.as_ref().is_some_and(|f| f.email.eq_ignore_ascii_case(me));
    if from_me {
        let mut seen = HashSet::new();
        for a in pm.to.iter().chain(&pm.cc).chain(&pm.bcc) {
            if a.email.eq_ignore_ascii_case(me) || !seen.insert(a.email.clone()) {
                continue;
            }
            // La prima volta che scrivo a qualcuno, i suoi thread passano in Importanti (sez. 8).
            let first_time = c
                .query_row("SELECT sent_count FROM contacts WHERE email = ?1", [&a.email], |r| r.get::<_, i64>(0))
                .optional()?
                .unwrap_or(0)
                == 0;
            c.execute(
                "INSERT INTO contacts (email, name, sent_count, last_sent_at) VALUES (?1, ?2, 1, ?3)
                 ON CONFLICT (email) DO UPDATE SET name = coalesce(excluded.name, name), sent_count = sent_count + 1,
                    last_sent_at = max(coalesce(last_sent_at, 0), excluded.last_sent_at)",
                params![a.email, a.name, pm.internal_date],
            )?;
            if first_time {
                crate::split::recompute_sender(c, &a.email)?;
            }
        }
    } else if let Some(f) = &pm.from {
        c.execute(
            "INSERT INTO contacts (email, name, received_count, last_received_at) VALUES (?1, ?2, 1, ?3)
             ON CONFLICT (email) DO UPDATE SET name = coalesce(excluded.name, name), received_count = received_count + 1,
                last_received_at = max(coalesce(last_received_at, 0), excluded.last_received_at)",
            params![f.email, f.name, pm.internal_date],
        )?;
    }
    Ok(())
}

/// Scrive un thread scaricato da Gmail: messaggi, corpi, allegati, indice di ricerca e contatti,
/// poi riapplica le azioni ancora in coda (che hanno la precedenza sullo stato del server).
pub fn write_thread(c: &Connection, t: &Thread, with_body: bool, me: &str) -> Result<WriteOutcome> {
    if t.messages.is_empty() {
        delete_thread(c, &t.id)?;
        return Ok(WriteOutcome::Deleted);
    }
    let parsed: Vec<ParsedMessage> = t.messages.iter().map(|m| parse_message(m, with_body)).collect();
    let history_id = parse_i64(t.history_id.as_deref())
        .or_else(|| parsed.iter().filter_map(|m| m.history_id).max())
        .unwrap_or(0);
    upsert_messages(c, &t.id, &parsed, true, me)?;
    c.execute(
        "UPDATE threads SET history_id = ?2, fetched_at = ?3 WHERE id = ?1",
        params![t.id, history_id, now_ms()],
    )?;
    recompute_thread(c, &t.id)?;
    reapply_pending_ops(c, &t.id)?;
    Ok(WriteOutcome::Written)
}

/// Scrive singoli messaggi di un thread (da `messages.get`) senza toccare gli altri.
pub fn write_messages(c: &Connection, thread_id: &str, msgs: &[Message], with_body: bool, me: &str) -> Result<()> {
    if msgs.is_empty() {
        return Ok(());
    }
    let parsed: Vec<ParsedMessage> = msgs.iter().map(|m| parse_message(m, with_body)).collect();
    let history_id = parsed.iter().filter_map(|m| m.history_id).max().unwrap_or(0);
    upsert_messages(c, thread_id, &parsed, false, me)?;
    c.execute(
        "UPDATE threads SET history_id = max(history_id, ?2), fetched_at = ?3 WHERE id = ?1",
        params![thread_id, history_id, now_ms()],
    )?;
    recompute_thread(c, thread_id)?;
    reapply_pending_ops(c, thread_id)?;
    Ok(())
}

/// Inserisce o aggiorna i messaggi di un thread. Con `authoritative` (thread intero da Gmail)
/// cancella anche quelli che Gmail non ha più.
fn upsert_messages(c: &Connection, thread_id: &str, parsed: &[ParsedMessage], authoritative: bool, me: &str) -> Result<()> {
    c.execute(
        "INSERT INTO threads (id, history_id, last_date, message_count, fetched_at) VALUES (?1, 0, 0, 0, ?2)
         ON CONFLICT (id) DO NOTHING",
        params![thread_id, now_ms()],
    )?;

    let existing: HashMap<String, (i64, bool)> = c
        .prepare("SELECT id, rowid, has_body FROM messages WHERE thread_id = ?1")?
        .query_map([thread_id], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, i64>(1)?, r.get::<_, bool>(2)?))))?
        .collect::<Result<_, _>>()?;
    if authoritative {
        let incoming: HashSet<&str> = parsed.iter().map(|m| m.id.as_str()).collect();
        for (id, (rowid, _)) in &existing {
            if !incoming.contains(id.as_str()) {
                c.execute("DELETE FROM search_index WHERE rowid = ?1", [rowid])?;
                c.execute("DELETE FROM messages WHERE rowid = ?1", [rowid])?;
            }
        }
    }

    for pm in parsed {
        let labels_json = serde_json::to_string(&pm.label_ids)?;
        let body = pm.body.as_ref().map(process_body);
        match existing.get(&pm.id) {
            Some(&(rowid, had_body)) => {
                // I messaggi Gmail non cambiano: cambiano solo le etichette. Una risposta più vecchia
                // di un cambiamento già applicato dalla history non lo sovrascrive.
                c.execute(
                    "UPDATE messages SET label_ids = ?1, history_id = ?2, snippet = ?3
                     WHERE rowid = ?4 AND (history_id IS NULL OR ?2 IS NULL OR history_id <= ?2)",
                    params![labels_json, pm.history_id, pm.snippet, rowid],
                )?;
                if let Some(b) = &body
                    && !had_body
                {
                    write_body(c, rowid, pm, b)?;
                    index_message(c, rowid, pm, Some(b))?;
                }
            }
            None => {
                c.execute(
                    "INSERT INTO messages (id, thread_id, history_id, internal_date, label_ids, snippet, from_email, from_name,
                        to_json, cc_json, bcc_json, reply_to_json, subject, rfc_message_id, in_reply_to, references_hdr,
                        list_unsubscribe, list_id, precedence, auto_submitted, size_estimate)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
                    params![
                        pm.id,
                        thread_id,
                        pm.history_id,
                        pm.internal_date,
                        labels_json,
                        pm.snippet,
                        pm.from.as_ref().map(|a| &a.email),
                        pm.from.as_ref().and_then(|a| a.name.as_ref()),
                        serde_json::to_string(&pm.to)?,
                        serde_json::to_string(&pm.cc)?,
                        serde_json::to_string(&pm.bcc)?,
                        serde_json::to_string(&pm.reply_to)?,
                        pm.subject,
                        pm.rfc_message_id,
                        pm.in_reply_to,
                        pm.references,
                        pm.list_unsubscribe,
                        pm.list_id,
                        pm.precedence,
                        pm.auto_submitted,
                        pm.size_estimate,
                    ],
                )?;
                let rowid = c.last_insert_rowid();
                // Un messaggio nuovo: la sintesi AI del thread va rifatta.
                c.execute("DELETE FROM ai_summaries WHERE thread_id = ?1", [thread_id])?;
                if let Some(b) = &body {
                    write_body(c, rowid, pm, b)?;
                }
                index_message(c, rowid, pm, body.as_ref())?;
                update_contacts(c, pm, me)?;
            }
        }
    }
    Ok(())
}

/// Cambio di etichette letto dalla history: si applica in locale, senza scaricare nulla.
/// Restituisce il thread del messaggio e se è cambiato qualcosa, `None` se il messaggio non è in
/// locale. Le etichette già uguali (per esempio dopo un'azione fatta da qui o uno sgombero) non
/// ricalcolano il thread: dopo uno sgombero arrivano a centinaia di migliaia.
pub fn apply_history_labels(c: &Connection, message_id: &str, labels: &[String], history_id: i64) -> Result<Option<(String, bool)>> {
    let row: Option<(i64, String, Option<i64>, String)> = c
        .query_row("SELECT rowid, thread_id, history_id, label_ids FROM messages WHERE id = ?1", [message_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()?;
    let Some((rowid, thread_id, local, current)) = row else { return Ok(None) };
    if local.is_some_and(|h| h > history_id) {
        return Ok(Some((thread_id, false)));
    }
    let current: BTreeSet<String> = serde_json::from_str::<Vec<String>>(&current).unwrap_or_default().into_iter().collect();
    let incoming: BTreeSet<String> = labels.iter().cloned().collect();
    c.execute("UPDATE messages SET history_id = ?2 WHERE rowid = ?1", params![rowid, history_id])?;
    c.execute("UPDATE threads SET history_id = max(history_id, ?2) WHERE id = ?1", params![thread_id, history_id])?;
    if current == incoming {
        return Ok(Some((thread_id, false)));
    }
    c.execute("UPDATE messages SET label_ids = ?2 WHERE rowid = ?1", params![rowid, serde_json::to_string(labels)?])?;
    recompute_thread(c, &thread_id)?;
    reapply_pending_ops(c, &thread_id)?;
    Ok(Some((thread_id, true)))
}

/// Messaggio cancellato su Gmail. Se era l'ultimo, sparisce il thread.
/// Restituisce il thread, `None` se il messaggio non era in locale.
pub fn delete_message(c: &Connection, message_id: &str) -> Result<Option<String>> {
    let row: Option<(i64, String)> = c
        .query_row("SELECT rowid, thread_id FROM messages WHERE id = ?1", [message_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let Some((rowid, thread_id)) = row else { return Ok(None) };
    c.execute("DELETE FROM search_index WHERE rowid = ?1", [rowid])?;
    c.execute("DELETE FROM messages WHERE rowid = ?1", [rowid])?;
    let left: i64 = c.query_row("SELECT count(*) FROM messages WHERE thread_id = ?1", [&thread_id], |r| r.get(0))?;
    if left == 0 {
        delete_thread(c, &thread_id)?;
    } else {
        recompute_thread(c, &thread_id)?;
        reapply_pending_ops(c, &thread_id)?;
    }
    Ok(Some(thread_id))
}

pub fn has_thread(c: &Connection, id: &str) -> Result<bool> {
    Ok(c.query_row("SELECT 1 FROM threads WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some())
}

/// L'unico messaggio del thread, se ne ha uno solo.
pub fn only_message(c: &Connection, thread_id: &str) -> Result<Option<String>> {
    let ids: Vec<String> = c
        .prepare("SELECT id FROM messages WHERE thread_id = ?1 LIMIT 2")?
        .query_map([thread_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(if ids.len() == 1 { ids.into_iter().next() } else { None })
}

pub fn delete_thread(c: &Connection, id: &str) -> Result<bool> {
    c.execute(
        "DELETE FROM search_index WHERE rowid IN (SELECT rowid FROM messages WHERE thread_id = ?1)",
        [id],
    )?;
    Ok(c.execute("DELETE FROM threads WHERE id = ?1", [id])? > 0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Participant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub email: String,
    pub unread: bool,
}

/// Ricalcola i campi aggregati del thread dai suoi messaggi.
fn recompute_thread(c: &Connection, id: &str) -> Result<()> {
    struct M {
        date: i64,
        labels: Vec<String>,
        from_email: Option<String>,
        from_name: Option<String>,
        snippet: String,
        subject: String,
        has_body: bool,
    }
    let msgs: Vec<M> = c
        .prepare(
            "SELECT internal_date, label_ids, from_email, from_name, snippet, subject, has_body
             FROM messages WHERE thread_id = ?1 ORDER BY internal_date, rowid",
        )?
        .query_map([id], |r| {
            let labels: String = r.get(1)?;
            Ok(M {
                date: r.get(0)?,
                labels: serde_json::from_str(&labels).unwrap_or_default(),
                from_email: r.get(2)?,
                from_name: r.get(3)?,
                snippet: r.get(4)?,
                subject: r.get(5)?,
                has_body: r.get(6)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    if msgs.is_empty() {
        delete_thread(c, id)?;
        return Ok(());
    }
    let is = |m: &M, l: &str| m.labels.iter().any(|x| x == l);
    let non_draft: Vec<&M> = msgs.iter().filter(|m| !is(m, DRAFT)).collect();
    let last = non_draft.last().copied().unwrap_or_else(|| msgs.last().expect("non vuoto"));
    let labels: BTreeSet<&str> = msgs.iter().flat_map(|m| m.labels.iter().map(String::as_str)).collect();
    let any = |l: &str| labels.contains(l);

    let mut participants: Vec<Participant> = Vec::new();
    for m in &non_draft {
        let Some(email) = &m.from_email else { continue };
        let unread = is(m, UNREAD);
        match participants.iter_mut().find(|p| &p.email == email) {
            Some(p) => {
                p.unread |= unread;
                if p.name.is_none() {
                    p.name = m.from_name.clone();
                }
            }
            None => participants.push(Participant { name: m.from_name.clone(), email: email.clone(), unread }),
        }
    }
    let subject = msgs.iter().map(|m| m.subject.as_str()).find(|s| !s.is_empty()).unwrap_or("");
    let has_attachments: bool = c.query_row(
        "SELECT EXISTS (SELECT 1 FROM attachments a JOIN messages m ON m.rowid = a.message_rowid
                        WHERE m.thread_id = ?1 AND a.is_inline = 0)",
        [id],
        |r| r.get(0),
    )?;

    c.execute(
        "UPDATE threads SET subject = ?2, snippet = ?3, last_date = ?4, participants = ?5, message_count = ?6,
            unread = ?7, starred = ?8, in_inbox = ?9, important = ?10, has_attachments = ?11, has_draft = ?12,
            bodies_complete = ?13
         WHERE id = ?1",
        params![
            id,
            subject,
            last.snippet,
            last.date,
            serde_json::to_string(&participants)?,
            msgs.len() as i64,
            any(UNREAD),
            any(STARRED),
            any(INBOX) && !any(TRASH) && !any(SPAM),
            any(IMPORTANT),
            has_attachments,
            any(DRAFT),
            msgs.iter().all(|m| m.has_body),
        ],
    )?;
    c.execute("DELETE FROM thread_labels WHERE thread_id = ?1", [id])?;
    let mut stmt = c.prepare("INSERT INTO thread_labels (thread_id, label_id) VALUES (?1, ?2)")?;
    for l in labels {
        stmt.execute(params![id, l])?;
    }
    crate::split::recompute(c, id)?;
    Ok(())
}

/// Applica in locale un cambio di etichette su tutti i messaggi del thread, come `threads.modify`.
pub fn apply_label_delta(c: &Connection, thread_id: &str, add: &[String], remove: &[String]) -> Result<bool> {
    let msgs: Vec<(i64, String)> = c
        .prepare("SELECT rowid, label_ids FROM messages WHERE thread_id = ?1")?
        .query_map([thread_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    if msgs.is_empty() {
        return Ok(false);
    }
    for (rowid, json) in msgs {
        let mut labels: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
        let before = labels.clone();
        labels.retain(|l| !remove.contains(l));
        for a in add {
            if !labels.contains(a) {
                labels.push(a.clone());
            }
        }
        if labels != before {
            c.execute("UPDATE messages SET label_ids = ?1 WHERE rowid = ?2", params![serde_json::to_string(&labels)?, rowid])?;
        }
    }
    recompute_thread(c, thread_id)?;
    Ok(true)
}

/// Cambio di etichette locale corrispondente a un'operazione della coda in uscita.
pub fn local_delta(kind: &str, payload: &str) -> (Vec<String>, Vec<String>) {
    #[derive(Deserialize, Default)]
    struct P {
        #[serde(default)]
        add: Vec<String>,
        #[serde(default)]
        remove: Vec<String>,
    }
    match kind {
        "modify" => {
            let p: P = serde_json::from_str(payload).unwrap_or_default();
            (p.add, p.remove)
        }
        "trash" => (vec![TRASH.into()], vec![INBOX.into()]),
        "untrash" => (vec![], vec![TRASH.into()]),
        _ => (vec![], vec![]),
    }
}

/// Le operazioni in attesa hanno la precedenza sugli aggiornamenti della sync (sez. 5.4, punto 5).
pub fn reapply_pending_ops(c: &Connection, thread_id: &str) -> Result<()> {
    let ops: Vec<(String, String)> = c
        .prepare("SELECT kind, payload FROM outbox_ops WHERE thread_id = ?1 AND state IN ('pending', 'in_flight') ORDER BY id")?
        .query_map([thread_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (kind, payload) in ops {
        let (add, remove) = local_delta(&kind, &payload);
        apply_label_delta(c, thread_id, &add, &remove)?;
    }
    Ok(())
}

pub fn thread_history_id(c: &Connection, id: &str) -> Result<Option<i64>> {
    Ok(c.query_row("SELECT history_id FROM threads WHERE id = ?1", [id], |r| r.get(0)).optional()?)
}

// ───────────────────────────── coda dei thread da scaricare ─────────────────────────────

/// Priorità della coda (più basso = prima).
pub mod prio {
    /// Thread aperto dall'utente senza corpo, o da riallineare dopo un errore.
    pub const USER: i64 = 0;
    /// Cambiamenti trovati dalla sync incrementale.
    pub const INCREMENTAL: i64 = 10;
    /// Prima pagina dell'inbox, solo metadati: l'utente deve poter lavorare dopo pochi secondi.
    pub const INBOX_FIRST: i64 = 20;
    /// Da qui in su: sync storica, con al massimo il 70% del budget.
    pub const BACKGROUND_FROM: i64 = 30;
    /// Corpi degli ultimi 90 giorni (inbox compresa): è la posta che si legge.
    pub const WINDOW_BODIES: i64 = 30;
    /// Il resto dell'inbox, solo metadati: il corpo arriva quando si apre il thread.
    pub const INBOX_REST: i64 = 40;
    /// Il resto della finestra di 12 mesi, solo metadati.
    pub const WINDOW_METADATA: i64 = 50;
    /// L'inbox più vecchia dei 12 mesi, per ultima.
    pub const INBOX_OLD: i64 = 60;
}

#[derive(Debug, Clone)]
pub struct FetchItem {
    pub thread_id: String,
    /// Se c'è, basta questo messaggio (`messages.get`, metà costo); altrimenti il thread intero.
    pub message_id: Option<String>,
    pub priority: i64,
    pub want_body: bool,
    pub attempts: i64,
    /// Se nel frattempo arriva una nuova richiesta per lo stesso thread, `seq` cambia e il thread
    /// resta in coda anche dopo questo download.
    pub seq: i64,
}

/// Un thread da mettere in coda.
#[derive(Debug, Clone)]
pub struct ToFetch {
    pub thread_id: String,
    /// Da `threads.list`: se in locale il thread è già così recente, si salta.
    pub listed_history_id: Option<i64>,
    /// Basta questo messaggio. Due richieste su messaggi diversi dello stesso thread diventano
    /// una richiesta per il thread intero.
    pub message_id: Option<String>,
}

impl ToFetch {
    pub fn thread(id: impl Into<String>) -> Self {
        ToFetch { thread_id: id.into(), listed_history_id: None, message_id: None }
    }
}

/// Mette in coda dei thread interi. Con `listed_history_id` salta quelli già aggiornati in locale.
/// Restituisce quanti sono stati messi in coda.
pub fn enqueue_fetch(c: &Connection, items: &[(String, Option<i64>)], priority: i64, want_body: bool) -> Result<usize> {
    let items: Vec<ToFetch> = items
        .iter()
        .map(|(id, h)| ToFetch { thread_id: id.clone(), listed_history_id: *h, message_id: None })
        .collect();
    enqueue(c, &items, priority, want_body)
}

pub fn enqueue(c: &Connection, items: &[ToFetch], priority: i64, want_body: bool) -> Result<usize> {
    let mut check = c.prepare_cached("SELECT history_id, bodies_complete FROM threads WHERE id = ?1")?;
    let mut ins = c.prepare_cached(
        "INSERT INTO fetch_queue (thread_id, message_id, priority, want_body, listed_history_id, enqueued_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (thread_id) DO UPDATE SET priority = min(priority, excluded.priority),
            want_body = max(want_body, excluded.want_body),
            message_id = CASE WHEN message_id = excluded.message_id THEN message_id ELSE NULL END,
            listed_history_id = coalesce(excluded.listed_history_id, listed_history_id),
            not_before = CASE WHEN excluded.priority < priority THEN 0 ELSE not_before END,
            seq = seq + 1",
    )?;
    let now = now_ms();
    let mut n = 0;
    for it in items {
        if let Some(listed) = it.listed_history_id {
            let local: Option<(i64, bool)> = check.query_row([&it.thread_id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            if let Some((h, complete)) = local
                && h >= listed
                && (complete || !want_body)
            {
                continue;
            }
        }
        ins.execute(params![it.thread_id, it.message_id, priority, want_body, it.listed_history_id, now])?;
        n += 1;
    }
    Ok(n)
}


pub fn next_fetch_batch(c: &Connection, min_prio: i64, max_prio: i64, limit: usize, claimed: &HashSet<String>) -> Result<Vec<FetchItem>> {
    let mut stmt = c.prepare_cached(
        "SELECT thread_id, priority, want_body, attempts, seq, message_id FROM fetch_queue
         WHERE priority >= ?1 AND priority <= ?2 AND not_before <= ?3
         ORDER BY priority, enqueued_at LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![min_prio, max_prio, now_ms(), (limit + claimed.len()) as i64], |r| {
        Ok(FetchItem {
            thread_id: r.get(0)?,
            priority: r.get(1)?,
            want_body: r.get(2)?,
            attempts: r.get(3)?,
            seq: r.get(4)?,
            message_id: r.get(5)?,
        })
    })?;
    let mut out = Vec::with_capacity(limit);
    for row in rows {
        let item = row?;
        if !claimed.contains(&item.thread_id) {
            out.push(item);
            if out.len() == limit {
                break;
            }
        }
    }
    Ok(out)
}

pub fn dequeue_fetch(c: &Connection, item: &FetchItem) -> Result<()> {
    c.execute("DELETE FROM fetch_queue WHERE thread_id = ?1 AND seq = ?2", params![item.thread_id, item.seq])?;
    Ok(())
}

/// Rimanda un thread in coda. `failed` conta il tentativo: un rifiuto per quota non è colpa del
/// thread e non lo avvicina alla rinuncia.
pub fn defer_fetch(c: &Connection, item: &FetchItem, not_before: i64, failed: bool) -> Result<()> {
    c.execute(
        "UPDATE fetch_queue SET attempts = attempts + ?4, not_before = ?3 WHERE thread_id = ?1 AND seq = ?2",
        params![item.thread_id, item.seq, not_before, failed as i64],
    )?;
    Ok(())
}

pub fn earliest_deferred_fetch(c: &Connection) -> Result<Option<i64>> {
    Ok(c.query_row("SELECT min(not_before) FROM fetch_queue WHERE not_before > ?1", [now_ms()], |r| r.get(0))?)
}

// ───────────────────────────── letture per l'interfaccia ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum View {
    Inbox,
    /// Scheda Importanti dell'inbox (sez. 8 della spec).
    Important,
    /// Scheda Altro dell'inbox.
    Other,
    /// Ricerca a testo pieno (FTS5, per prefisso). Gli operatori di Gmail arrivano con M3.
    Search { q: String },
    /// Fatti: thread fuori dall'inbox con almeno un messaggio ricevuto (niente solo inviati o bozze),
    /// esclusi cestino e spam.
    Done,
    Label { id: String },
    /// Sezione mostrata come scheda accanto a Importanti e Altro: i thread in inbox con l'etichetta.
    Section { id: String },
    All,
}

/// I thread di una sezione mostrata come scheda stanno solo lì, non in Importanti o Altro.
const IN_TAB: &str = "NOT EXISTS (SELECT 1 FROM thread_labels x JOIN sections s ON s.label_id = x.label_id
    WHERE x.thread_id = t.id AND s.show_as_tab = 1 AND s.archived = 0)";

/// Condizione SQL di una vista sulla tabella `threads t` (il parametro dell'etichetta è ?3).
fn view_filter(view: &View) -> (String, Option<String>) {
    let excl = "NOT EXISTS (SELECT 1 FROM thread_labels x WHERE x.thread_id = t.id AND x.label_id IN ('TRASH', 'SPAM'))";
    match view {
        View::Inbox => ("t.in_inbox = 1".to_owned(), None),
        View::Important => (format!("t.in_inbox = 1 AND coalesce(t.split, 'important') = 'important' AND {IN_TAB}"), None),
        View::Other => (format!("t.in_inbox = 1 AND t.split = 'other' AND {IN_TAB}"), None),
        View::Section { id } => (
            "t.in_inbox = 1 AND EXISTS (SELECT 1 FROM thread_labels l WHERE l.thread_id = t.id AND l.label_id = ?3)".to_owned(),
            Some(id.clone()),
        ),
        View::Done => (
            format!(
                "t.in_inbox = 0 AND {excl} AND EXISTS (SELECT 1 FROM messages m WHERE m.thread_id = t.id
                   AND m.label_ids NOT LIKE '%\"SENT\"%' AND m.label_ids NOT LIKE '%\"DRAFT\"%')"
            ),
            None,
        ),
        View::All => (excl.to_owned(), None),
        View::Search { q } => (
            format!(
                "t.id IN (SELECT m.thread_id FROM messages m WHERE m.rowid IN
                   (SELECT rowid FROM search_index WHERE search_index MATCH ?3)) AND {excl}"
            ),
            Some(fts_query(q)),
        ),
        View::Label { id } => {
            let filter = if id == TRASH || id == SPAM { "1".to_owned() } else { excl.to_owned() };
            (
                format!("EXISTS (SELECT 1 FROM thread_labels l WHERE l.thread_id = t.id AND l.label_id = ?3) AND {filter}"),
                Some(id.clone()),
            )
        }
    }
}

/// Dal testo digitato a una query FTS5: ogni parola per prefisso ("prev" trova "preventivo"),
/// tutte richieste. Niente sintassi FTS5 dall'utente: solo lettere e cifre.
///
/// L'indice spezza gli indirizzi sulla punteggiatura (sara.conti@acme.it diventa sara, conti,
/// acme, it): una parola con punti, chiocciole o trattini diventa una frase di quei pezzi in fila,
/// l'ultimo per prefisso. "sara.conti" trova sara.conti@acme.it (11 novembre; prima i pezzi
/// venivano incollati in "saraconti", che non trovava niente).
pub fn fts_query(q: &str) -> String {
    let words: Vec<String> = q
        .split_whitespace()
        .map(|w| w.split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" "))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect();
    if words.is_empty() { "\"\"".to_owned() } else { words.join(" ") }
}

pub fn count_threads(c: &Connection, view: &View) -> Result<i64> {
    let (filter, label) = view_filter(view);
    let sql = format!("SELECT count(*) FROM threads t WHERE {filter}");
    let sql = sql.replace("?3", "?1");
    Ok(match label {
        Some(l) => c.query_row(&sql, [l], |r| r.get(0))?,
        None => c.query_row(&sql, [], |r| r.get(0))?,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadSummary {
    pub id: String,
    pub subject: String,
    pub snippet: String,
    pub last_date: i64,
    pub participants: Vec<Participant>,
    pub message_count: i64,
    pub unread: bool,
    pub starred: bool,
    pub in_inbox: bool,
    pub important: bool,
    pub has_attachments: bool,
    pub has_draft: bool,
    pub bodies_complete: bool,
    pub labels: Vec<String>,
    /// Posticipato e appena tornato: in cima alla lista con l'indicazione "Promemoria".
    pub reminded: bool,
    /// Nomi degli allegati del thread (al massimo 3, senza doppioni) e quanti sono in tutto.
    pub attachment_names: Vec<String>,
    pub attachment_count: i64,
}

const SUMMARY_COLS: &str = "t.id, t.subject, t.snippet, t.last_date, t.participants, t.message_count, t.unread, t.starred,
    t.in_inbox, t.important, t.has_attachments, t.has_draft, t.bodies_complete,
    (SELECT group_concat(label_id, ' ') FROM thread_labels tl WHERE tl.thread_id = t.id),
    coalesce(t.reminded_at, 0) > t.last_date,
    (SELECT group_concat(a.filename, char(31)) FROM attachments a JOIN messages m ON m.rowid = a.message_rowid
       WHERE m.thread_id = t.id AND a.is_inline = 0 AND a.filename <> '')";

fn summary_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadSummary> {
    let participants: String = r.get(4)?;
    let labels: Option<String> = r.get(13)?;
    Ok(ThreadSummary {
        id: r.get(0)?,
        subject: r.get(1)?,
        snippet: r.get(2)?,
        last_date: r.get(3)?,
        participants: serde_json::from_str(&participants).unwrap_or_default(),
        message_count: r.get(5)?,
        unread: r.get(6)?,
        starred: r.get(7)?,
        in_inbox: r.get(8)?,
        important: r.get(9)?,
        has_attachments: r.get(10)?,
        has_draft: r.get(11)?,
        bodies_complete: r.get(12)?,
        labels: labels.map(|s| s.split(' ').map(str::to_owned).collect()).unwrap_or_default(),
        reminded: r.get(14)?,
        attachment_names: Vec::new(),
        attachment_count: 0,
    })
    .map(|mut t| {
        let names: Option<String> = r.get(15).ok().flatten();
        let mut seen = Vec::<String>::new();
        for n in names.as_deref().unwrap_or("").split('\u{1f}').filter(|n| !n.is_empty()) {
            if !seen.iter().any(|s| s == n) {
                seen.push(n.to_owned());
            }
        }
        t.attachment_count = seen.len() as i64;
        seen.truncate(3);
        t.attachment_names = seen;
        t
    })
}

pub fn list_threads(c: &Connection, view: &View, limit: i64, offset: i64) -> Result<Vec<ThreadSummary>> {
    let (filter, label) = view_filter(view);
    // I posticipati appena tornati stanno in cima: contano dall'ora del ritorno.
    let sql = format!(
        "SELECT {SUMMARY_COLS} FROM threads t WHERE {filter}
         ORDER BY max(t.last_date, coalesce(t.reminded_at, 0)) DESC LIMIT ?1 OFFSET ?2"
    );
    let mut stmt = c.prepare_cached(&sql)?;
    let rows = match label {
        Some(l) => stmt.query_map(params![limit, offset, l], summary_from_row)?.collect::<Result<Vec<_>, _>>()?,
        None => stmt.query_map(params![limit, offset], summary_from_row)?.collect::<Result<Vec<_>, _>>()?,
    };
    Ok(rows)
}

pub fn thread_summary(c: &Connection, id: &str) -> Result<Option<ThreadSummary>> {
    Ok(c.query_row(&format!("SELECT {SUMMARY_COLS} FROM threads t WHERE t.id = ?1"), [id], summary_from_row).optional()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct AttachmentView {
    pub part_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub content_id: Option<String>,
    pub is_inline: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageView {
    pub id: String,
    pub date: i64,
    pub from: Option<Addr>,
    pub to: Vec<Addr>,
    pub cc: Vec<Addr>,
    pub subject: String,
    pub snippet: String,
    pub label_ids: Vec<String>,
    pub has_body: bool,
    pub text_plain: Option<String>,
    pub html: Option<String>,
    pub attachments: Vec<AttachmentView>,
    /// Mittente conosciuto (gli ho scritto, o è del mio dominio): le immagini remote si caricano
    /// da sole (sez. 6 della spec). Per gli altri restano bloccate.
    pub trusted_sender: bool,
    /// Testo senza citazioni né firma, per la vista a conversazione.
    pub text_clean: Option<String>,
    /// Invio di massa o lista (newsletter, notifiche): si mostra l'email originale.
    pub bulk: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadDetail {
    pub thread: ThreadSummary,
    pub messages: Vec<MessageView>,
}

pub fn get_thread(c: &Connection, id: &str) -> Result<Option<ThreadDetail>> {
    let Some(thread) = thread_summary(c, id)? else { return Ok(None) };
    let mut stmt = c.prepare_cached(
        "SELECT m.rowid, m.id, m.internal_date, m.from_email, m.from_name, m.to_json, m.cc_json, m.subject, m.snippet,
                m.label_ids, m.has_body, b.text_plain, b.html_clean, b.text_unquoted,
                m.list_unsubscribe IS NOT NULL OR m.list_id IS NOT NULL OR lower(coalesce(m.precedence, '')) IN ('bulk', 'list', 'junk')
         FROM messages m LEFT JOIN message_bodies b ON b.message_rowid = m.rowid
         WHERE m.thread_id = ?1 ORDER BY m.internal_date, m.rowid",
    )?;
    let mut att = c.prepare_cached(
        "SELECT part_id, filename, mime_type, size, content_id, is_inline FROM attachments WHERE message_rowid = ?1 ORDER BY part_id",
    )?;
    let rows: Vec<(i64, MessageView)> = stmt
        .query_map([id], |r| {
            let from_email: Option<String> = r.get(3)?;
            let to: String = r.get(5)?;
            let cc: String = r.get(6)?;
            let labels: String = r.get(9)?;
            Ok((
                r.get(0)?,
                MessageView {
                    id: r.get(1)?,
                    date: r.get(2)?,
                    from: from_email.map(|email| Addr { email, name: r.get(4).ok().flatten() }),
                    to: serde_json::from_str(&to).unwrap_or_default(),
                    cc: serde_json::from_str(&cc).unwrap_or_default(),
                    subject: r.get(7)?,
                    snippet: r.get(8)?,
                    label_ids: serde_json::from_str(&labels).unwrap_or_default(),
                    has_body: r.get(10)?,
                    text_plain: r.get(11)?,
                    html: r.get(12)?,
                    attachments: Vec::new(),
                    trusted_sender: false,
                    text_clean: r.get(13)?,
                    bulk: r.get(14)?,
                },
            ))
        })?
        .collect::<Result<_, _>>()?;
    let my_domains = my_domains(c)?;
    let mut trusted = c.prepare_cached("SELECT sent_count > 0 FROM contacts WHERE email = ?1")?;
    let mut messages = Vec::with_capacity(rows.len());
    for (rowid, mut m) in rows {
        if let Some(from) = &m.from {
            let same_domain = is_my_domain(&my_domains, &from.email);
            let wrote_to: bool = trusted.query_row([&from.email], |r| r.get(0)).optional()?.unwrap_or(false);
            m.trusted_sender = same_domain || wrote_to;
        }
        m.attachments = att
            .query_map([rowid], |r| {
                Ok(AttachmentView {
                    part_id: r.get(0)?,
                    filename: r.get(1)?,
                    mime_type: r.get(2)?,
                    size: r.get(3)?,
                    content_id: r.get(4)?,
                    is_inline: r.get(5)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        messages.push(m);
    }
    Ok(Some(ThreadDetail { thread, messages }))
}

/// Thread segnati come fatti oggi (da mezzanotte locale), al netto di quelli rimessi in inbox.
pub fn done_today(c: &Connection, since_ms: i64) -> Result<i64> {
    Ok(c.query_row(
        "SELECT coalesce(sum(CASE WHEN payload LIKE '%\"remove\":[%\"INBOX\"%' THEN 1 ELSE -1 END), 0)
         FROM outbox_ops WHERE kind = 'modify' AND created_at >= ?1 AND state <> 'failed'
           AND (payload LIKE '%\"remove\":[%\"INBOX\"%' OR payload LIKE '%\"add\":[%\"INBOX\"%')",
        [since_ms],
        |r| r.get::<_, i64>(0),
    )?
    .max(0))
}

// ───────────── I miei domini ─────────────
//
// I domini dei colleghi: i loro messaggi vanno sempre in Importanti e le loro immagini si caricano.
// Di base è il dominio dell'account, tranne i provider personali (con gmail.com tutti gli utenti
// Gmail sarebbero colleghi); nelle impostazioni se ne possono mettere altri o nessuno.

const MY_DOMAINS_SETTING: &str = "my_domains";

#[derive(Debug, Clone, Serialize)]
pub struct MyDomains {
    pub domains: Vec<String>,
    /// Scelti nelle impostazioni (altrimenti ricavati dall'account).
    pub custom: bool,
    /// Il dominio dell'account, se non è un provider personale.
    pub suggested: Option<String>,
}

pub fn my_domains_view(c: &Connection) -> Result<MyDomains> {
    let email: Option<String> = c.prepare_cached("SELECT email FROM account WHERE id = 1")?.query_row([], |r| r.get(0)).optional()?;
    let suggested = email
        .and_then(|e| e.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase()))
        .filter(|d| !crate::people::PERSONAL_DOMAINS.contains(&d.as_str()));
    let saved: Option<String> =
        c.prepare_cached("SELECT value FROM settings WHERE key = ?1")?.query_row([MY_DOMAINS_SETTING], |r| r.get(0)).optional()?;
    match saved.and_then(|v| serde_json::from_str::<Vec<String>>(&v).ok()) {
        Some(domains) => Ok(MyDomains { domains, custom: true, suggested }),
        None => Ok(MyDomains { domains: suggested.clone().into_iter().collect(), custom: false, suggested }),
    }
}

pub fn my_domains(c: &Connection) -> Result<Vec<String>> {
    Ok(my_domains_view(c)?.domains)
}

pub fn is_my_domain(domains: &[String], email: &str) -> bool {
    email.rsplit_once('@').is_some_and(|(_, d)| domains.iter().any(|x| d.eq_ignore_ascii_case(x)))
}

/// "@Esempio.it " → "esempio.it". `None` se non è un dominio.
pub fn normalize_domain(raw: &str) -> Option<String> {
    let d = raw.trim().trim_start_matches('@').trim().to_ascii_lowercase();
    let ok = d.len() <= 253
        && d.contains('.')
        && d.split('.').all(|part| {
            !part.is_empty() && !part.starts_with('-') && !part.ends_with('-') && part.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        });
    ok.then_some(d)
}

/// Salva i domini e ridivide l'inbox per i mittenti dei domini entrati o usciti.
pub fn set_my_domains(c: &mut Connection, raw: &[String]) -> Result<MyDomains> {
    let mut list: Vec<String> = Vec::new();
    for d in raw.iter().filter(|d| !d.trim().is_empty()) {
        let n = normalize_domain(d).ok_or_else(|| crate::Error::other(format!("«{}» non è un dominio (es. azienda.it)", d.trim())))?;
        if !list.contains(&n) {
            list.push(n);
        }
    }
    if list.len() > 20 {
        return Err(crate::Error::other("al massimo 20 domini"));
    }
    let before = my_domains(c)?;
    let tx = c.transaction()?;
    set_setting(&tx, MY_DOMAINS_SETTING, &serde_json::json!(list))?;
    let mut n = 0;
    for d in before.iter().filter(|d| !list.contains(d)).chain(list.iter().filter(|d| !before.contains(d))) {
        n += crate::split::recompute_domain(&tx, d)?;
    }
    tx.commit()?;
    tracing::info!(domini = list.len(), thread = n, "i miei domini aggiornati");
    my_domains_view(c)
}

/// Impostazioni dell'interfaccia e dell'account, in JSON.
pub fn get_setting(c: &Connection, key: &str) -> Result<Option<serde_json::Value>> {
    let v: Option<String> = c.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?;
    Ok(v.and_then(|v| serde_json::from_str(&v).ok()))
}

pub fn set_setting(c: &Connection, key: &str, value: &serde_json::Value) -> Result<()> {
    c.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value.to_string()],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncStatus {
    /// initial | syncing | idle | offline | auth_required | error
    pub phase: String,
    pub initial_sync_state: String,
    pub queued: i64,
    pub queued_background: i64,
    pub pending_actions: i64,
    pub threads_local: i64,
    pub inbox_local: i64,
    pub last_sync_at: Option<i64>,
    pub last_error: Option<String>,
}

pub fn sync_counts(c: &Connection) -> Result<SyncStatus> {
    let acc = account(c)?;
    let (queued, queued_bg): (i64, i64) = c.query_row(
        "SELECT coalesce(sum(priority < ?1), 0), coalesce(sum(priority >= ?1), 0) FROM fetch_queue",
        [prio::BACKGROUND_FROM],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let pending: i64 = c.query_row("SELECT count(*) FROM outbox_ops WHERE state IN ('pending', 'in_flight')", [], |r| r.get(0))?;
    let (threads, inbox): (i64, i64) =
        c.query_row("SELECT count(*), coalesce(sum(in_inbox), 0) FROM threads", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(SyncStatus {
        phase: String::new(),
        initial_sync_state: acc.as_ref().map(|a| a.initial_sync_state.clone()).unwrap_or_else(|| "pending".into()),
        queued,
        queued_background: queued_bg,
        pending_actions: pending,
        threads_local: threads,
        inbox_local: inbox,
        last_sync_at: acc.as_ref().and_then(|a| a.last_sync_at),
        last_error: acc.and_then(|a| a.last_sync_error),
    })
}

#[cfg(test)]
pub(crate) mod testdata {
    use crate::gmail::{Message, MessagePart, Thread};
    use crate::util::b64url;

    pub fn msg(id: &str, thread: &str, date: i64, from: &str, labels: &[&str], body: Option<&str>) -> Message {
        let mut payload = serde_json::json!({
            "mimeType": "text/plain",
            "headers": [
                {"name": "From", "value": from},
                {"name": "To", "value": "Tommaso <tommaso@esempio.it>"},
                {"name": "Subject", "value": format!("Oggetto {thread}")},
                {"name": "Message-ID", "value": format!("<{id}@x>")}
            ],
            "body": {"size": 0}
        });
        if let Some(b) = body {
            payload["body"] = serde_json::json!({"size": b.len(), "data": b64url(b.as_bytes())});
        }
        Message {
            id: id.into(),
            thread_id: thread.into(),
            label_ids: labels.iter().map(|s| s.to_string()).collect(),
            snippet: body.unwrap_or("").chars().take(50).collect(),
            history_id: Some(date.to_string()),
            internal_date: Some(date.to_string()),
            payload: Some(serde_json::from_value::<MessagePart>(payload).unwrap()),
            size_estimate: Some(100),
        }
    }

    pub fn thread(id: &str, history: i64, messages: Vec<Message>) -> Thread {
        Thread { id: id.into(), history_id: Some(history.to_string()), messages }
    }
}

#[cfg(test)]
mod tests {
    use super::testdata::*;
    use super::*;
    use crate::db::test_db;

    const ME: &str = "tommaso@esempio.it";

    #[test]
    fn writes_thread_with_flags_index_and_contacts() {
        let (_d, db) = test_db();
        db.write(|c| {
            ensure_account(c, ME, Some("Tommaso"), None)?;
            let t = thread(
                "t1",
                100,
                vec![
                    msg("m1", "t1", 1000, "Anna Bianchi <anna@cliente.it>", &["INBOX", "UNREAD"], Some("Mi mandi il preventivo?")),
                    msg("m2", "t1", 2000, "Tommaso <tommaso@esempio.it>", &["SENT"], Some("Eccolo")),
                ],
            );
            let tx = c.transaction()?;
            assert_eq!(write_thread(&tx, &t, true, ME)?, WriteOutcome::Written);
            tx.commit()?;

            let s = thread_summary(c, "t1")?.unwrap();
            assert!(s.unread && s.in_inbox && !s.starred && s.bodies_complete);
            assert_eq!(s.message_count, 2);
            assert_eq!(s.last_date, 2000);
            assert_eq!(s.snippet, "Eccolo");
            assert_eq!(s.participants.len(), 2);
            assert!(s.labels.contains(&"SENT".to_string()));

            let hits: i64 = c.query_row("SELECT count(*) FROM search_index WHERE search_index MATCH 'preven*'", [], |r| r.get(0))?;
            assert_eq!(hits, 1);
            let (recv,): (i64,) = c.query_row("SELECT received_count FROM contacts WHERE email = 'anna@cliente.it'", [], |r| Ok((r.get(0)?,)))?;
            assert_eq!(recv, 1);

            // Riscrivere lo stesso thread non conta due volte i contatti né duplica l'indice.
            let tx = c.transaction()?;
            write_thread(&tx, &t, true, ME)?;
            tx.commit()?;
            let (recv,): (i64,) = c.query_row("SELECT received_count FROM contacts WHERE email = 'anna@cliente.it'", [], |r| Ok((r.get(0)?,)))?;
            assert_eq!(recv, 1);
            let rows: i64 = c.query_row("SELECT count(*) FROM search_index WHERE search_index MATCH 'preventivo'", [], |r| r.get(0))?;
            assert_eq!(rows, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn metadata_refetch_keeps_bodies_and_removed_messages_disappear() {
        let (_d, db) = test_db();
        db.write(|c| {
            ensure_account(c, ME, None, None)?;
            let full = thread("t1", 10, vec![msg("m1", "t1", 1, "a@x.it", &["INBOX"], Some("corpo")), msg("m2", "t1", 2, "b@x.it", &["INBOX"], Some("due"))]);
            write_thread(c, &full, true, ME)?;
            let meta = thread("t1", 11, vec![msg("m1", "t1", 1, "a@x.it", &[], None)]);
            write_thread(c, &meta, false, ME)?;
            let d = get_thread(c, "t1")?.unwrap();
            assert_eq!(d.messages.len(), 1);
            assert_eq!(d.messages[0].text_plain.as_deref(), Some("corpo"));
            assert!(!d.thread.in_inbox);
            let idx: i64 = c.query_row("SELECT count(*) FROM search_index WHERE search_index MATCH 'due'", [], |r| r.get(0))?;
            assert_eq!(idx, 0);
            write_thread(c, &thread("t1", 12, vec![]), true, ME)?;
            assert!(thread_summary(c, "t1")?.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn pending_ops_win_over_server_state() {
        let (_d, db) = test_db();
        db.write(|c| {
            ensure_account(c, ME, None, None)?;
            let t = thread("t1", 10, vec![msg("m1", "t1", 1, "a@x.it", &["INBOX", "UNREAD"], Some("x"))]);
            write_thread(c, &t, true, ME)?;
            c.execute(
                "INSERT INTO outbox_ops (kind, thread_id, payload, next_attempt_at, created_at) VALUES ('modify', 't1', '{\"remove\":[\"INBOX\"]}', 0, 0)",
                [],
            )?;
            // La sync riporta lo stato del server (ancora in inbox): l'archiviazione in coda prevale.
            write_thread(c, &t, true, ME)?;
            assert!(!thread_summary(c, "t1")?.unwrap().in_inbox);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn search_finds_by_prefix_and_ignores_fts_syntax() {
        let (_d, db) = test_db();
        db.write(|c| {
            ensure_account(c, ME, None, None)?;
            write_thread(c, &thread("t1", 1, vec![msg("m1", "t1", 1, "Anna <anna@x.it>", &["INBOX"], Some("Il preventivo per giovedì"))]), true, ME)?;
            write_thread(c, &thread("t2", 2, vec![msg("m2", "t2", 2, "Luca <luca@x.it>", &["INBOX"], Some("Riunione di lunedì"))]), true, ME)?;
            let ids = |q: &str| -> Result<Vec<String>> {
                Ok(list_threads(c, &View::Search { q: q.into() }, 10, 0)?.into_iter().map(|t| t.id).collect())
            };
            assert_eq!(ids("prev")?, ["t1"]);
            assert_eq!(ids("anna giov")?, ["t1"], "tutte le parole, per prefisso, anche nel mittente");
            assert_eq!(ids("riunione")?, ["t2"]);
            assert!(ids("\"*) OR (")?.is_empty(), "la sintassi FTS5 non passa");
            assert_eq!(count_threads(c, &View::Search { q: "lunedi".into() })?, 1, "senza accenti trova con gli accenti");
            // Indirizzi: nel mittente e nei destinatari, interi o in parte.
            write_thread(c, &thread("t3", 3, vec![msg("m3", "t3", 3, "Sara Conti <sara.conti@acme.it>", &["INBOX"], Some("Ciao"))]), true, ME)?;
            let mut sent = msg("m4", "t4", 4, "Tommaso <tommaso@esempio.it>", &["SENT"], Some("Proposta"));
            if let Some(p) = sent.payload.as_mut() {
                for h in p.headers.iter_mut() {
                    if h.name == "To" {
                        h.value = "Marta Bianchi <marta.bianchi@studio-legale.it>".into();
                    }
                }
            }
            write_thread(c, &thread("t4", 4, vec![sent]), true, ME)?;
            assert_eq!(ids("sara.conti")?, ["t3"]);
            assert_eq!(ids("sara.conti@acme.it")?, ["t3"]);
            assert_eq!(ids("sara.co")?, ["t3"], "l'ultimo pezzo per prefisso");
            assert_eq!(ids("marta.bianchi")?, ["t4"], "anche tra i destinatari di un messaggio inviato");
            assert_eq!(ids("studio-legale")?, ["t4"]);
            assert_eq!(fts_query("sara.conti"), "\"sara conti\"*");
            assert_eq!(fts_query("--"), "\"\"");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn enqueue_skips_threads_already_up_to_date() {
        let (_d, db) = test_db();
        db.write(|c| {
            ensure_account(c, ME, None, None)?;
            write_thread(c, &thread("t1", 50, vec![msg("m1", "t1", 1, "a@x.it", &["INBOX"], Some("x"))]), true, ME)?;
            let n = enqueue_fetch(c, &[("t1".into(), Some(50)), ("t2".into(), Some(7))], prio::INBOX_REST, true)?;
            assert_eq!(n, 1);
            let n = enqueue_fetch(c, &[("t1".into(), Some(51))], prio::INBOX_REST, true)?;
            assert_eq!(n, 1);
            // Priorità: vince la più urgente, e il corpo resta richiesto.
            enqueue_fetch(c, &[("t2".into(), None)], prio::WINDOW_METADATA, false)?;
            enqueue_fetch(c, &[("t2".into(), None)], prio::INCREMENTAL, false)?;
            let batch = next_fetch_batch(c, 0, 100, 10, &HashSet::new())?;
            assert_eq!(batch[0].thread_id, "t2");
            assert_eq!(batch[0].priority, prio::INCREMENTAL);
            assert!(batch[0].want_body);
            Ok(())
        })
        .unwrap();
    }

    fn one(thread: &str, message: Option<&str>) -> ToFetch {
        ToFetch { thread_id: thread.into(), listed_history_id: None, message_id: message.map(Into::into) }
    }

    #[test]
    fn two_messages_of_the_same_thread_become_the_whole_thread() {
        let (_d, db) = test_db();
        db.write(|c| {
            enqueue(c, &[one("t1", Some("t1"))], prio::WINDOW_BODIES, true)?;
            enqueue(c, &[one("t1", Some("t1"))], prio::INBOX_REST, false)?;
            let b = next_fetch_batch(c, 0, 100, 10, &HashSet::new())?;
            assert_eq!(b[0].message_id.as_deref(), Some("t1"), "stesso messaggio: resta singolo");

            enqueue(c, &[one("t1", Some("t1x"))], prio::INCREMENTAL, true)?;
            let b = next_fetch_batch(c, 0, 100, 10, &HashSet::new())?;
            assert_eq!(b[0].message_id, None, "messaggi diversi: thread intero");

            enqueue(c, &[one("t2", None)], prio::USER, true)?;
            enqueue(c, &[one("t2", Some("t2"))], prio::USER, true)?;
            let b = next_fetch_batch(c, 0, 100, 10, &HashSet::new())?;
            assert_eq!((b[0].thread_id.as_str(), b[0].message_id.as_deref()), ("t2", None), "il thread intero resta intero");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn history_labels_apply_locally_and_stale_fetches_do_not_undo_them() {
        let (_d, db) = test_db();
        db.write(|c| {
            ensure_account(c, ME, None, None)?;
            let mut m = msg("m1", "t1", 1, "a@x.it", &["INBOX", "UNREAD"], Some("x"));
            m.history_id = Some("10".into());
            write_messages(c, "t1", &[m.clone()], true, ME)?;
            assert!(thread_summary(c, "t1")?.unwrap().in_inbox);

            // Archiviato e letto su Gmail web (record 12).
            assert_eq!(apply_history_labels(c, "m1", &[], 12)?, Some(("t1".to_owned(), true)));
            // Lo stesso stato ripetuto non ricalcola nulla.
            assert_eq!(apply_history_labels(c, "m1", &[], 13)?, Some(("t1".to_owned(), false)));
            let t = thread_summary(c, "t1")?.unwrap();
            assert!(!t.in_inbox && !t.unread);
            assert_eq!(thread_history_id(c, "t1")?, Some(13));

            // Una risposta di Gmail partita prima del record 12 non riporta il thread in inbox.
            write_messages(c, "t1", &[m], true, ME)?;
            assert!(!thread_summary(c, "t1")?.unwrap().in_inbox);

            // Un messaggio che non abbiamo: niente da fare in locale.
            assert_eq!(apply_history_labels(c, "boh", &["INBOX".into()], 13)?, None);

            // Cancellato l'unico messaggio: sparisce il thread.
            assert_eq!(delete_message(c, "m1")?.as_deref(), Some("t1"));
            assert!(thread_summary(c, "t1")?.is_none());
            Ok(())
        })
        .unwrap();
    }
}
