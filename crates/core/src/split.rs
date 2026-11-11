//! Inbox divisa: Importanti e Altro (sez. 8 della spec), calcolata in locale, senza server.
//!
//! Un thread va in Altro se vale almeno una di queste condizioni, altrimenti in Importanti:
//! 1. Gmail lo ha messo in Promozioni, Social, Aggiornamenti o Forum;
//! 2. contiene un header di disiscrizione o di invio di massa;
//! 3. il mittente è un indirizzo "no-reply" o un servizio automatico;
//! 4. Gmail non lo considera importante e non ho mai scritto a quel mittente.
//!
//! Lo riportano sempre in Importanti: un mittente dei miei domini (impostazioni; di base quello
//! dell'account, se non è un provider personale come gmail.com), un mittente a cui ho già
//! scritto, l'essere nel campo A (non solo in Cc) di un thread a cui ho partecipato.
//! Le correzioni dell'utente (per thread, per mittente, per dominio) prevalgono su tutto.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::Result;
use crate::util::now_ms;

pub const IMPORTANT: &str = "important";
pub const OTHER: &str = "other";

const CATEGORIES: &[&str] = &["CATEGORY_PROMOTIONS", "CATEGORY_SOCIAL", "CATEGORY_UPDATES", "CATEGORY_FORUMS"];

/// Indirizzi di servizi automatici: si guarda la parte prima della chiocciola.
fn automatic_sender(email: &str) -> bool {
    let local = email.split('@').next().unwrap_or("").to_ascii_lowercase();
    let local = local.replace(['-', '_', '.'], "");
    ["noreply", "donotreply", "notreply", "noresponse", "notification", "notifications", "notify", "mailerdaemon",
     "postmaster", "bounce", "bounces", "newsletter", "newsletters", "alerts", "alert", "automated", "system"]
        .iter()
        .any(|p| local == *p || local.starts_with(p))
}

struct Msg {
    from: Option<String>,
    sent: bool,
    to_me: bool,
    bulk: bool,
}

/// Correzione esplicita: thread, poi mittente, poi dominio.
fn correction(c: &Connection, thread_id: &str, sender: Option<&str>) -> Result<Option<String>> {
    if let Some(s) = c
        .query_row("SELECT split FROM split_thread_overrides WHERE thread_id = ?1", [thread_id], |r| r.get(0))
        .optional()?
    {
        return Ok(Some(s));
    }
    let Some(sender) = sender else { return Ok(None) };
    if let Some(s) = c
        .query_row("SELECT split FROM split_rules WHERE kind = 'sender' AND pattern = ?1", [sender], |r| r.get(0))
        .optional()?
    {
        return Ok(Some(s));
    }
    let domain = sender.rsplit_once('@').map(|(_, d)| d).unwrap_or("");
    Ok(c
        .query_row("SELECT split FROM split_rules WHERE kind = 'domain' AND pattern = ?1", [domain], |r| r.get(0))
        .optional()?)
}

/// Il mittente che conta per le correzioni: l'ultimo che non sono io.
pub fn main_sender(c: &Connection, thread_id: &str, me: &str) -> Result<Option<String>> {
    Ok(c
        .query_row(
            "SELECT from_email FROM messages WHERE thread_id = ?1 AND from_email IS NOT NULL AND from_email <> ?2
               AND label_ids NOT LIKE '%\"SENT\"%' AND label_ids NOT LIKE '%\"DRAFT\"%'
             ORDER BY internal_date DESC LIMIT 1",
            params![thread_id, me],
            |r| r.get(0),
        )
        .optional()?)
}

