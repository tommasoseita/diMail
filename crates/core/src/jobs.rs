//! Lavori a tempo (sez. 10 della spec). Per ora Posticipa: il thread esce dall'inbox (con l'etichetta
//! `diMail/Posticipati`, così anche Gmail web sa dov'è) e all'ora scelta ci torna, in cima alla lista.
//! Se nel frattempo arriva una risposta, Gmail lo rimette in inbox da solo: il lavoro si chiude prima.
//!
//! Funziona solo a Mac acceso e app aperta, come dice la spec: al risveglio e all'avvio i lavori
//! scaduti partono subito. Lo stato passa da `in_attesa` a `in_esecuzione` in modo atomico.

use std::sync::Arc;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::events::CoreEvent;
use crate::gmail::GmailApi;
use crate::outbox::{self, Action};
use crate::sync::Ctx;
use crate::util::now_ms;
use crate::Result;

/// Su Gmail. Fino al 9 novembre 2026 si chiamava `Dimail/Posticipati`: si cerca senza distinguere
/// le maiuscole, così quella già creata continua a valere.
pub const SNOOZE_LABEL: &str = "diMail/Posticipati";

#[derive(Debug, Clone, Serialize)]
pub struct Snoozed {
    pub job_id: i64,
    pub thread_id: String,
    pub due_at: i64,
}

/// L'etichetta di Gmail per i posticipati, se esiste già in locale.
pub fn snooze_label_id(c: &Connection) -> Result<Option<String>> {
    Ok(c.query_row("SELECT id FROM labels WHERE lower(name) = lower(?1)", [SNOOZE_LABEL], |r| r.get(0)).optional()?)
}

/// Posticipa: azioni ottimistiche (fuori dall'inbox, etichetta) e un lavoro per thread.
/// Restituisce le operazioni della coda in uscita (per l'annulla) e i lavori.
pub fn snooze(c: &mut Connection, thread_ids: &[String], due_at: i64, label_id: Option<&str>) -> Result<(Vec<i64>, Vec<i64>)> {
    let mut ops = outbox::enqueue_action(c, thread_ids, &Action::Archive)?;
    if let Some(l) = label_id {
        ops.extend(outbox::enqueue_action(c, thread_ids, &Action::AddLabel { label_id: l.to_owned() })?);
    }
    let tx = c.transaction()?;
    let mut jobs = Vec::new();
    for t in thread_ids {
        // Un solo posticipo attivo per thread: il nuovo sostituisce il vecchio.
        tx.execute("UPDATE jobs SET state = 'annullato', completed_at = ?2 WHERE thread_id = ?1 AND kind = 'snooze' AND state = 'in_attesa'",
            params![t, now_ms()])?;
        tx.execute(
            "INSERT INTO jobs (kind, thread_id, due_at, state, payload, created_at) VALUES ('snooze', ?1, ?2, 'in_attesa', ?3, ?4)",
            params![t, due_at, serde_json::json!({ "label_id": label_id }).to_string(), now_ms()],
        )?;
        jobs.push(tx.last_insert_rowid());
    }
    tx.commit()?;
    Ok((ops, jobs))
}

pub fn cancel(c: &Connection, job_ids: &[i64]) -> Result<()> {
    for id in job_ids {
        c.execute("UPDATE jobs SET state = 'annullato', completed_at = ?2 WHERE id = ?1 AND state = 'in_attesa'", params![id, now_ms()])?;
    }
    Ok(())
}

pub fn list_snoozed(c: &Connection) -> Result<Vec<Snoozed>> {
    Ok(c
        .prepare("SELECT id, thread_id, due_at FROM jobs WHERE kind = 'snooze' AND state = 'in_attesa' ORDER BY due_at")?
        .query_map([], |r| Ok(Snoozed { job_id: r.get(0)?, thread_id: r.get(1)?, due_at: r.get(2)? }))?
        .collect::<Result<_, _>>()?)
}

struct Due {
    id: i64,
    thread_id: String,
    label_id: Option<String>,
    early: bool,
}

