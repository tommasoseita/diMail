//! Barra del contatto (specifica UX, sezione 4): chi è la persona di un thread, solo con i dati
//! della posta. Niente fonti esterne nella v1.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::Result;
use crate::store::{self, ThreadSummary};

#[derive(Debug, Clone, Serialize)]
pub struct ContactCard {
    pub email: String,
    pub name: Option<String>,
    /// Ricavata dal dominio, se non è un provider di posta personale.
    pub company: Option<String>,
    /// Dalla firma dell'ultimo messaggio di questa persona. Nel dubbio, niente.
    pub role: Option<String>,
    pub phone: Option<String>,
    /// Messaggi ricevuti da questa persona e messaggi che le ho scritto.
    pub received_count: i64,
    pub sent_count: i64,
    /// Primo messaggio ricevuto in locale (la finestra è di 12 mesi, l'inbox intera).
    pub first_date: Option<i64>,
    pub last_date: Option<i64>,
    pub recent: Vec<ThreadSummary>,
}

/// Provider di posta personale: un indirizzo lì non dice niente su azienda e colleghi.
pub(crate) const PERSONAL_DOMAINS: &[&str] = &[
    "gmail.com", "googlemail.com", "outlook.com", "outlook.it", "hotmail.com", "hotmail.it", "live.com", "live.it",
    "msn.com", "yahoo.com", "yahoo.it", "icloud.com", "me.com", "mac.com", "libero.it", "virgilio.it", "alice.it",
    "tim.it", "tin.it", "tiscali.it", "fastwebnet.it", "email.it", "inwind.it", "proton.me", "protonmail.com",
    "gmx.com", "gmx.it", "aol.com", "pec.it", "legalmail.it",
];

/// "acme-group.co.uk" → "Acme Group". `None` per i provider personali.
pub fn company_from_domain(email: &str) -> Option<String> {
    let domain = email.rsplit_once('@')?.1.to_ascii_lowercase();
    if PERSONAL_DOMAINS.contains(&domain.as_str()) {
        return None;
    }
    let parts: Vec<&str> = domain.split('.').collect();
    // Il nome è l'etichetta prima del suffisso: "mail.cliente.co.uk" → "cliente".
    let second_level = ["co", "com", "org", "net", "gov", "ac", "edu"];
    let idx = match parts.len() {
        0 | 1 => return None,
        n if n >= 3 && second_level.contains(&parts[n - 2]) => n - 3,
        n => n - 2,
    };
    let name = parts[idx]
        .split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    (!name.is_empty()).then_some(name)
}

fn digits(s: &str) -> usize {
    s.chars().filter(char::is_ascii_digit).count()
}

/// Un numero di telefono nella riga, con o senza etichetta ("Tel.", "Mob:", "M:"...).
fn phone_in(line: &str) -> Option<String> {
    let l = line.trim();
    // La parte dopo un'etichetta, se c'è.
    let lower = l.to_lowercase();
    // Dalla più lunga, così "telefono" non viene preso per "tel".
    let labels = ["cellulare", "telefono", "mobile", "phone", "cell", "mob", "tel", "m", "t", "p"];
    let body = labels
        .iter()
        .find(|label| {
            lower.starts_with(*label) && lower[label.len()..].starts_with([' ', ':', '.'])
        })
        .map(|label| l[label.len()..].trim_start_matches([' ', ':', '.']).trim())
        .unwrap_or(l);
    // Solo cifre, spazi e separatori tipici, tra 8 e 15 cifre.
    let ok = body.chars().all(|c| c.is_ascii_digit() || " +-./()".contains(c));
    let n = digits(body);
    (ok && (8..=15).contains(&n) && body.chars().next().is_some_and(|c| c == '+' || c == '(' || c.is_ascii_digit()))
        .then(|| body.to_owned())
}

/// Ruolo e telefono dalla firma. Si guarda la fine del testo (senza citazioni): la riga dopo quella
/// con il nome della persona, se è corta e sembra un ruolo, e il primo numero di telefono.
pub fn signature_info(text: &str, name: Option<&str>) -> (Option<String>, Option<String>) {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let tail = &lines[lines.len().saturating_sub(12)..];
    let phone = tail.iter().find_map(|l| phone_in(l));

    let mut role = None;
    if let Some(name) = name.map(str::trim).filter(|n| n.len() >= 3) {
        let name_l = name.to_lowercase();
        let first = name_l.split_whitespace().next().unwrap_or("");
        for (i, line) in tail.iter().enumerate() {
            let l = line.to_lowercase();
            // "Mario Rossi | Responsabile acquisti" sulla stessa riga.
            if l.starts_with(&name_l) {
                let rest = line[name.len()..].trim_start_matches([' ', '|', '-', ',', '·']).trim();
                if looks_like_role(rest) {
                    role = Some(rest.to_owned());
                } else if let Some(next) = tail.get(i + 1)
                    && looks_like_role(next)
                {
                    role = Some((*next).to_owned());
                }
                break;
            }
            if !first.is_empty() && l == first {
                if let Some(next) = tail.get(i + 1)
                    && looks_like_role(next)
                {
                    role = Some((*next).to_owned());
                }
                break;
            }
        }
    }
    (role, phone)
}

