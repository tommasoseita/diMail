//! Dal formato `full` (o `metadata`) di Gmail a un messaggio pronto per SQLite.
//! Il parsing si fa una sola volta, alla sync (sez. 3).

use serde::{Deserialize, Serialize};

use crate::gmail::{Message, MessagePart};
use crate::sanitize::decode_entities;
use crate::util::{b64url_decode, parse_i64};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Addr {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub email: String,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedBody {
    pub text_plain: Option<String>,
    pub html: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AttachmentMeta {
    pub part_id: String,
    pub gmail_attachment_id: Option<String>,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub content_id: Option<String>,
    pub is_inline: bool,
}

#[derive(Debug, Clone)]
pub struct ParsedMessage {
    pub id: String,
    pub thread_id: String,
    pub history_id: Option<i64>,
    pub internal_date: i64,
    pub label_ids: Vec<String>,
    pub snippet: String,
    pub size_estimate: Option<i64>,
    pub from: Option<Addr>,
    pub to: Vec<Addr>,
    pub cc: Vec<Addr>,
    pub bcc: Vec<Addr>,
    pub reply_to: Vec<Addr>,
    pub subject: String,
    pub rfc_message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub list_unsubscribe: Option<String>,
    pub list_id: Option<String>,
    pub precedence: Option<String>,
    pub auto_submitted: Option<String>,
    /// `None` se il messaggio è arrivato in formato `metadata`.
    pub body: Option<ParsedBody>,
    pub attachments: Vec<AttachmentMeta>,
}

impl ParsedMessage {
    pub fn has_label(&self, id: &str) -> bool {
        self.label_ids.iter().any(|l| l == id)
    }
}

pub fn parse_message(m: &Message, with_body: bool) -> ParsedMessage {
    let empty = MessagePart::default();
    let payload = m.payload.as_ref().unwrap_or(&empty);
    let h = |name: &str| payload.header(name).map(|v| decode_header(v).trim().to_owned()).filter(|v| !v.is_empty());
    let addrs = |name: &str| payload.header(name).map(parse_addrs).unwrap_or_default();

    let (body, attachments) = if with_body {
        let mut ex = Extract::default();
        walk(payload, &mut ex);
        let join = |v: Vec<String>, sep: &str| (!v.is_empty()).then(|| v.join(sep));
        (
            Some(ParsedBody { text_plain: join(ex.plain, "\n\n"), html: join(ex.html, "<hr>") }),
            ex.attachments,
        )
    } else {
        (None, Vec::new())
    };

    ParsedMessage {
        id: m.id.clone(),
        thread_id: m.thread_id.clone(),
        history_id: parse_i64(m.history_id.as_deref()),
        internal_date: parse_i64(m.internal_date.as_deref()).unwrap_or(0),
        label_ids: m.label_ids.clone(),
        snippet: decode_entities(&m.snippet),
        size_estimate: m.size_estimate,
        from: addrs("From").into_iter().next(),
        to: addrs("To"),
        cc: addrs("Cc"),
        bcc: addrs("Bcc"),
        reply_to: addrs("Reply-To"),
        subject: h("Subject").unwrap_or_default(),
        rfc_message_id: h("Message-ID"),
        in_reply_to: h("In-Reply-To"),
        references: h("References"),
        list_unsubscribe: h("List-Unsubscribe"),
        list_id: h("List-Id"),
        precedence: h("Precedence"),
        auto_submitted: h("Auto-Submitted"),
        body,
        attachments,
    }
}

/// Decodifica le parole codificate RFC 2047 (`=?UTF-8?B?...?=`) se presenti.
pub fn decode_header(v: &str) -> String {
    if !v.contains("=?") {
        return v.to_owned();
    }
    match mailparse::parse_header(format!("X: {v}").as_bytes()) {
        Ok((hdr, _)) => hdr.get_value(),
        Err(_) => v.to_owned(),
    }
}

pub fn parse_addrs(v: &str) -> Vec<Addr> {
    let raw = format!("X: {v}");
    let parsed = mailparse::parse_header(raw.as_bytes())
        .ok()
        .and_then(|(hdr, _)| mailparse::addrparse_header(&hdr).ok());
    let mut out = Vec::new();
    if let Some(list) = parsed {
        for a in list.iter() {
            match a {
                mailparse::MailAddr::Single(s) => push_addr(&mut out, s.display_name.as_deref(), &s.addr),
                mailparse::MailAddr::Group(g) => {
                    for s in &g.addrs {
                        push_addr(&mut out, s.display_name.as_deref(), &s.addr);
                    }
                }
            }
        }
    }
    if out.is_empty() {
        // Header malformato: prendiamo almeno gli indirizzi.
        for token in v.split([',', ';', ' ', '<', '>']) {
            if token.contains('@') {
                push_addr(&mut out, None, token);
            }
        }
    }
    out
}

fn push_addr(out: &mut Vec<Addr>, name: Option<&str>, email: &str) {
    let email = email.trim().trim_matches(['<', '>', '"', '\'']).to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return;
    }
    let name = name
        .map(|n| n.trim().trim_matches(['"', '\'']).trim().to_owned())
        .filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case(&email));
    out.push(Addr { name, email });
}