/// Lavori da eseguire adesso: scaduti, oppure posticipati tornati in inbox da soli (una risposta).
/// Il passaggio a `in_esecuzione` è atomico: un lavoro non parte due volte.
fn claim_due(c: &mut Connection) -> Result<Vec<Due>> {
    let tx = c.transaction()?;
    let now = now_ms();
    let rows: Vec<(i64, String, String, bool)> = tx
        .prepare(
            "SELECT j.id, j.thread_id, j.payload, j.due_at > ?1 FROM jobs j LEFT JOIN threads t ON t.id = j.thread_id
             WHERE j.kind = 'snooze' AND j.state = 'in_attesa'
               AND (j.due_at <= ?1 OR (t.in_inbox = 1 AND j.created_at < ?1 - 60000))",
        )?
        .query_map([now], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for (id, thread_id, payload, early) in rows {
        if tx.execute("UPDATE jobs SET state = 'in_esecuzione' WHERE id = ?1 AND state = 'in_attesa'", [id])? == 1 {
            let label_id = serde_json::from_str::<serde_json::Value>(&payload)
                .ok()
                .and_then(|v| v.get("label_id").and_then(|l| l.as_str()).map(str::to_owned));
            out.push(Due { id, thread_id, label_id, early });
        }
    }
    tx.commit()?;
    Ok(out)
}

/// Il posticipo è finito: il thread torna in inbox, senza etichetta, e in cima alla lista.
fn finish(c: &mut Connection, d: &Due) -> Result<()> {
    let ids = [d.thread_id.clone()];
    let exists: bool = c.query_row("SELECT EXISTS (SELECT 1 FROM threads WHERE id = ?1)", [&d.thread_id], |r| r.get(0))?;
    if exists {
        if !d.early {
            outbox::enqueue_action(c, &ids, &Action::MoveToInbox)?;
        }
        if let Some(l) = &d.label_id {
            outbox::enqueue_action(c, &ids, &Action::RemoveLabel { label_id: l.clone() })?;
        }
        c.execute("UPDATE threads SET reminded_at = ?2 WHERE id = ?1", params![d.thread_id, now_ms()])?;
    }
    c.execute("UPDATE jobs SET state = 'completato', completed_at = ?2 WHERE id = ?1", params![d.id, now_ms()])?;
    Ok(())
}

/// Scheduler: ogni 30 secondi, e subito al risveglio, al ritorno della rete e quando cambia qualcosa.
pub async fn run<G: GmailApi>(ctx: Arc<Ctx<G>>) {
    loop {
        match ctx.db.write_async(|c| {
            let due = claim_due(c)?;
            let mut done = Vec::new();
            for d in &due {
                finish(c, d)?;
                done.push(d.thread_id.clone());
            }
            Ok(done)
        }).await {
            Ok(done) if !done.is_empty() => {
                tracing::info!(n = done.len(), "posticipati tornati in inbox");
                ctx.emit(CoreEvent::ThreadsChanged { ids: done });
                ctx.outbox_notify.notify_one();
            }
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "scheduler: lettura fallita"),
        }
        tokio::select! {
            _ = ctx.jobs_notify.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
        }
    }
}

/// L'etichetta `diMail/Posticipati`: se manca, la crea su Gmail (una volta sola).
pub async fn ensure_snooze_label<G: GmailApi>(ctx: &Ctx<G>) -> Option<String> {
    if let Ok(Some(id)) = ctx.db.read_async(|c| snooze_label_id(c)).await {
        return Some(id);
    }
    match ctx.gmail.create_label(SNOOZE_LABEL).await {
        Ok(_) => {
            let _ = crate::sync::refresh_labels(ctx).await;
            ctx.db.read_async(|c| snooze_label_id(c)).await.ok().flatten()
        }
        Err(e) => {
            // Senza etichetta il posticipo funziona lo stesso: Gmail web lo vede solo come archiviato.
            tracing::warn!(error = %e, "etichetta dei posticipati non creata");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_db;
    use crate::store;
    use crate::store::testdata::{msg, thread};

    #[test]
    fn snooze_leaves_inbox_and_comes_back_at_the_top() {
        let (_d, db) = test_db();
        db.write(|c| {
            store::ensure_account(c, "tommaso@esempio.it", None, None)?;
            store::write_thread(c, &thread("t1", 1, vec![msg("m1", "t1", 1_000, "a@x.it", &["INBOX"], Some("x"))]), true, "tommaso@esempio.it")?;
            Ok(())
        })
        .unwrap();
        db.write(|c| {
            let (_ops, jobs) = snooze(c, &["t1".into()], now_ms() - 1, Some("Label_snooze"))?;
            assert_eq!(jobs.len(), 1);
            let t = store::thread_summary(c, "t1")?.unwrap();
            assert!(!t.in_inbox && t.labels.contains(&"Label_snooze".to_owned()));
            // È già scaduto: lo scheduler lo riporta.
            let due = claim_due(c)?;
            assert_eq!(due.len(), 1);
            assert!(claim_due(c)?.is_empty(), "non parte due volte");
            finish(c, &due[0])?;
            let t = store::thread_summary(c, "t1")?.unwrap();
            assert!(t.in_inbox && !t.labels.contains(&"Label_snooze".to_owned()));
            let reminded: Option<i64> = c.query_row("SELECT reminded_at FROM threads WHERE id = 't1'", [], |r| r.get(0))?;
            assert!(reminded.is_some());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_reply_brings_it_back_early_and_cancel_stops_it() {
        let (_d, db) = test_db();
        db.write(|c| {
            store::ensure_account(c, "tommaso@esempio.it", None, None)?;
            store::write_thread(c, &thread("t1", 1, vec![msg("m1", "t1", 1_000, "a@x.it", &["INBOX"], Some("x"))]), true, "tommaso@esempio.it")?;
            store::write_thread(c, &thread("t2", 1, vec![msg("m2", "t2", 1_000, "b@x.it", &["INBOX"], Some("y"))]), true, "tommaso@esempio.it")?;
            Ok(())
        })
        .unwrap();
        db.write(|c| {
            let tomorrow = now_ms() + 86_400_000;
            let (_, j1) = snooze(c, &["t1".into()], tomorrow, None)?;
            let (_, j2) = snooze(c, &["t2".into()], tomorrow, None)?;
            cancel(c, &j2)?;
            // Una risposta: Gmail rimette t1 in inbox. Il lavoro è vecchio di più di un minuto.
            c.execute("UPDATE jobs SET created_at = created_at - 120000 WHERE id = ?1", [j1[0]])?;
            store::apply_label_delta(c, "t1", &["INBOX".into()], &[])?;
            let due = claim_due(c)?;
            assert_eq!(due.iter().map(|d| (d.thread_id.as_str(), d.early)).collect::<Vec<_>>(), [("t1", true)]);
            assert!(list_snoozed(c)?.is_empty());
            Ok(())
        })
        .unwrap();
    }
}
