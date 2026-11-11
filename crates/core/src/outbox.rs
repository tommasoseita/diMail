//! Azioni ottimistiche e coda in uscita (sez. 5.4).
//!
//! 1. Una transazione cambia lo stato locale e aggiunge l'operazione alla coda.
//! 2. L'interfaccia si aggiorna subito.
//! 3. Il worker svuota la coda verso Gmail, raggruppando in batch, con ritentativi.
//! 4. Se Gmail rifiuta in modo definitivo, lo stato locale torna indietro e parte un avviso.
//! 5. Finché un'operazione è in coda, la sync non sovrascrive lo stato locale (vedi
//!    `store::reapply_pending_ops`).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::events::CoreEvent;
use crate::gmail::{GmailApi, RemoteOp};
use crate::store::{self, INBOX, STARRED, UNREAD, apply_label_delta, local_delta, prio};
use crate::sync::Ctx;
use crate::util::{backoff_ms, now_ms};
use crate::{Error, Result};

/// Dopo questi tentativi falliti per errori transitori, l'operazione si considera fallita.
const MAX_ATTEMPTS: i64 = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Archive,
    MoveToInbox,
    MarkRead,
    MarkUnread,
    Star,
    Unstar,
    Trash,
    Untrash,
    AddLabel { label_id: String },
    RemoveLabel { label_id: String },
}

fn modify(add: &[&str], remove: &[&str]) -> (&'static str, String) {
    ("modify", json!({ "add": add, "remove": remove }).to_string())
}

fn op_for(action: &Action, was_in_inbox: bool) -> (&'static str, String) {
    match action {
        Action::Archive => modify(&[], &[INBOX]),
        Action::MoveToInbox => modify(&[INBOX], &[]),
        Action::MarkRead => modify(&[], &[UNREAD]),
        Action::MarkUnread => modify(&[UNREAD], &[]),
        Action::Star => modify(&[STARRED], &[]),
        Action::Unstar => modify(&[], &[STARRED]),
        Action::Trash => ("trash", json!({ "was_in_inbox": was_in_inbox }).to_string()),
        Action::Untrash => ("untrash", "{}".to_owned()),
        Action::AddLabel { label_id } => modify(&[label_id], &[]),
        Action::RemoveLabel { label_id } => modify(&[], &[label_id]),
    }
}

fn insert_op(c: &Connection, kind: &str, thread_id: &str, payload: &str) -> Result<i64> {
    let now = now_ms();
    c.execute(
        "INSERT INTO outbox_ops (kind, thread_id, payload, next_attempt_at, created_at) VALUES (?1, ?2, ?3, ?4, ?4)",
        params![kind, thread_id, payload, now],
    )?;
    let id = c.last_insert_rowid();
    let (add, remove) = local_delta(kind, payload);
    apply_label_delta(c, thread_id, &add, &remove)?;
    Ok(id)
}

/// Applica l'azione in locale e la mette in coda, in un'unica transazione.
/// Restituisce gli ID delle operazioni (servono per annullare).
pub fn enqueue_action(c: &mut Connection, thread_ids: &[String], action: &Action) -> Result<Vec<i64>> {
    let tx = c.transaction()?;
    let mut ids = Vec::with_capacity(thread_ids.len());
    for t in thread_ids {
        let in_inbox: Option<bool> = tx
            .query_row("SELECT in_inbox FROM threads WHERE id = ?1", [t], |r| r.get(0))
            .optional()?;
        let Some(in_inbox) = in_inbox else { continue };
        let (kind, payload) = op_for(action, in_inbox);
        ids.push(insert_op(&tx, kind, t, &payload)?);
    }
    tx.commit()?;
    Ok(ids)
}