#[derive(Default)]
struct Extract {
    plain: Vec<String>,
    html: Vec<String>,
    attachments: Vec<AttachmentMeta>,
}

fn walk(part: &MessagePart, ex: &mut Extract) {
    let mime = part.mime_type.as_deref().unwrap_or("").to_ascii_lowercase();
    let filename = part.filename.as_deref().unwrap_or("").trim();
    let disposition = part.header("Content-Disposition").unwrap_or("").trim().to_ascii_lowercase();

    if mime.starts_with("multipart/") || (mime == "message/rfc822" && filename.is_empty() && !part.parts.is_empty()) {
        for p in &part.parts {
            walk(p, ex);
        }
        return;
    }

    let body = part.body.as_ref();
    let is_text = mime == "text/plain" || mime == "text/html";
    let is_attachment = !filename.is_empty() || disposition.starts_with("attachment");

    if is_text && !is_attachment {
        if let Some(data) = body.and_then(|b| b.data.as_deref())
            && let Ok(bytes) = b64url_decode(data)
        {
            let text = decode_text(&bytes, part.header("Content-Type"));
            if mime == "text/plain" { ex.plain.push(text) } else { ex.html.push(text) }
        }
        return;
    }

    let Some(b) = body else { return };
    if b.attachment_id.is_none() && b.size.unwrap_or(0) == 0 {
        return;
    }
    let content_id = part
        .header("Content-ID")
        .or_else(|| part.header("X-Attachment-Id"))
        .map(|v| v.trim().trim_matches(['<', '>']).to_owned())
        .filter(|v| !v.is_empty());
    ex.attachments.push(AttachmentMeta {
        part_id: part.part_id.clone().unwrap_or_else(|| ex.attachments.len().to_string()),
        gmail_attachment_id: b.attachment_id.clone(),
        filename: decode_header(filename),
        mime_type: mime,
        size: b.size.unwrap_or(0),
        is_inline: disposition.starts_with("inline") && content_id.is_some(),
        content_id,
    });
}