fn looks_like_role(s: &str) -> bool {
    let s = s.trim();
    (3..=60).contains(&s.chars().count())
        && s.chars().filter(|c| c.is_alphabetic()).count() >= 3
        && digits(s) == 0
        && !s.contains('@')
        && !s.to_lowercase().contains("http")
        && !s.to_lowercase().contains("www.")
        && !s.ends_with(['.', '!', '?', ':'])
}

pub fn contact_card(c: &Connection, email: &str) -> Result<ContactCard> {
    let email = email.to_lowercase();
    let (name, sent, received): (Option<String>, i64, i64) = c
        .query_row("SELECT name, sent_count, received_count FROM contacts WHERE email = ?1", [&email], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?
        .unwrap_or((None, 0, 0));
    let (first_date, last_date): (Option<i64>, Option<i64>) = c.query_row(
        "SELECT min(internal_date), max(internal_date) FROM messages WHERE from_email = ?1",
        [&email],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    // Firma dell'ultimo messaggio con il corpo.
    let last_text: Option<String> = c
        .query_row(
            "SELECT coalesce(b.text_unquoted, b.text_plain) FROM messages m JOIN message_bodies b ON b.message_rowid = m.rowid
             WHERE m.from_email = ?1 AND coalesce(b.text_unquoted, b.text_plain) IS NOT NULL
             ORDER BY m.internal_date DESC LIMIT 1",
            [&email],
            |r| r.get(0),
        )
        .optional()?;
    let (role, phone) = last_text.as_deref().map(|t| signature_info(t, name.as_deref())).unwrap_or((None, None));

    let ids: Vec<String> = c
        .prepare(
            "SELECT thread_id FROM messages WHERE from_email = ?1
             GROUP BY thread_id ORDER BY max(internal_date) DESC LIMIT 5",
        )?
        .query_map([&email], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut recent = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(t) = store::thread_summary(c, &id)? {
            recent.push(t);
        }
    }

    Ok(ContactCard {
        company: company_from_domain(&email),
        email,
        name,
        role,
        phone,
        received_count: received,
        sent_count: sent,
        first_date,
        last_date,
        recent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_db;
    use crate::store::testdata::{msg, thread};

    #[test]
    fn company_from_domain_skips_personal_providers() {
        assert_eq!(company_from_domain("anna@cliente-srl.it").as_deref(), Some("Cliente Srl"));
        assert_eq!(company_from_domain("bob@mail.acme.co.uk").as_deref(), Some("Acme"));
        assert_eq!(company_from_domain("tommaso@esempio.it").as_deref(), Some("Esempio"));
        assert_eq!(company_from_domain("mario@gmail.com"), None);
        assert_eq!(company_from_domain("senza-chiocciola"), None);
    }

    #[test]
    fn reads_role_and_phone_from_the_signature() {
        let text = "Ciao Tommaso,\nti mando il preventivo.\n\nA presto,\nAnna Rossi\nResponsabile acquisti\nCliente Srl\nTel. +39 011 123 4567\nwww.cliente.it";
        assert_eq!(
            signature_info(text, Some("Anna Rossi")),
            (Some("Responsabile acquisti".into()), Some("+39 011 123 4567".into()))
        );
        let inline = "Grazie\nMarco Bianchi | Head of Sales\nM: 333 1234567";
        assert_eq!(signature_info(inline, Some("Marco Bianchi")), (Some("Head of Sales".into()), Some("333 1234567".into())));
        // Una frase dopo il nome non è un ruolo; le date non sono telefoni.
        let doubt = "Luca\nCi sentiamo domani alle 10.\nInviato il 12/11/2026";
        assert_eq!(signature_info(doubt, Some("Luca Verdi")), (None, None));
    }

    #[test]
    fn card_counts_messages_and_lists_recent_threads() {
        let (_d, db) = test_db();
        db.write(|c| {
            store::ensure_account(c, "tommaso@esempio.it", None, None)?;
            for (t, d) in [("t1", 1_000), ("t2", 3_000), ("t3", 2_000)] {
                store::write_thread(c, &thread(t, d, vec![msg(&format!("{t}m"), t, d, "Anna Rossi <anna@cliente.it>", &["INBOX"], Some("Ciao\nAnna Rossi\nResponsabile acquisti"))]), true, "tommaso@esempio.it")?;
            }
            let card = contact_card(c, "Anna@Cliente.it")?;
            assert_eq!(card.received_count, 3);
            assert_eq!(card.company.as_deref(), Some("Cliente"));
            assert_eq!(card.role.as_deref(), Some("Responsabile acquisti"));
            assert_eq!(card.recent.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["t2", "t3", "t1"]);
            assert_eq!((card.first_date, card.last_date), (Some(1_000), Some(3_000)));
            Ok(())
        })
        .unwrap();
    }
}