pub fn classify(c: &Connection, thread_id: &str, me: &str) -> Result<&'static str> {
    let my_domains = crate::store::my_domains(c)?;
    let me_json = format!("\"email\":\"{me}\"");
    let msgs: Vec<Msg> = c
        .prepare_cached(
            "SELECT from_email, label_ids, to_json, list_unsubscribe IS NOT NULL OR list_id IS NOT NULL
                    OR lower(coalesce(precedence, '')) IN ('bulk', 'list', 'junk')
                    OR (auto_submitted IS NOT NULL AND lower(auto_submitted) <> 'no')
             FROM messages WHERE thread_id = ?1",
        )?
        .query_map([thread_id], |r| {
            let labels: String = r.get(1)?;
            let to: String = r.get(2)?;
            Ok(Msg {
                from: r.get(0)?,
                sent: labels.contains("\"SENT\""),
                to_me: to.contains(&me_json),
                bulk: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    let labels: HashSet<String> = c
        .prepare_cached("SELECT label_id FROM thread_labels WHERE thread_id = ?1")?
        .query_map([thread_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;

    let received: Vec<&Msg> = msgs.iter().filter(|m| !m.sent && m.from.as_deref().is_some_and(|f| f != me)).collect();
    let senders: Vec<&str> = received.iter().filter_map(|m| m.from.as_deref()).collect();

    let sender = senders.last().copied();
    if let Some(s) = correction(c, thread_id, sender)? {
        return Ok(if s == OTHER { OTHER } else { IMPORTANT });
    }
    // Solo cose mie (bozze, inviati a me stesso): Importanti.
    if senders.is_empty() {
        return Ok(IMPORTANT);
    }

    // Sempre Importanti.
    if senders.iter().any(|s| crate::store::is_my_domain(&my_domains, s)) {
        return Ok(IMPORTANT);
    }
    let mut wrote_to = c.prepare_cached("SELECT sent_count > 0 FROM contacts WHERE email = ?1")?;
    let mut any_written = false;
    for s in &senders {
        if wrote_to.query_row([s], |r| r.get::<_, bool>(0)).optional()?.unwrap_or(false) {
            any_written = true;
            break;
        }
    }
    if any_written {
        return Ok(IMPORTANT);
    }
    let participated = msgs.iter().any(|m| m.sent);
    if participated && received.iter().any(|m| m.to_me) {
        return Ok(IMPORTANT);
    }

    // Altro.
    if CATEGORIES.iter().any(|cat| labels.contains(*cat))
        || received.iter().any(|m| m.bulk)
        || senders.iter().any(|s| automatic_sender(s))
        || !labels.contains("IMPORTANT")
    {
        return Ok(OTHER);
    }
    Ok(IMPORTANT)
}

pub fn my_email(c: &Connection) -> Result<Option<String>> {
    Ok(c.query_row("SELECT email FROM account WHERE id = 1", [], |r| r.get(0)).optional()?)
}

/// Ricalcola la divisione di un thread e la salva.
pub fn recompute(c: &Connection, thread_id: &str) -> Result<()> {
    let Some(me) = my_email(c)? else { return Ok(()) };
    let split = classify(c, thread_id, &me)?;
    c.execute("UPDATE threads SET split = ?2 WHERE id = ?1 AND coalesce(split, '') <> ?2", params![thread_id, split])?;
    Ok(())
}

/// Ricalcola i thread di un mittente (dopo che gli ho scritto per la prima volta, o dopo una regola).
pub fn recompute_sender(c: &Connection, email: &str) -> Result<usize> {
    let ids: Vec<String> = c
        .prepare_cached("SELECT DISTINCT thread_id FROM messages WHERE from_email = ?1")?
        .query_map([email], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for id in &ids {
        recompute(c, id)?;
    }
    Ok(ids.len())
}

pub(crate) fn recompute_domain(c: &Connection, domain: &str) -> Result<usize> {
    let ids: Vec<String> = c
        .prepare("SELECT DISTINCT thread_id FROM messages WHERE from_email LIKE ?1")?
        .query_map([format!("%@{domain}")], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for id in &ids {
        recompute(c, id)?;
    }
    Ok(ids.len())
}

/// I thread ancora senza divisione (dopo la migrazione). Restituisce quanti.
pub fn recompute_missing(c: &mut Connection) -> Result<usize> {
    let tx = c.transaction()?;
    let ids: Vec<String> = tx
        .prepare("SELECT id FROM threads WHERE split IS NULL")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for id in &ids {
        recompute(&tx, id)?;
    }
    tx.commit()?;
    Ok(ids.len())
}

/// Tutti i thread: dopo la migrazione, o quando cambiano le regole di classificazione.
pub fn recompute_all(c: &mut Connection) -> Result<usize> {
    let tx = c.transaction()?;
    let ids: Vec<String> = tx.prepare("SELECT id FROM threads")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    for id in &ids {
        recompute(&tx, id)?;
    }
    tx.commit()?;
    Ok(ids.len())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Remember {
    /// Solo questi thread.
    Thread,
    /// Sempre per il mittente.
    Sender,
    /// Sempre per tutto il dominio del mittente.
    Domain,
}

/// Correzione manuale: sposta i thread nella scheda `split` e, se richiesto, lo ricorda per il
/// mittente o per il dominio. Restituisce i thread ricalcolati (per aggiornare l'interfaccia).
pub fn correct(c: &mut Connection, thread_ids: &[String], split: &str, remember: Remember) -> Result<Vec<String>> {
    let split = if split == OTHER { OTHER } else { IMPORTANT };
    let me = my_email(c)?.unwrap_or_default();
    let tx = c.transaction()?;
    let mut touched: Vec<String> = Vec::new();
    for id in thread_ids {
        // La correzione vale per il thread oppure per il mittente o il dominio, non per tutti e due:
        // togliendo la regola, il thread torna alle regole automatiche.
        if remember == Remember::Thread {
            tx.execute(
                "INSERT INTO split_thread_overrides (thread_id, split, created_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (thread_id) DO UPDATE SET split = excluded.split, created_at = excluded.created_at",
                params![id, split, now_ms()],
            )?;
        } else {
            tx.execute("DELETE FROM split_thread_overrides WHERE thread_id = ?1", [id])?;
        }
        touched.push(id.clone());
        let sender = main_sender(&tx, id, &me)?;
        if let Some(sender) = sender.filter(|_| remember != Remember::Thread) {
            let (kind, pattern) = match remember {
                Remember::Domain => ("domain", sender.rsplit_once('@').map(|(_, d)| d.to_owned()).unwrap_or(sender.clone())),
                _ => ("sender", sender.clone()),
            };
            tx.execute(
                "INSERT INTO split_rules (kind, pattern, split, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (kind, pattern) DO UPDATE SET split = excluded.split, created_at = excluded.created_at",
                params![kind, pattern, split, now_ms()],
            )?;
            // Le correzioni per thread degli altri thread di quel mittente lasciano il posto alla regola.
            if kind == "sender" {
                recompute_sender(&tx, &pattern)?;
            } else {
                recompute_domain(&tx, &pattern)?;
            }
        }
        recompute(&tx, id)?;
    }
    tx.commit()?;
    Ok(touched)
}

#[derive(Debug, Clone, Serialize)]
pub struct SplitRule {
    pub kind: String,
    pub pattern: String,
    pub split: String,
    pub created_at: i64,
}

pub fn rules(c: &Connection) -> Result<Vec<SplitRule>> {
    Ok(c
        .prepare("SELECT kind, pattern, split, created_at FROM split_rules ORDER BY created_at DESC")?
        .query_map([], |r| Ok(SplitRule { kind: r.get(0)?, pattern: r.get(1)?, split: r.get(2)?, created_at: r.get(3)? }))?
        .collect::<Result<_, _>>()?)
}

pub fn remove_rule(c: &mut Connection, kind: &str, pattern: &str) -> Result<()> {
    let tx = c.transaction()?;
    tx.execute("DELETE FROM split_rules WHERE kind = ?1 AND pattern = ?2", params![kind, pattern])?;
    if kind == "sender" {
        recompute_sender(&tx, pattern)?;
    } else {
        recompute_domain(&tx, pattern)?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_db;
    use crate::gmail::{Message, MessagePart};
    use crate::store::{self, testdata::thread};

    const ME: &str = "tommaso@esempio.it";

    fn m(id: &str, t: &str, from: &str, to: &str, labels: &[&str], headers: &[(&str, &str)]) -> Message {
        let mut hs = vec![
            serde_json::json!({"name": "From", "value": from}),
            serde_json::json!({"name": "To", "value": to}),
            serde_json::json!({"name": "Subject", "value": "Oggetto"}),
        ];
        for (n, v) in headers {
            hs.push(serde_json::json!({"name": n, "value": v}));
        }
        Message {
            id: id.into(),
            thread_id: t.into(),
            label_ids: labels.iter().map(|s| s.to_string()).collect(),
            snippet: "x".into(),
            history_id: Some("1".into()),
            internal_date: Some("1000".into()),
            payload: Some(serde_json::from_value::<MessagePart>(serde_json::json!({"mimeType": "text/plain", "headers": hs, "body": {"size": 0}})).unwrap()),
            size_estimate: None,
        }
    }

    fn split_of(c: &Connection, id: &str) -> String {
        c.query_row("SELECT split FROM threads WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    }

    #[test]
    fn rules_of_section_8() {
        let (_d, db) = test_db();
        db.write(|c| {
            store::ensure_account(c, ME, None, None)?;
            let w = |c: &Connection, t: &str, msgs: Vec<Message>| store::write_thread(c, &thread(t, 1, msgs), false, ME).map(|_| ());
            // Promozioni di Gmail: Altro.
            w(c, "promo", vec![m("p", "promo", "Negozio <offerte@negozio.it>", ME, &["INBOX", "CATEGORY_PROMOTIONS", "IMPORTANT"], &[])])?;
            // Newsletter con List-Unsubscribe, anche se Gmail la segna importante: Altro.
            w(c, "news", vec![m("n", "news", "news@x.it", ME, &["INBOX", "IMPORTANT"], &[("List-Unsubscribe", "<mailto:u@x.it>")])])?;
            // Indirizzo no-reply: Altro.
            w(c, "noreply", vec![m("r", "noreply", "no-reply@servizio.com", ME, &["INBOX", "IMPORTANT"], &[])])?;
            // Sconosciuto e non importante per Gmail: Altro.
            w(c, "ignoto", vec![m("i", "ignoto", "chi@sconosciuto.it", ME, &["INBOX"], &[])])?;
            // Sconosciuto ma importante per Gmail: Importanti.
            w(c, "gmailimp", vec![m("g", "gmailimp", "chi@altro.it", ME, &["INBOX", "IMPORTANT"], &[])])?;
            // Collega, anche se in Aggiornamenti: Importanti.
            w(c, "collega", vec![m("c", "collega", "luca@esempio.it", ME, &["INBOX", "CATEGORY_UPDATES"], &[])])?;
            // Ho partecipato e sono in A: Importanti, anche se Gmail non lo segna importante.
            w(c, "partecipo", vec![
                m("a1", "partecipo", "anna@cliente.it", ME, &["INBOX"], &[]),
                m("a2", "partecipo", ME, "anna@cliente.it", &["SENT"], &[]),
            ])?;
            for (t, want) in [("promo", OTHER), ("news", OTHER), ("noreply", OTHER), ("ignoto", OTHER), ("gmailimp", IMPORTANT),
                              ("collega", IMPORTANT), ("partecipo", IMPORTANT)] {
                assert_eq!(split_of(c, t), want, "{t}");
            }
            // Ho scritto ad anna: adesso anche i suoi thread non importanti vanno in Importanti.
            w(c, "anna2", vec![m("b", "anna2", "anna@cliente.it", "altri@cliente.it", &["INBOX", "CATEGORY_UPDATES"], &[])])?;
            assert_eq!(split_of(c, "anna2"), IMPORTANT);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn my_domains_default_to_the_account_and_can_be_changed() {
        let (_d, db) = test_db();
        db.write(|c| {
            store::ensure_account(c, ME, None, None)?;
            let w = |c: &Connection, t: &str, from: &str| {
                store::write_thread(c, &thread(t, 1, vec![m(t, t, from, ME, &["INBOX", "CATEGORY_UPDATES"], &[])]), false, ME).map(|_| ())
            };
            w(c, "collega", "luca@esempio.it")?;
            w(c, "partner", "sara@partner.it")?;
            assert_eq!(split_of(c, "collega"), IMPORTANT);
            assert_eq!(split_of(c, "partner"), OTHER);
            let v = store::set_my_domains(c, &["@Partner.it ".into(), "esempio.it".into(), "".into()])?;
            assert_eq!(v.domains, vec!["partner.it", "esempio.it"]);
            assert!(v.custom);
            assert_eq!(split_of(c, "partner"), IMPORTANT);
            // Nessun dominio: il collega torna a dipendere dalle altre regole.
            store::set_my_domains(c, &[])?;
            assert_eq!(split_of(c, "collega"), OTHER);
            assert!(store::set_my_domains(c, &["non un dominio".into()]).is_err());
            Ok(())
        })
        .unwrap();

        // Con un indirizzo Gmail non c'è un dominio di colleghi.
        let (_d, db) = test_db();
        db.write(|c| {
            store::ensure_account(c, "mario@gmail.com", None, None)?;
            let v = store::my_domains_view(c)?;
            assert!(v.domains.is_empty() && v.suggested.is_none() && !v.custom);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn corrections_win_and_can_be_remembered() {
        let (_d, db) = test_db();
        db.write(|c| {
            store::ensure_account(c, ME, None, None)?;
            store::write_thread(c, &thread("a", 1, vec![m("a", "a", "info@fornitore.it", ME, &["INBOX"], &[])]), false, ME)?;
            store::write_thread(c, &thread("b", 1, vec![m("b", "b", "info@fornitore.it", ME, &["INBOX"], &[])]), false, ME)?;
            store::write_thread(c, &thread("c", 1, vec![m("c", "c", "altro@fornitore.it", ME, &["INBOX"], &[])]), false, ME)?;
            assert_eq!(split_of(c, "a"), OTHER);
            Ok(())
        })
        .unwrap();
        db.write(|c| {
            correct(c, &["a".into()], IMPORTANT, Remember::Thread)?;
            assert_eq!((split_of(c, "a").as_str(), split_of(c, "b").as_str()), (IMPORTANT, OTHER));
            correct(c, &["b".into()], IMPORTANT, Remember::Sender)?;
            assert_eq!((split_of(c, "b").as_str(), split_of(c, "c").as_str()), (IMPORTANT, OTHER));
            correct(c, &["c".into()], IMPORTANT, Remember::Domain)?;
            assert_eq!(split_of(c, "c"), IMPORTANT);
            assert_eq!(rules(c)?.len(), 2);
            remove_rule(c, "domain", "fornitore.it")?;
            assert_eq!(split_of(c, "c"), OTHER, "senza la regola torna dove lo mettono le regole automatiche");
            Ok(())
        })
        .unwrap();
    }
}
