//! Sgombero della posta vecchia (specifica UX, sezione 1): "segna come fatti i thread più vecchi
//! di…". In locale vale subito, in una transazione; su Gmail va in background: si elencano i
//! messaggi con la stessa query (`messages.list`, 5 unità ogni 500) e si tolgono dall'inbox con
//! `messages.batchModify` (50 unità ogni 1.000). L'operazione riprende dopo un riavvio e si annulla
//! come un'azione sola.

use std::sync::Arc;
use std::time::Duration;

use chrono::TimeZone;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::events::CoreEvent;
use crate::gmail::{GmailApi, ListThreads, Priority};
use crate::store::{INBOX, prio};
use crate::sync::Ctx;
use crate::util::{backoff_ms, now_ms};
use crate::{Error, Result};

/// Messaggi per `messages.batchModify` (il massimo di Gmail).
const BATCH: usize = 1_000;

#[derive(Debug, Clone, Serialize)]
pub struct BulkPreview {
    pub cutoff: i64,
    pub query: String,
    /// Thread in inbox, in locale, con l'ultimo messaggio prima di `cutoff`.
    pub local: i64,
    /// Stima di Gmail, che conta anche quelli non ancora scaricati.
    pub estimate: Option<i64>,
}

/// La query Gmail per i thread in inbox più vecchi di `cutoff` (data locale, a giorni interi).
pub fn query_for(cutoff: i64) -> String {
    let day = chrono::Local
        .timestamp_millis_opt(cutoff)
        .single()
        .map(|d| d.format("%Y/%m/%d").to_string())
        .unwrap_or_else(|| "1970/01/01".into());
    format!("in:inbox before:{day}")
}

fn count_local(c: &Connection, cutoff: i64) -> Result<i64> {
    Ok(c.query_row("SELECT count(*) FROM threads WHERE in_inbox = 1 AND last_date < ?1", [cutoff], |r| r.get(0))?)
}

pub async fn preview<G: GmailApi>(ctx: &Ctx<G>, cutoff: i64) -> Result<BulkPreview> {
    let query = query_for(cutoff);
    let local = ctx.db.read_async(move |c| count_local(c, cutoff)).await?;
    let estimate = ctx
        .gmail
        .list_threads(ListThreads { q: Some(query.clone()), max_results: 1, ..Default::default() }, Priority::Interactive)
        .await
        .ok()
        .and_then(|p| p.result_size_estimate);
    Ok(BulkPreview { cutoff, query, local, estimate })
}

/// Toglie `INBOX` da tutti i messaggi dei thread elencati, direttamente in SQL: con decine di
/// migliaia di thread ricalcolarli uno per uno costerebbe secondi.
fn remove_inbox_locally(c: &Connection, op_id: i64) -> Result<()> {
    c.execute(
        "UPDATE messages SET label_ids = (SELECT json_group_array(value) FROM json_each(messages.label_ids) WHERE value <> 'INBOX')
         WHERE thread_id IN (SELECT thread_id FROM bulk_op_threads WHERE op_id = ?1) AND label_ids LIKE '%\"INBOX\"%'",
        [op_id],
    )?;
    c.execute(
        "DELETE FROM thread_labels WHERE label_id = 'INBOX' AND thread_id IN (SELECT thread_id FROM bulk_op_threads WHERE op_id = ?1)",
        [op_id],
    )?;
    c.execute("UPDATE threads SET in_inbox = 0 WHERE id IN (SELECT thread_id FROM bulk_op_threads WHERE op_id = ?1)", [op_id])?;
    Ok(())
}

/// Rimette `INBOX` sui messaggi locali dell'operazione `op_id`.
fn restore_inbox_locally(c: &Connection, op_id: i64) -> Result<()> {
    c.execute(
        "UPDATE messages SET label_ids = json_insert(label_ids, '$[#]', 'INBOX')
         WHERE id IN (SELECT message_id FROM bulk_op_messages WHERE op_id = ?1) AND label_ids NOT LIKE '%\"INBOX\"%'",
        [op_id],
    )?;
    let threads = "SELECT DISTINCT m.thread_id FROM messages m JOIN bulk_op_messages b ON b.message_id = m.id WHERE b.op_id = ?1";
    c.execute(&format!("INSERT OR IGNORE INTO thread_labels (thread_id, label_id) SELECT thread_id, 'INBOX' FROM ({threads})"), [op_id])?;
    c.execute(
        &format!(
            "UPDATE threads SET in_inbox = 1 WHERE id IN ({threads})
             AND NOT EXISTS (SELECT 1 FROM thread_labels x WHERE x.thread_id = threads.id AND x.label_id IN ('TRASH', 'SPAM'))"
        ),
        [op_id],
    )?;
    Ok(())
}

