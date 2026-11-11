//! Pulizia dell'HTML, testo semplice e testo senza citazioni (sez. 3, 6, 12).
//!
//! L'HTML pulito si mostra comunque in un iframe isolato (sandbox, senza script, CSP restrittiva):
//! la pulizia toglie script, gestori di eventi e schemi pericolosi, ma lascia stili e tabelle
//! perché le email sono fatte così.

use std::collections::HashSet;
use std::sync::LazyLock;

/// Da incrementare quando cambia la pulizia: permette di rielaborare i corpi già salvati.
pub const SANITIZER_VERSION: i64 = 1;

static CLEANER: LazyLock<ammonia::Builder<'static>> = LazyLock::new(|| {
    let mut b = ammonia::Builder::default();
    b.rm_clean_content_tags(&["style"])
        .add_tags(&["style", "font", "center"])
        .add_generic_attributes(&[
            "style", "class", "id", "align", "valign", "width", "height", "bgcolor", "border", "dir", "lang", "title",
        ])
        .add_tag_attributes("table", &["cellpadding", "cellspacing", "background", "role"])
        .add_tag_attributes("td", &["colspan", "rowspan", "nowrap", "background"])
        .add_tag_attributes("th", &["colspan", "rowspan", "nowrap", "background"])
        .add_tag_attributes("font", &["color", "face", "size"])
        .add_tag_attributes("img", &["src", "alt", "width", "height"])
        .add_tag_attributes("a", &["href", "name", "target"])
        .url_schemes(HashSet::from(["http", "https", "mailto", "tel", "cid"]))
        .link_rel(Some("noopener noreferrer"))
        .strip_comments(true);
    b
});

pub fn clean_html(html: &str) -> String {
    CLEANER.clean(html).to_string()
}

