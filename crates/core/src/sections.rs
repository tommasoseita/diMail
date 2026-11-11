//! Sezioni: le etichette di Gmail nella barra a sinistra, con una configurazione locale.
//! Si possono creare (su Gmail), rinominare (su Gmail), riordinare e archiviare (in locale: Gmail
//! non ha un ordine né un archivio delle etichette). Una sezione può avere istruzioni per l'AI,
//! che ci porta da sola i thread che corrispondono, e può comparire come scheda accanto a
//! Importanti e Altro: i suoi thread in inbox stanno lì e non nelle altre due schede.

use rusqlite::{Connection, params};
use serde::Serialize;

use crate::Result;
use crate::util::now_ms;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Section {
    pub label_id: String,
    pub name: String,
    pub position: i64,
    pub archived: bool,
    pub show_as_tab: bool,
    pub instructions: Option<String>,
    pub color_bg: Option<String>,
    /// Thread in inbox con l'etichetta, e quanti non letti.
    pub inbox: i64,
    pub unread: i64,
}

/// Etichette dell'utente senza riga in `sections`: in fondo, in ordine di nome.
/// Le etichette di servizio di diMail (`diMail/…`) restano fuori (LIKE non distingue le maiuscole,
/// quindi anche le vecchie `Dimail/…`).
pub fn ensure_rows(c: &Connection) -> Result<()> {
    let max: i64 = c.query_row("SELECT coalesce(max(position), -1) FROM sections", [], |r| r.get(0))?;
    let missing: Vec<String> = c
        .prepare(
            "SELECT l.id FROM labels l WHERE l.type = 'user' AND l.name NOT LIKE 'diMail/%'
               AND NOT EXISTS (SELECT 1 FROM sections s WHERE s.label_id = l.id)
             ORDER BY l.name COLLATE NOCASE",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for (i, id) in missing.iter().enumerate() {
        c.execute(
            "INSERT OR IGNORE INTO sections (label_id, position, created_at) VALUES (?1, ?2, ?3)",
            params![id, max + 1 + i as i64, now_ms()],
        )?;
    }
    // Etichette cancellate su Gmail: via anche dalla configurazione.
    c.execute("DELETE FROM sections WHERE label_id NOT IN (SELECT id FROM labels)", [])?;
    Ok(())
}

pub fn list(c: &Connection) -> Result<Vec<Section>> {
    ensure_rows(c)?;
    Ok(c
        .prepare(
            "SELECT s.label_id, l.name, s.position, s.archived, s.show_as_tab, s.instructions, l.color_bg,
                    (SELECT count(*) FROM thread_labels x JOIN threads t ON t.id = x.thread_id WHERE x.label_id = s.label_id AND t.in_inbox = 1),
                    (SELECT count(*) FROM thread_labels x JOIN threads t ON t.id = x.thread_id WHERE x.label_id = s.label_id AND t.in_inbox = 1 AND t.unread = 1)
             FROM sections s JOIN labels l ON l.id = s.label_id
             ORDER BY s.position, l.name COLLATE NOCASE",
        )?
        .query_map([], |r| {
            Ok(Section {
                label_id: r.get(0)?,
                name: r.get(1)?,
                position: r.get(2)?,
                archived: r.get(3)?,
                show_as_tab: r.get(4)?,
                instructions: r.get(5)?,
                color_bg: r.get(6)?,
                inbox: r.get(7)?,
                unread: r.get(8)?,
            })
        })?
        .collect::<Result<_, _>>()?)
}

/// Nuova sezione, dopo averla creata su Gmail: in cima alla lista.
pub fn insert(c: &Connection, label_id: &str, show_as_tab: bool, instructions: Option<&str>) -> Result<()> {
    c.execute("UPDATE sections SET position = position + 1", [])?;
    c.execute(
        "INSERT INTO sections (label_id, position, archived, show_as_tab, instructions, created_at) VALUES (?1, 0, 0, ?2, ?3, ?4)
         ON CONFLICT (label_id) DO UPDATE SET position = 0, archived = 0, show_as_tab = excluded.show_as_tab, instructions = excluded.instructions",
        params![label_id, show_as_tab, clean(instructions), now_ms()],
    )?;
    Ok(())
}