/// Avvia uno sgombero: in locale subito, su Gmail con il worker. Restituisce l'ID dell'operazione
/// e quanti thread locali sono usciti dall'inbox.
pub fn start_done(c: &mut Connection, cutoff: i64, window_start: i64) -> Result<(i64, i64)> {
    let tx = c.transaction()?;
    tx.execute(
        "INSERT INTO bulk_ops (kind, query, cutoff, state, created_at) VALUES ('done', ?1, ?2, 'listing', ?3)",
        params![query_for(cutoff), cutoff, now_ms()],
    )?;
    let op = tx.last_insert_rowid();
    let n = tx.execute(
        "INSERT INTO bulk_op_threads (op_id, thread_id) SELECT ?1, id FROM threads WHERE in_inbox = 1 AND last_date < ?2",
        params![op, cutoff],
    )? as i64;
    // I messaggi locali entrano subito: l'annulla li ritrova anche se l'elenco su Gmail non è finito.
    tx.execute(
        "INSERT OR IGNORE INTO bulk_op_messages (op_id, message_id)
         SELECT ?1, m.id FROM messages m JOIN bulk_op_threads t ON t.thread_id = m.thread_id
         WHERE t.op_id = ?1 AND m.label_ids LIKE '%\"INBOX\"%'",
        [op],
    )?;
    remove_inbox_locally(&tx, op)?;
    tx.execute("UPDATE bulk_ops SET threads_local = ?2 WHERE id = ?1", params![op, n])?;
    // L'inbox più vecchia della finestra non serve più: è tutta prima della data scelta.
    if cutoff >= window_start {
        tx.execute("DELETE FROM fetch_queue WHERE priority = ?1", [prio::INBOX_OLD])?;
    }
    tx.commit()?;
    Ok((op, n))
}