pub fn html_to_text(html: &str) -> String {
    html2text::config::plain_no_decorate()
        .string_from_read(html.as_bytes(), 200)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// Toglie citazioni, intestazioni di risposta e firma. Euristico: basta per ricerca e AI.
pub fn strip_quotes(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t == "--" || t == "-- " || line.starts_with("-- ") && t.len() == 2 {
            break; // delimitatore di firma
        }
        if is_reply_header(t, lines.get(i + 1).map(|l| l.trim())) || is_forward_block(&lines[i..]) {
            break;
        }
        if t.starts_with('>') {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}

fn is_reply_header(line: &str, next: Option<&str>) -> bool {
    let l = line.to_lowercase();
    let ends_reply = |s: &str| {
        s.ends_with("wrote:")
            || s.ends_with("ha scritto:")
            || s.ends_with("a écrit :")
            || s.ends_with("a écrit:")
            || s.ends_with("schrieb:")
            || s.ends_with("escribió:")
    };
    let starts_reply = l.starts_with("on ") || l.starts_with("il ") || l.starts_with("le ") || l.starts_with("am ") || l.starts_with("el ");
    if starts_reply && ends_reply(&l) {
        return true;
    }
    // Gmail spezza a volte la riga: "On ..., Name <a@b>\nwrote:"
    if starts_reply && let Some(n) = next {
        let joined = format!("{l} {}", n.to_lowercase());
        if ends_reply(&joined) && n.len() < 20 {
            return true;
        }
    }
    l.starts_with("-----original message-----")
        || l.starts_with("----- original message -----")
        || l.starts_with("-----messaggio originale-----")
        || l.starts_with("----- messaggio originale -----")
        || l.starts_with("---------- forwarded message")
        || l.starts_with("---------- messaggio inoltrato")
}

/// Blocco in stile Outlook: "Da: ... / Inviato: ..." oppure "From: ... / Sent: ...".
fn is_forward_block(rest: &[&str]) -> bool {
    let first = rest[0].trim().trim_start_matches('_').trim().to_lowercase();
    let start = if first.is_empty() && rest.len() > 1 && rest[0].trim().starts_with("____") {
        1
    } else {
        0
    };
    let head = rest.get(start).map(|l| l.trim().to_lowercase()).unwrap_or_default();
    if !(head.starts_with("from:") || head.starts_with("da:") || head.starts_with("de :") || head.starts_with("von:")) {
        return false;
    }
    rest.iter().skip(start + 1).take(4).any(|l| {
        let l = l.trim().to_lowercase();
        l.starts_with("sent:") || l.starts_with("inviato:") || l.starts_with("date:") || l.starts_with("data:") || l.starts_with("envoyé :")
    })
}

/// Entità HTML negli snippet di Gmail (`&#39;`, `&amp;`, ...).
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        // Si cerca il ';' nei primi 12 byte senza tagliare la stringa: il dodicesimo byte può
        // cadere dentro un carattere (es. "&»..."). ';' è ASCII, quindi la sua posizione è un
        // confine valido.
        let end = rest.bytes().take(12).position(|b| b == b';');
        let decoded = end.and_then(|e| {
            let ent = &rest[1..e];
            let ch = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
                _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            ch.map(|c| (c, e + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_scripts_and_handlers_but_keeps_styles() {
        let dirty = r#"<html><head><style>p{color:red}</style><script>alert(1)</script></head>
<body><p style="color:blue" onclick="x()">Ciao</p><a href="javascript:alert(1)">x</a>
<img src="https://t.example/p.gif" onerror="y()"><img src="cid:logo@x"><iframe src="https://e.vil"></iframe></body></html>"#;
        let clean = clean_html(dirty);
        assert!(clean.contains("<style>p{color:red}</style>"), "{clean}");
        assert!(clean.contains(r#"style="color:blue""#));
        assert!(!clean.contains("script"));
        assert!(!clean.contains("onclick"));
        assert!(!clean.contains("onerror"));
        assert!(!clean.contains("javascript"));
        assert!(!clean.contains("iframe"));
        assert!(clean.contains(r#"src="cid:logo@x""#));
    }

    #[test]
    fn style_cannot_break_out() {
        let clean = clean_html("<style>a{}</style><script>x</script><style>b{}</style>");
        assert!(!clean.contains("<script"), "{clean}");
    }

    #[test]
    fn strips_italian_and_english_quotes() {
        let t = "Va bene, confermo.\n\nIl giorno lun 3 nov 2025 alle 10:00 Mario <m@x.it> ha scritto:\n> Confermi?\n";
        assert_eq!(strip_quotes(t), "Va bene, confermo.");
        let t = "Sure.\nOn Mon, Nov 3, 2025 at 10:00 AM Mario Rossi <m@x.it>\nwrote:\n> ok?";
        assert_eq!(strip_quotes(t), "Sure.");
        let t = "Inoltro.\n________________________________\nDa: Anna\nInviato: lunedì\nA: Io\n\nTesto";
        assert_eq!(strip_quotes(t), "Inoltro.");
        let t = "Ciao\n> citazione\nrisposta inline\n-- \nTommaso\nEsempio";
        assert_eq!(strip_quotes(t), "Ciao\nrisposta inline");
    }

    #[test]
    fn decodes_snippet_entities() {
        assert_eq!(decode_entities("l&#39;offerta &amp; il &quot;prezzo&quot; &#x2014; &foo"), "l'offerta & il \"prezzo\" — &foo");
        // Il dodicesimo byte dopo '&' cade dentro un carattere: non deve andare in panic.
        assert_eq!(decode_entities("A&B «xx» &ègrave; &amp;"), "A&B «xx» &ègrave; &");
        assert_eq!(decode_entities("&»»»»»»»»"), "&»»»»»»»»");
    }

    #[test]
    fn html_to_text_works() {
        let t = html_to_text("<p>Ciao <b>Tommaso</b></p><p>Riga due</p>");
        assert!(t.contains("Ciao Tommaso"), "{t}");
        assert!(t.contains("Riga due"));
    }
}