fn clean(s: Option<&str>) -> Option<String> {
    s.map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

/// Scheda e istruzioni. Se le istruzioni cambiano, l'AI rivaluta i thread in inbox.
pub fn configure(c: &Connection, label_id: &str, show_as_tab: bool, instructions: Option<&str>) -> Result<bool> {
    ensure_rows(c)?;
    let before: Option<String> = c.query_row("SELECT instructions FROM sections WHERE label_id = ?1", [label_id], |r| r.get(0))?;
    let after = clean(instructions);
    c.execute(
        "UPDATE sections SET show_as_tab = ?2, instructions = ?3 WHERE label_id = ?1",
        params![label_id, show_as_tab, after],
    )?;
    let changed = before != after;
    if changed {
        c.execute("DELETE FROM section_checks WHERE label_id = ?1", [label_id])?;
    }
    Ok(changed)
}

pub fn set_archived(c: &Connection, label_id: &str, archived: bool) -> Result<()> {
    ensure_rows(c)?;
    c.execute("UPDATE sections SET archived = ?2 WHERE label_id = ?1", params![label_id, archived])?;
    Ok(())
}

/// Nuovo ordine: gli ID nell'ordine voluto; quelli non elencati vengono dopo, nell'ordine di prima.
pub fn reorder(c: &mut Connection, ordered: &[String]) -> Result<()> {
    ensure_rows(c)?;
    let tx = c.transaction()?;
    let rest: Vec<String> = tx
        .prepare("SELECT label_id FROM sections ORDER BY position")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<Vec<String>, _>>()?
        .into_iter()
        .filter(|id| !ordered.contains(id))
        .collect();
    for (i, id) in ordered.iter().chain(rest.iter()).enumerate() {
        tx.execute("UPDATE sections SET position = ?2 WHERE label_id = ?1", params![id, i as i64])?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_db;
    use crate::store::{self, View, testdata::{msg, thread}};

    fn add_label(c: &Connection, id: &str, name: &str) {
        c.execute("INSERT INTO labels (id, name, type) VALUES (?1, ?2, 'user')", params![id, name]).unwrap();
    }

    #[test]
    fn order_archive_and_tabs() {
        let (_d, db) = test_db();
        db.write(|c| {
            store::ensure_account(c, "tommaso@esempio.it", None, None)?;
            add_label(c, "L_b", "Beta");
            add_label(c, "L_a", "Alfa");
            add_label(c, "L_s", "diMail/Posticipati");
            let names: Vec<String> = list(c)?.into_iter().map(|s| s.name).collect();
            assert_eq!(names, ["Alfa", "Beta"], "in ordine di nome, senza le etichette di servizio");
            reorder(c, &["L_b".into()])?;
            assert_eq!(list(c)?.iter().map(|s| s.label_id.as_str()).collect::<Vec<_>>(), ["L_b", "L_a"]);
            set_archived(c, "L_a", true)?;
            assert!(list(c)?.iter().find(|s| s.label_id == "L_a").unwrap().archived);

            // Una sezione come scheda: i suoi thread escono da Importanti.
            store::write_thread(c, &thread("t1", 1, vec![msg("m1", "t1", 1_000, "a@x.it", &["INBOX", "L_b"], Some("x"))]), true, "tommaso@esempio.it")?;
            store::write_thread(c, &thread("t2", 1, vec![msg("m2", "t2", 2_000, "b@x.it", &["INBOX"], Some("y"))]), true, "tommaso@esempio.it")?;
            c.execute("UPDATE threads SET split = 'important'", [])?;
            assert_eq!(store::count_threads(c, &View::Important)?, 2);
            assert!(configure(c, "L_b", true, Some("  Tutti gli executive teacher  "))?);
            assert!(!configure(c, "L_b", true, Some("Tutti gli executive teacher"))?, "stesse istruzioni: niente da rifare");
            assert_eq!(store::count_threads(c, &View::Important)?, 1);
            assert_eq!(store::count_threads(c, &View::Section { id: "L_b".into() })?, 1);
            let s = list(c)?.into_iter().find(|s| s.label_id == "L_b").unwrap();
            assert_eq!((s.inbox, s.show_as_tab, s.instructions.as_deref()), (1, true, Some("Tutti gli executive teacher")));
            Ok(())
        })
        .unwrap();
    }
}