/// Annullare aggiunge l'operazione inversa alla coda (sez. 5.4).
pub fn enqueue_undo(c: &mut Connection, op_ids: &[i64]) -> Result<Vec<i64>> {
    #[derive(Deserialize, Default)]
    struct Mod {
        #[serde(default)]
        add: Vec<String>,
        #[serde(default)]
        remove: Vec<String>,
    }
    #[derive(Deserialize, Default)]
    struct TrashP {
        #[serde(default)]
        was_in_inbox: bool,
    }
    let tx = c.transaction()?;
    let mut out = Vec::new();
    // In ordine inverso, come una pila.
    for id in op_ids.iter().rev() {
        let row: Option<(String, String, String)> = tx
            .query_row("SELECT kind, thread_id, payload FROM outbox_ops WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let Some((kind, thread, payload)) = row else { continue };
        match kind.as_str() {
            "modify" => {
                let m: Mod = serde_json::from_str(&payload).unwrap_or_default();
                let inv = json!({ "add": m.remove, "remove": m.add }).to_string();
                out.push(insert_op(&tx, "modify", &thread, &inv)?);
            }
            "trash" => {
                let p: TrashP = serde_json::from_str(&payload).unwrap_or_default();
                out.push(insert_op(&tx, "untrash", &thread, "{}")?);
                if p.was_in_inbox {
                    let (k, pl) = modify(&[INBOX], &[]);
                    out.push(insert_op(&tx, k, &thread, &pl)?);
                }
            }
            "untrash" => {
                let (k, pl) = op_for(&Action::Trash, false);
                out.push(insert_op(&tx, k, &thread, &pl)?);
            }
            _ => {}
        }
    }
    tx.commit()?;
    Ok(out)
}

#[derive(Debug, Clone)]
struct OpRow {
    id: i64,
    kind: String,
    thread_id: String,
    payload: String,
    attempts: i64,
}

fn to_remote(op: &OpRow) -> RemoteOp {
    match op.kind.as_str() {
        "trash" => RemoteOp::Trash { thread_id: op.thread_id.clone() },
        "untrash" => RemoteOp::Untrash { thread_id: op.thread_id.clone() },
        _ => {
            let (add, remove) = local_delta(&op.kind, &op.payload);
            RemoteOp::Modify { thread_id: op.thread_id.clone(), add, remove }
        }
    }
}

/// Prende le operazioni da inviare: al massimo una per thread (l'ordine sullo stesso thread conta,
/// e dentro un batch Gmail non lo garantisce), e nessuna su thread con un'operazione già in volo.
fn claim_due(c: &mut Connection, limit: usize) -> Result<Vec<OpRow>> {
    let tx = c.transaction()?;
    let busy: HashSet<String> = tx
        .prepare("SELECT thread_id FROM outbox_ops WHERE state = 'in_flight'")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let candidates: Vec<(OpRow, i64)> = tx
        .prepare(
            "SELECT id, kind, thread_id, payload, attempts, next_attempt_at FROM outbox_ops
             WHERE state = 'pending' ORDER BY id LIMIT 500",
        )?
        .query_map([], |r| {
            Ok((
                OpRow { id: r.get(0)?, kind: r.get(1)?, thread_id: r.get(2)?, payload: r.get(3)?, attempts: r.get(4)? },
                r.get::<_, i64>(5)?,
            ))
        })?
        .collect::<Result<_, _>>()?;
    let now = now_ms();
    let mut seen = busy;
    let mut out = Vec::new();
    for (op, due) in candidates {
        // La prima operazione di ogni thread blocca le successive, anche se non è ancora scaduta.
        if !seen.insert(op.thread_id.clone()) || due > now {
            continue;
        }
        tx.execute("UPDATE outbox_ops SET state = 'in_flight' WHERE id = ?1", [op.id])?;
        out.push(op);
        if out.len() == limit {
            break;
        }
    }
    tx.commit()?;
    Ok(out)
}

fn next_due(c: &Connection) -> Result<Option<i64>> {
    Ok(c.query_row("SELECT min(next_attempt_at) FROM outbox_ops WHERE state = 'pending'", [], |r| r.get(0))?)
}

/// Riprova più tardi. Un rifiuto per quota (`limited`) non conta come tentativo: un'azione
/// dell'utente non deve mai essere annullata perché la sync ha consumato la quota.
fn retry_later(c: &Connection, op: &OpRow, err: &str, limited: bool) -> Result<()> {
    let attempts = if limited { op.attempts } else { op.attempts + 1 };
    let wait = if limited { 1_000 } else { backoff_ms(attempts as u32, 5 * 60_000) as i64 };
    c.execute(
        "UPDATE outbox_ops SET state = 'pending', attempts = ?2, next_attempt_at = ?3, last_error = ?4 WHERE id = ?1",
        params![op.id, attempts, now_ms() + wait, err],
    )?;
    Ok(())
}

/// Fallimento definitivo: lo stato locale torna indietro subito e il thread si riallinea con Gmail.
fn fail(c: &Connection, op: &OpRow, err: &str) -> Result<()> {
    c.execute(
        "UPDATE outbox_ops SET state = 'failed', last_error = ?2, completed_at = ?3 WHERE id = ?1",
        params![op.id, err, now_ms()],
    )?;
    let (add, remove) = local_delta(&op.kind, &op.payload);
    apply_label_delta(c, &op.thread_id, &remove, &add)?;
    store::reapply_pending_ops(c, &op.thread_id)?;
    store::enqueue_fetch(c, &[(op.thread_id.clone(), None)], prio::USER, false)?;
    Ok(())
}

pub fn reset_in_flight(c: &Connection) -> Result<usize> {
    Ok(c.execute("UPDATE outbox_ops SET state = 'pending' WHERE state = 'in_flight'", [])?)
}

/// Worker della coda in uscita.
pub async fn run<G: GmailApi>(ctx: Arc<Ctx<G>>) {
    // Dopo un crash: le modifiche di etichette si possono ripetere senza danni.
    if let Err(e) = ctx.db.write_async(|c| reset_in_flight(c)).await {
        tracing::error!(error = %e, "coda in uscita: ripristino fallito");
    }
    loop {
        let ops = match ctx.db.write_async(|c| claim_due(c, crate::gmail::batch::MAX_BATCH)).await {
            Ok(ops) => ops,
            Err(e) => {
                tracing::error!(error = %e, "coda in uscita: lettura fallita");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        if ops.is_empty() {
            let due = ctx.db.read_async(|c| next_due(c)).await.ok().flatten();
            let wait = due.map(|d| Duration::from_millis((d - now_ms()).clamp(50, 30_000) as u64)).unwrap_or(Duration::from_secs(30));
            tokio::select! {
                _ = ctx.outbox_notify.notified() => {}
                _ = tokio::time::sleep(wait) => {}
            }
            continue;
        }
        if ctx.is_auth_required() {
            let _ = ctx.db.write_async(|c| reset_in_flight(c)).await;
            ctx.outbox_notify.notified().await;
            continue;
        }

        let remote: Vec<RemoteOp> = ops.iter().map(to_remote).collect();
        tracing::debug!(n = ops.len(), "invio azioni a Gmail");
        let result = ctx.gmail.apply_ops(&remote).await;
        let ops2 = ops.clone();
        let outcome = ctx
            .db
            .write_async(move |c| {
                let tx = c.transaction()?;
                let mut done = 0usize;
                let mut failed: Vec<(Option<String>, String)> = Vec::new();
                match result {
                    Err(e) => {
                        let msg = e.to_string();
                        for op in &ops2 {
                            if !e.is_transient() && !matches!(e, Error::AuthRevoked) && op.attempts + 1 >= MAX_ATTEMPTS {
                                fail(&tx, op, &msg)?;
                                failed.push((Some(op.thread_id.clone()), msg.clone()));
                            } else {
                                retry_later(&tx, op, &msg, e.is_rate_limit())?;
                            }
                        }
                        tx.commit()?;
                        return Ok((done, failed, Some(e)));
                    }
                    Ok(results) => {
                        for (op, res) in ops2.iter().zip(results) {
                            match res {
                                Ok(()) => {
                                    tx.execute("UPDATE outbox_ops SET state = 'done', completed_at = ?2, last_error = NULL WHERE id = ?1", params![op.id, now_ms()])?;
                                    done += 1;
                                }
                                Err(Error::NotFound) => {
                                    // Il thread non esiste più su Gmail: lo togliamo anche in locale.
                                    tx.execute("UPDATE outbox_ops SET state = 'done', completed_at = ?2, last_error = 'thread non trovato' WHERE id = ?1", params![op.id, now_ms()])?;
                                    store::enqueue_fetch(&tx, &[(op.thread_id.clone(), None)], prio::USER, false)?;
                                }
                                Err(e) if e.is_rate_limit() => retry_later(&tx, op, &e.to_string(), true)?,
                                Err(e) if e.is_transient() && op.attempts + 1 < MAX_ATTEMPTS => retry_later(&tx, op, &e.to_string(), false)?,
                                Err(e) => {
                                    let msg = e.to_string();
                                    fail(&tx, op, &msg)?;
                                    failed.push((Some(op.thread_id.clone()), msg));
                                }
                            }
                        }
                    }
                }
                tx.commit()?;
                Ok((done, failed, None))
            })
            .await;

        match outcome {
            Ok((done, failed, batch_err)) => {
                let ids: Vec<String> = ops.iter().map(|o| o.thread_id.clone()).collect();
                if !failed.is_empty() {
                    ctx.emit(CoreEvent::ThreadsChanged { ids: ids.clone() });
                    for (thread_id, message) in failed {
                        tracing::warn!(error = %message, "azione rifiutata da Gmail");
                        ctx.emit(CoreEvent::ActionFailed { thread_id, message });
                    }
                    ctx.fetch_urgent.notify_one();
                }
                if done > 0 {
                    // Subito dopo un'azione confermata, si risincronizza (sez. 5.3).
                    ctx.sync_trigger.notify_one();
                }
                match batch_err {
                    Some(Error::AuthRevoked) => ctx.set_auth_required(),
                    Some(e @ Error::Network(_)) => {
                        tracing::info!(error = %e, "coda in uscita: rete assente, riprovo più tardi");
                        ctx.set_offline(true);
                    }
                    Some(e) => tracing::warn!(error = %e, "coda in uscita: batch fallito"),
                    None => ctx.set_offline(false),
                }
                ctx.emit_status().await;
            }
            Err(e) => tracing::error!(error = %e, "coda in uscita: scrittura del risultato fallita"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_db;
    use crate::store::testdata::*;
    use crate::store::{ensure_account, thread_summary, write_thread};

    #[test]
    fn archive_is_local_immediately_and_undo_restores() {
        let (_d, db) = test_db();
        db.write(|c| {
            ensure_account(c, "me@esempio.it", None, None)?;
            write_thread(c, &thread("t1", 5, vec![msg("m1", "t1", 1, "a@x.it", &["INBOX", "UNREAD"], Some("x"))]), true, "me@esempio.it")?;
            let ops = enqueue_action(c, &["t1".into()], &Action::Archive)?;
            assert_eq!(ops.len(), 1);
            assert!(!thread_summary(c, "t1")?.unwrap().in_inbox);
            enqueue_undo(c, &ops)?;
            assert!(thread_summary(c, "t1")?.unwrap().in_inbox);
            let n: i64 = c.query_row("SELECT count(*) FROM outbox_ops WHERE state = 'pending'", [], |r| r.get(0))?;
            assert_eq!(n, 2);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn trash_undo_brings_back_to_inbox() {
        let (_d, db) = test_db();
        db.write(|c| {
            ensure_account(c, "me@esempio.it", None, None)?;
            write_thread(c, &thread("t1", 5, vec![msg("m1", "t1", 1, "a@x.it", &["INBOX"], Some("x"))]), true, "me@esempio.it")?;
            let ops = enqueue_action(c, &["t1".into()], &Action::Trash)?;
            let s = thread_summary(c, "t1")?.unwrap();
            assert!(!s.in_inbox && s.labels.contains(&"TRASH".to_string()));
            let undo = enqueue_undo(c, &ops)?;
            assert_eq!(undo.len(), 2);
            let s = thread_summary(c, "t1")?.unwrap();
            assert!(s.in_inbox && !s.labels.contains(&"TRASH".to_string()));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn claims_one_op_per_thread_in_order() {
        let (_d, db) = test_db();
        db.write(|c| {
            ensure_account(c, "me@esempio.it", None, None)?;
            for t in ["t1", "t2"] {
                write_thread(c, &thread(t, 5, vec![msg(&format!("m{t}"), t, 1, "a@x.it", &["INBOX", "UNREAD"], Some("x"))]), true, "me@esempio.it")?;
            }
            enqueue_action(c, &["t1".into()], &Action::MarkRead)?;
            enqueue_action(c, &["t1".into(), "t2".into()], &Action::Archive)?;
            let first = claim_due(c, 50)?;
            assert_eq!(first.iter().map(|o| o.thread_id.as_str()).collect::<Vec<_>>(), vec!["t1", "t2"]);
            assert_eq!(first[0].kind, "modify");
            assert!(first[0].payload.contains("UNREAD"));
            // Con t1 in volo, la seconda operazione su t1 aspetta.
            assert!(claim_due(c, 50)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn permanent_failure_reverts_local_state() {
        let (_d, db) = test_db();
        db.write(|c| {
            ensure_account(c, "me@esempio.it", None, None)?;
            write_thread(c, &thread("t1", 5, vec![msg("m1", "t1", 1, "a@x.it", &["INBOX"], Some("x"))]), true, "me@esempio.it")?;
            enqueue_action(c, &["t1".into()], &Action::Archive)?;
            let ops = claim_due(c, 50)?;
            fail(c, &ops[0], "rifiutato")?;
            assert!(thread_summary(c, "t1")?.unwrap().in_inbox);
            let q: i64 = c.query_row("SELECT count(*) FROM fetch_queue WHERE thread_id = 't1'", [], |r| r.get(0))?;
            assert_eq!(q, 1);
            Ok(())
        })
        .unwrap();
    }
}