/// Gmail di solito consegna il testo già in UTF-8; se non lo è, si usa il charset dichiarato.
fn decode_text(bytes: &[u8], content_type: Option<&str>) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    let label = content_type.and_then(|ct| {
        ct.split(';').find_map(|p| {
            let (k, v) = p.split_once('=')?;
            k.trim().eq_ignore_ascii_case("charset").then(|| v.trim().trim_matches('"').to_owned())
        })
    });
    let enc = label
        .and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()))
        .unwrap_or(encoding_rs::WINDOWS_1252);
    enc.decode(bytes).0.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::b64url;

    fn part(json: serde_json::Value) -> MessagePart {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn parses_addresses_with_quotes_and_encoded_words() {
        let a = parse_addrs(r#""Rossi, Mario" <Mario.Rossi@Example.com>, anna@esempio.it"#);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].name.as_deref(), Some("Rossi, Mario"));
        assert_eq!(a[0].email, "mario.rossi@example.com");
        assert_eq!(a[1], Addr { name: None, email: "anna@esempio.it".into() });

        let b = parse_addrs("=?UTF-8?B?TmljY29sw7I=?= <n@x.it>");
        assert_eq!(b[0].name.as_deref(), Some("Niccolò"));
        assert_eq!(decode_header("=?utf-8?q?Caff=C3=A8?="), "Caffè");
    }

    #[test]
    fn extracts_alternative_bodies_and_attachments() {
        let plain = b64url("Ciao Tommaso".as_bytes());
        let html = b64url("<p>Ciao <b>Tommaso</b></p>".as_bytes());
        let latin1 = b64url(&[0x63, 0x61, 0x66, 0x66, 0xe8]); // "caffè" in ISO-8859-1
        let p = part(serde_json::json!({
            "mimeType": "multipart/mixed",
            "headers": [{"name": "Subject", "value": "Preventivo"}, {"name": "From", "value": "Anna <anna@esempio.it>"}],
            "parts": [
                {"mimeType": "multipart/alternative", "parts": [
                    {"partId": "0.0", "mimeType": "text/plain", "body": {"data": plain, "size": 12}},
                    {"partId": "0.1", "mimeType": "text/html", "body": {"data": html, "size": 27}}
                ]},
                {"partId": "1", "mimeType": "text/plain", "headers": [{"name": "Content-Type", "value": "text/plain; charset=iso-8859-1"}], "body": {"data": latin1, "size": 5}},
                {"partId": "2", "mimeType": "application/pdf", "filename": "offerta.pdf",
                 "headers": [{"name": "Content-Disposition", "value": "attachment; filename=offerta.pdf"}],
                 "body": {"attachmentId": "ATT1", "size": 1000}},
                {"partId": "3", "mimeType": "image/png", "filename": "logo.png",
                 "headers": [{"name": "Content-Disposition", "value": "inline"}, {"name": "Content-ID", "value": "<logo@x>"}],
                 "body": {"attachmentId": "ATT2", "size": 50}}
            ]
        }));
        let m = Message {
            id: "m1".into(),
            thread_id: "t1".into(),
            label_ids: vec!["INBOX".into()],
            snippet: "Ciao &amp; grazie".into(),
            history_id: Some("10".into()),
            internal_date: Some("1700000000000".into()),
            payload: Some(p),
            size_estimate: None,
        };
        let pm = parse_message(&m, true);
        assert_eq!(pm.subject, "Preventivo");
        assert_eq!(pm.snippet, "Ciao & grazie");
        assert_eq!(pm.from.as_ref().unwrap().email, "anna@esempio.it");
        let body = pm.body.unwrap();
        assert_eq!(body.text_plain.as_deref(), Some("Ciao Tommaso\n\ncaffè"));
        assert_eq!(body.html.as_deref(), Some("<p>Ciao <b>Tommaso</b></p>"));
        assert_eq!(pm.attachments.len(), 2);
        assert_eq!(pm.attachments[0].filename, "offerta.pdf");
        assert!(!pm.attachments[0].is_inline);
        assert!(pm.attachments[1].is_inline);
        assert_eq!(pm.attachments[1].content_id.as_deref(), Some("logo@x"));
    }

    #[test]
    fn metadata_format_has_no_body() {
        let m = Message {
            id: "m1".into(),
            thread_id: "t1".into(),
            label_ids: vec![],
            snippet: String::new(),
            history_id: None,
            internal_date: Some("5".into()),
            payload: Some(part(serde_json::json!({"headers": [{"name": "Subject", "value": "x"}]}))),
            size_estimate: None,
        };
        let pm = parse_message(&m, false);
        assert!(pm.body.is_none());
        assert_eq!(pm.internal_date, 5);
    }
}