/// Annulla uno sgombero: in locale subito, su Gmail rimette `INBOX` sui messaggi già modificati.
pub fn start_undo(c: &mut Connection, op_id: i64) -> Result<i64> {
    let tx = c.transaction()?;
    let orig: Option<(String, String, String, i64)> = tx
        .query_row("SELECT kind, state, query, cutoff FROM bulk_ops WHERE id = ?1", [op_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()?;
    let Some((kind, state, query, cutoff)) = orig else { return Err(Error::NotFound) };
    if kind != "done" || matches!(state.as_str(), "undone" | "cancelled") {
        return Err(Error::other("operazione già annullata"));
    }
    // Se l'elenco o l'invio su Gmail non sono finiti, il worker si ferma qui.
    let new_state = if matches!(state.as_str(), "listing" | "applying") { "cancelled" } else { "undone" };
    tx.execute("UPDATE bulk_ops SET state = ?2, completed_at = ?3 WHERE id = ?1", params![op_id, new_state, now_ms()])?;
    restore_inbox_locally(&tx, op_id)?;
    tx.execute(
        "INSERT INTO bulk_ops (kind, query, cutoff, ref_op, state, created_at) VALUES ('undo', ?1, ?2, ?3, 'applying', ?4)",
        params![query, cutoff, op_id, now_ms()],
    )?;
    let undo = tx.last_insert_rowid();
    let n = tx.execute(
        "INSERT INTO bulk_op_messages (op_id, message_id) SELECT ?1, message_id FROM bulk_op_messages WHERE op_id = ?2 AND applied = 1",
        params![undo, op_id],
    )? as i64;
    tx.execute("UPDATE bulk_ops SET messages_total = ?2 WHERE id = ?1", params![undo, n])?;
    tx.commit()?;
    Ok(undo)
}

struct Pending {
    id: i64,
    kind: String,
    query: String,
    state: String,
}

fn next_pending(c: &Connection) -> Result<Option<Pending>> {
    Ok(c
        .query_row(
            "SELECT id, kind, query, state FROM bulk_ops WHERE state IN ('listing', 'applying') ORDER BY id LIMIT 1",
            [],
            |r| Ok(Pending { id: r.get(0)?, kind: r.get(1)?, query: r.get(2)?, state: r.get(3)? }),
        )
        .optional()?)
}

fn state_of(c: &Connection, id: i64) -> Result<String> {
    Ok(c.query_row("SELECT state FROM bulk_ops WHERE id = ?1", [id], |r| r.get(0))?)
}

async fn emit_status<G: GmailApi>(ctx: &Ctx<G>, id: i64) {
    let row = ctx
        .db
        .read_async(move |c| {
            Ok(c.query_row(
                "SELECT kind, state, messages_done, messages_total FROM bulk_ops WHERE id = ?1",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
            )?)
        })
        .await;
    if let Ok((kind, state, done, total)) = row {
        ctx.emit(CoreEvent::BulkStatus { op_id: id, kind, state, done, total });
    }
}

/// Un passo di lavoro su un'operazione. `Ok(true)` se c'è ancora da fare.
async fn step<G: GmailApi>(ctx: &Ctx<G>, op: &Pending) -> Result<bool> {
    let id = op.id;
    if op.state == "listing" {
        let mut req = ListThreads { q: Some(op.query.clone()), max_results: 500, ..Default::default() };
        loop {
            let page = ctx.gmail.list_messages(req.clone(), Priority::Interactive).await?;
            let ids: Vec<String> = page.messages.into_iter().map(|m| m.id).collect();
            let stop = ctx
                .db
                .write_async(move |c| {
                    if state_of(c, id)? != "listing" {
                        return Ok(true);
                    }
                    let tx = c.transaction()?;
                    for m in &ids {
                        tx.execute("INSERT OR IGNORE INTO bulk_op_messages (op_id, message_id) VALUES (?1, ?2)", params![id, m])?;
                    }
                    tx.execute(
                        "UPDATE bulk_ops SET messages_total = (SELECT count(*) FROM bulk_op_messages WHERE op_id = ?1) WHERE id = ?1",
                        [id],
                    )?;
                    tx.commit()?;
                    Ok(false)
                })
                .await?;
            if stop {
                return Ok(false);
            }
            match page.next_page_token {
                Some(t) => req.page_token = Some(t),
                None => break,
            }
        }
        ctx.db
            .write_async(move |c| {
                c.execute("UPDATE bulk_ops SET state = 'applying' WHERE id = ?1 AND state = 'listing'", [id])?;
                Ok(())
            })
            .await?;
        tracing::info!(op = id, "sgombero: messaggi elencati");
        emit_status(ctx, id).await;
        return Ok(true);
    }

    // applying
    let batch: Vec<String> = ctx
        .db
        .read_async(move |c| {
            Ok(c.prepare("SELECT message_id FROM bulk_op_messages WHERE op_id = ?1 AND applied = 0 LIMIT ?2")?
                .query_map(params![id, BATCH as i64], |r| r.get(0))?
                .collect::<Result<Vec<String>, _>>()?)
        })
        .await?;
    if batch.is_empty() {
        ctx.db
            .write_async(move |c| {
                c.execute("UPDATE bulk_ops SET state = 'done', completed_at = ?2 WHERE id = ?1 AND state = 'applying'", params![id, now_ms()])?;
                Ok(())
            })
            .await?;
        tracing::info!(op = id, kind = %op.kind, "sgombero completato su Gmail");
        emit_status(ctx, id).await;
        return Ok(false);
    }
    let inbox = vec![INBOX.to_owned()];
    let (add, remove) = if op.kind == "undo" { (inbox, vec![]) } else { (vec![], inbox) };
    match ctx.gmail.batch_modify_messages(&batch, &add, &remove).await {
        Ok(()) => {}
        // Un ID non più valido fa fallire l'intero batch: si riprova un messaggio alla volta, e si
        // saltano quelli spariti.
        Err(e) if !e.is_transient() && batch.len() > 1 => {
            tracing::warn!(op = id, error = %e, "sgombero: batch rifiutato, provo un messaggio alla volta");
            for m in &batch {
                match ctx.gmail.batch_modify_messages(std::slice::from_ref(m), &add, &remove).await {
                    Err(e) if e.is_transient() => return Err(e),
                    // Fatto, oppure sparito o non modificabile: in tutti e due i casi non c'è altro da fare.
                    _ => {}
                }
            }
        }
        Err(e) => return Err(e),
    }
    ctx.db
        .write_async(move |c| {
            let tx = c.transaction()?;
            for m in &batch {
                tx.execute("UPDATE bulk_op_messages SET applied = 1 WHERE op_id = ?1 AND message_id = ?2", params![id, m])?;
            }
            tx.execute(
                "UPDATE bulk_ops SET messages_done = (SELECT count(*) FROM bulk_op_messages WHERE op_id = ?1 AND applied = 1) WHERE id = ?1",
                [id],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    emit_status(ctx, id).await;
    Ok(true)
}

/// Worker degli sgomberi: un'operazione alla volta, nell'ordine in cui sono partite.
pub async fn run<G: GmailApi>(ctx: Arc<Ctx<G>>) {
    let mut attempt = 0u32;
    loop {
        if ctx.is_auth_required() {
            ctx.bulk_notify.notified().await;
            continue;
        }
        let op = match ctx.db.read_async(|c| next_pending(c)).await {
            Ok(Some(op)) => op,
            Ok(None) => {
                tokio::select! {
                    _ = ctx.bulk_notify.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                }
                continue;
            }
            Err(e) => {
                tracing::error!(error = %e, "sgombero: lettura fallita");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        match step(&ctx, &op).await {
            Ok(_) => attempt = 0,
            Err(e) if e.is_transient() || matches!(e, Error::Network(_)) => {
                attempt += 1;
                tracing::debug!(op = op.id, error = %e, "sgombero: riprovo");
                tokio::time::sleep(Duration::from_millis(backoff_ms(attempt, 60_000))).await;
            }
            Err(e) => {
                attempt += 1;
                tracing::warn!(op = op.id, error = %e, tentativi = attempt, "sgombero: errore");
                if attempt >= 5 {
                    let (id, msg) = (op.id, e.to_string());
                    let _ = ctx
                        .db
                        .write_async(move |c| {
                            c.execute("UPDATE bulk_ops SET state = 'failed', last_error = ?2 WHERE id = ?1", params![id, msg])?;
                            Ok(())
                        })
                        .await;
                    emit_status(&ctx, op.id).await;
                    attempt = 0;
                } else {
                    tokio::time::sleep(Duration::from_millis(backoff_ms(attempt, 60_000))).await;
                }
            }
        }
    }
}

/// Inizio della finestra dei metadati (per decidere se l'inbox vecchia esce dalla coda).
pub fn window_start(months: u32) -> i64 {
    now_ms() - i64::from(months) * 31 * 24 * 3600 * 1000
}

pub fn count_local_before(c: &Connection, cutoff: i64) -> Result<i64> {
    count_local(c, cutoff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_db;
    use crate::store;
    use crate::store::testdata::{msg, thread};

    #[test]
    fn local_apply_and_undo_are_symmetric() {
        let (_d, db) = test_db();
        db.write(|c| {
            store::ensure_account(c, "tommaso@esempio.it", None, None)?;
            store::write_thread(c, &thread("old", 1, vec![msg("old", "old", 1_000, "a@x.it", &["INBOX", "UNREAD"], None)]), false, "tommaso@esempio.it")?;
            store::write_thread(c, &thread("new", 2, vec![msg("new", "new", 9_000, "b@x.it", &["INBOX"], None)]), false, "tommaso@esempio.it")?;
            store::enqueue_fetch(c, &[("vecchissimo".into(), None)], prio::INBOX_OLD, false)?;
            assert_eq!(count_local(c, 5_000)?, 1);

            let (op, n) = start_done(c, 5_000, 0)?;
            assert_eq!(n, 1);
            let old = store::thread_summary(c, "old")?.unwrap();
            assert!(!old.in_inbox && old.unread, "esce dall'inbox, il resto non cambia");
            assert!(!old.labels.contains(&"INBOX".to_owned()));
            assert!(store::thread_summary(c, "new")?.unwrap().in_inbox);
            let queued: i64 = c.query_row("SELECT count(*) FROM fetch_queue", [], |r| r.get(0))?;
            assert_eq!(queued, 0, "l'inbox oltre la finestra esce dalla coda");
            let label_json: String = c.query_row("SELECT label_ids FROM messages WHERE id = 'old'", [], |r| r.get(0))?;
            assert_eq!(label_json, r#"["UNREAD"]"#);

            start_undo(c, op)?;
            let old = store::thread_summary(c, "old")?.unwrap();
            assert!(old.in_inbox && old.labels.contains(&"INBOX".to_owned()));
            assert_eq!(state_of(c, op)?, "cancelled", "Gmail non aveva ancora finito: il worker si ferma");
            assert!(start_undo(c, op).is_err(), "non si annulla due volte");
            Ok(())
        })
        .unwrap();
    }
}
