//! Composer di base (sez. 10 della spec, anticipato il 9 novembre): nuove email, risposte, risposte a
//! tutti, inoltri. Bozze su Gmail e snippet arrivano dopo.
//!
//! Il messaggio parte in testo e in HTML (multipart/alternative), come da Gmail web: nell'HTML la firma
//! di Gmail (letta con `settings.sendAs`) e il testo citato in un blocco `gmail_quote`, che i client
//! di posta piegano. Gli allegati (file del Mac, o negli inoltri quelli del messaggio originale) vanno
//! in un multipart/mixed, fino a 25 MB in tutto come su Gmail (9 novembre, sera).
//!
//! Annulla invio: il messaggio resta in una coda persistente per qualche secondo (10 di default).
//! Mai due invii: il Message-ID si genera qui, e prima di ripetere un invio fallito si controlla
//! negli Inviati che non sia già partito.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::events::CoreEvent;
use crate::gmail::{GmailApi, SendAs};
use crate::mime::Addr;
use crate::sanitize::{clean_html, html_to_text};
use crate::store;
use crate::sync::Ctx;
use crate::util::{b64url, backoff_ms, now_ms, random_bytes};
use crate::{Error, Result};

/// Il tempo per annullare un invio (30 secondi dal 9 novembre, sera; prima 10).
pub const DEFAULT_DELAY_MS: i64 = 30_000;

/// Il limite di Gmail per gli allegati di un'email.
pub const MAX_ATTACHMENTS_BYTES: usize = 25 * 1024 * 1024;

/// Un allegato dell'email che scrivo: un file del Mac (`path`) o, negli inoltri, un allegato del
/// messaggio originale (`message_id` e `part_id`), scaricato da Gmail all'invio se non è già in cache.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OutAttachment {
    pub name: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub part_id: Option<String>,
}

/// Il contenuto di un allegato, pronto per il messaggio.
pub struct FilePart {
    pub name: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

/// La firma impostata su Gmail web, in HTML (ripulito) e in testo.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Signature {
    pub html: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Draft {
    #[serde(default)]
    pub to: Vec<Addr>,
    #[serde(default)]
    pub cc: Vec<Addr>,
    #[serde(default)]
    pub bcc: Vec<Addr>,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body: String,
    /// Per le risposte: il thread Gmail in cui resta il messaggio.
    pub thread_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    /// Testo citato (risposte) o inoltrato, aggiunto sotto il corpo all'invio.
    pub quoted: Option<String>,
    #[serde(default)]
    pub attachments: Vec<OutAttachment>,
    /// Firma di Gmail tolta nel composer, per questa email.
    #[serde(default)]
    pub no_signature: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplyMode {
    Reply,
    ReplyAll,
    Forward,
}

// ───────────────────────────── messaggio RFC 2822 ─────────────────────────────

fn needs_quotes(s: &str) -> bool {
    s.chars().any(|c| "()<>[]:;@\\,.\"".contains(c))
}

/// Nome in un header: ASCII semplice così com'è, con punteggiatura tra virgolette, altrimenti RFC 2047.
fn encode_phrase(s: &str) -> String {
    if s.is_ascii() {
        if needs_quotes(s) { format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")) } else { s.to_owned() }
    } else {
        encode_words(s)
    }
}

/// RFC 2047 in base64, a pezzi di al massimo 45 byte (le parole codificate stanno sotto i 75 caratteri).
fn encode_words(s: &str) -> String {
    let mut words = Vec::new();
    let mut chunk = String::new();
    for ch in s.chars() {
        if chunk.len() + ch.len_utf8() > 45 {
            words.push(format!("=?UTF-8?B?{}?=", STANDARD.encode(chunk.as_bytes())));
            chunk.clear();
        }
        chunk.push(ch);
    }
    if !chunk.is_empty() {
        words.push(format!("=?UTF-8?B?{}?=", STANDARD.encode(chunk.as_bytes())));
    }
    words.join("\r\n ")
}

fn header_addr(a: &Addr) -> String {
    match a.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => format!("{} <{}>", encode_phrase(n), a.email),
        None => a.email.clone(),
    }
}

fn header_addrs(list: &[Addr]) -> String {
    list.iter().map(header_addr).collect::<Vec<_>>().join(", ")
}

/// Un Message-ID nostro, generato prima dell'invio.
pub fn new_message_id(from: &str) -> String {
    let domain = from.rsplit_once('@').map(|(_, d)| d).unwrap_or("dimail.local");
    format!("<{}.dimail@{domain}>", b64url(&random_bytes::<15>()))
}

/// Il corpo in testo semplice: quello che ho scritto, la firma (dopo il separatore "-- ") e, per
/// risposte e inoltri, il testo citato.
pub fn full_body(d: &Draft, sig: Option<&Signature>) -> String {
    let mut out = plain_links(d.body.trim_end());
    if let Some(s) = sig.filter(|s| !s.text.trim().is_empty()) {
        out.push_str("\n\n-- \n");
        out.push_str(s.text.trim());
    }
    if let Some(q) = d.quoted.as_deref().filter(|q| !q.trim().is_empty()) {
        out.push_str("\n\n");
        out.push_str(q.trim_end());
    }
    out.push('\n');
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// I link fatti nel composer (testo selezionato e ⌘V con un URL negli appunti): nella bozza sono
/// `[testo](url)`, con l'URL che comincia per http://, https:// o mailto: e le parentesi codificate.
/// Restituisce i pezzi della riga: testo semplice (`None`) o link (`Some(url)`).
pub fn split_links(line: &str) -> Vec<(&str, Option<&str>)> {
    let mut out = Vec::new();
    let mut plain_from = 0;
    let mut i = 0;
    while let Some(open) = line[i..].find('[').map(|o| o + i) {
        let link = line[open + 1..].find("](").map(|c| open + 1 + c).and_then(|close| {
            let text = &line[open + 1..close];
            let url_from = close + 2;
            let url_to = url_from + line[url_from..].find(')')?;
            let url = &line[url_from..url_to];
            let ok_url = ["https://", "http://", "mailto:"].iter().any(|p| url.starts_with(p) && url.len() > p.len())
                && !url.chars().any(char::is_whitespace);
            (ok_url && !text.trim().is_empty() && !text.contains(['[', ']'])).then_some((text, url, url_to + 1))
        });
        match link {
            Some((text, url, next)) => {
                if open > plain_from {
                    out.push((&line[plain_from..open], None));
                }
                out.push((text, Some(url)));
                plain_from = next;
                i = next;
            }
            None => i = open + 1,
        }
    }
    if plain_from < line.len() {
        out.push((&line[plain_from..], None));
    }
    out
}

/// Il testo semplice dell'email: i link del composer come "testo <url>", come fa Gmail.
fn plain_links(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            split_links(line)
                .into_iter()
                .map(|(t, url)| match url {
                    Some(u) => format!("{t} <{u}>"),
                    None => t.to_owned(),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Una riga di testo in HTML: i link del composer con il loro testo, gli URL scritti per intero
/// cliccabili (la punteggiatura finale resta fuori dal link).
fn line_html(line: &str) -> String {
    split_links(line)
        .into_iter()
        .map(|(text, url)| match url {
            Some(u) => format!("<a href=\"{}\">{}</a>", esc(u), esc(text)),
            None => autolink_html(text),
        })
        .collect()
}

fn autolink_html(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = [rest.find("https://"), rest.find("http://")].into_iter().flatten().min() {
        out.push_str(&esc(&rest[..i]));
        let tail = &rest[i..];
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"')).unwrap_or(tail.len());
        let mut url = &tail[..end];
        while let Some(c) = url.chars().last().filter(|c| ".,;:!?)]'".contains(*c)) {
            url = &url[..url.len() - c.len_utf8()];
        }
        out.push_str(&format!("<a href=\"{0}\">{0}</a>", esc(url)));
        rest = &tail[url.len()..];
    }
    out.push_str(&esc(rest));
    out
}

/// Il testo come lo scrive Gmail web: una riga per `<div>`, le righe vuote come `<div><br></div>`.
fn text_html(text: &str) -> String {
    text.lines()
        .map(|l| if l.trim().is_empty() { "<div><br></div>".to_owned() } else { format!("<div>{}</div>", line_html(l)) })
        .collect()
}

/// Il testo citato o inoltrato in un blocco `gmail_quote`: l'intestazione ("Il …, X ha scritto:" o
/// "Messaggio inoltrato") e poi il testo, tra virgolette se è una risposta.
fn quoted_html(q: &str) -> String {
    let lines: Vec<&str> = q.trim_end().lines().collect();
    let head = lines.iter().position(|l| l.starts_with('>') || l.trim().is_empty()).unwrap_or(lines.len());
    let mut out = String::from("<div class=\"gmail_quote\">");
    if head > 0 {
        out.push_str("<div dir=\"ltr\" class=\"gmail_attr\">");
        out.push_str(&lines[..head].iter().map(|l| line_html(l)).collect::<Vec<_>>().join("<br>"));
        out.push_str("<br></div>");
    }
    let rest = &lines[head..];
    if rest.iter().any(|l| l.starts_with('>')) {
        let inner = rest
            .iter()
            .map(|l| line_html(l.strip_prefix("> ").or_else(|| l.strip_prefix('>')).unwrap_or(l)))
            .collect::<Vec<_>>()
            .join("<br>");
        out.push_str(&format!(
            "<blockquote class=\"gmail_quote\" style=\"margin:0px 0px 0px 0.8ex;border-left:1px solid rgb(204,204,204);padding-left:1ex\">{inner}</blockquote>"
        ));
    } else {
        out.push_str(&rest.iter().map(|l| line_html(l)).collect::<Vec<_>>().join("<br>"));
    }
    out.push_str("</div>");
    out
}

/// Il corpo in HTML: il testo, la firma di Gmail e il testo citato.
pub fn html_body(d: &Draft, sig: Option<&Signature>) -> String {
    let mut out = format!("<div dir=\"ltr\">{}", text_html(d.body.trim_end()));
    if let Some(s) = sig.filter(|s| !s.html.trim().is_empty()) {
        out.push_str(&format!("<br><div dir=\"ltr\" class=\"gmail_signature\" data-smartmail=\"gmail_signature\">{}</div>", s.html));
    }
    out.push_str("</div>");
    if let Some(q) = d.quoted.as_deref().filter(|q| !q.trim().is_empty()) {
        out.push_str("<br>");
        out.push_str(&quoted_html(q));
    }
    out
}

fn crlf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\n', "\r\n")
}

fn push_b64(out: &mut String, data: &[u8]) {
    let encoded = STANDARD.encode(data);
    for line in encoded.as_bytes().chunks(76) {
        out.push_str(std::str::from_utf8(line).unwrap_or_default());
        out.push_str("\r\n");
    }
}

/// Il nome di un allegato in `name=` e `filename=`: tra virgolette, codificato RFC 2047 se non è
/// ASCII (come fa Gmail web).
fn file_param(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c == '"' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    if clean.is_ascii() { format!("\"{clean}\"") } else { format!("\"=?UTF-8?B?{}?=\"", STANDARD.encode(clean.as_bytes())) }
}

fn boundary(tag: &str) -> String {
    format!("{tag}-{}", b64url(&random_bytes::<12>()))
}

/// Il messaggio RFC 2822 in base64url, come lo vuole `messages.send`.
pub fn build_raw(d: &Draft, from: &Addr, message_id: &str, date: &str, sig: Option<&Signature>, files: &[FilePart]) -> String {
    let mut h = String::new();
    h.push_str(&format!("From: {}\r\n", header_addr(from)));
    if !d.to.is_empty() {
        h.push_str(&format!("To: {}\r\n", header_addrs(&d.to)));
    }
    if !d.cc.is_empty() {
        h.push_str(&format!("Cc: {}\r\n", header_addrs(&d.cc)));
    }
    if !d.bcc.is_empty() {
        h.push_str(&format!("Bcc: {}\r\n", header_addrs(&d.bcc)));
    }
    let subject = if d.subject.is_ascii() { d.subject.clone() } else { encode_words(&d.subject) };
    h.push_str(&format!("Subject: {subject}\r\n"));
    h.push_str(&format!("Date: {date}\r\n"));
    h.push_str(&format!("Message-ID: {message_id}\r\n"));
    if let Some(r) = d.in_reply_to.as_deref().filter(|r| !r.is_empty()) {
        h.push_str(&format!("In-Reply-To: {r}\r\n"));
    }
    if let Some(r) = d.references.as_deref().filter(|r| !r.is_empty()) {
        h.push_str(&format!("References: {r}\r\n"));
    }
    h.push_str("MIME-Version: 1.0\r\n");

    let alt = boundary("alt");
    let mut body = String::new();
    body.push_str(&format!("--{alt}\r\nContent-Type: text/plain; charset=\"UTF-8\"\r\nContent-Transfer-Encoding: base64\r\n\r\n"));
    push_b64(&mut body, crlf(&full_body(d, sig)).as_bytes());
    body.push_str(&format!("--{alt}\r\nContent-Type: text/html; charset=\"UTF-8\"\r\nContent-Transfer-Encoding: base64\r\n\r\n"));
    push_b64(&mut body, html_body(d, sig).as_bytes());
    body.push_str(&format!("--{alt}--\r\n"));

    if files.is_empty() {
        h.push_str(&format!("Content-Type: multipart/alternative; boundary=\"{alt}\"\r\n\r\n"));
        h.push_str(&body);
    } else {
        let mix = boundary("mix");
        h.push_str(&format!("Content-Type: multipart/mixed; boundary=\"{mix}\"\r\n\r\n"));
        h.push_str(&format!("--{mix}\r\nContent-Type: multipart/alternative; boundary=\"{alt}\"\r\n\r\n"));
        h.push_str(&body);
        for f in files {
            let mime: String = f.mime_type.trim().chars().take_while(|c| c.is_ascii_alphanumeric() || "./+-".contains(*c)).collect();
            let mime = if mime.contains('/') { mime } else { "application/octet-stream".into() };
            let name = file_param(&f.name);
            h.push_str(&format!(
                "--{mix}\r\nContent-Type: {mime}; name={name}\r\nContent-Disposition: attachment; filename={name}\r\nContent-Transfer-Encoding: base64\r\n\r\n"
            ));
            push_b64(&mut h, &f.data);
        }
        h.push_str(&format!("--{mix}--\r\n"));
    }
    b64url(h.as_bytes())
}

// ───────────────────────────── firma di Gmail ─────────────────────────────

const SIGNATURE_KEY: &str = "compose.signature";
/// La firma si rilegge da Gmail al massimo una volta l'ora (aprendo il composer).
const SIGNATURE_TTL_MS: i64 = 3_600_000;

#[derive(Serialize, Deserialize)]
struct CachedSignature {
    html: String,
    text: String,
    fetched_at: i64,
}

/// La firma salvata e quando è stata letta.
pub fn cached_signature(c: &Connection) -> Result<Option<(Signature, i64)>> {
    Ok(store::get_setting(c, SIGNATURE_KEY)?
        .and_then(|v| serde_json::from_value::<CachedSignature>(v).ok())
        .map(|s| (Signature { html: s.html, text: s.text }, s.fetched_at)))
}

/// La firma del mio indirizzo tra quelli "invia come".
pub fn signature_from(list: &[SendAs], me: &str) -> Signature {
    let mine = list.iter().find(|s| s.send_as_email.eq_ignore_ascii_case(me)).or_else(|| list.iter().find(|s| s.is_primary));
    let html = mine.and_then(|s| s.signature.as_deref()).map(clean_html).unwrap_or_default();
    let text = if html.trim().is_empty() { String::new() } else { html_to_text(&html) };
    Signature { html, text }
}

fn non_empty(s: Signature) -> Option<Signature> {
    (!s.html.trim().is_empty()).then_some(s)
}

/// La firma da mettere nelle email (nessuna se su Gmail non c'è): dalla cache, o da Gmail se è
/// vecchia o se `fresh`. Se Gmail non risponde vale quella salvata.
pub async fn signature<G: GmailApi>(ctx: &Ctx<G>, fresh: bool) -> Result<Option<Signature>> {
    let cached = ctx.db.read_async(cached_signature).await?;
    if let Some((s, at)) = &cached
        && !fresh
        && now_ms() - at < SIGNATURE_TTL_MS
    {
        return Ok(non_empty(s.clone()));
    }
    match ctx.gmail.send_as().await {
        Ok(list) => {
            let sig = signature_from(&list, &ctx.me);
            let v = serde_json::to_value(CachedSignature { html: sig.html.clone(), text: sig.text.clone(), fetched_at: now_ms() })?;
            ctx.db.write_async(move |c| store::set_setting(c, SIGNATURE_KEY, &v)).await?;
            Ok(non_empty(sig))
        }
        Err(e) => {
            tracing::warn!(error = %e, "firma di Gmail non letta");
            Ok(cached.and_then(|(s, _)| non_empty(s)))
        }
    }
}

// ───────────────────────────── risposte e inoltri ─────────────────────────────

struct Src {
    rowid: i64,
    from: Option<Addr>,
    to: Vec<Addr>,
    cc: Vec<Addr>,
    reply_to: Vec<Addr>,
    date: i64,
    subject: String,
    rfc_id: Option<String>,
    references: Option<String>,
    text: String,
    message_id: String,
}

fn strip_prefix_ci<'a>(s: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    let lower = s.to_lowercase();
    prefixes.iter().find(|p| lower.starts_with(*p)).map(|p| s[p.len()..].trim_start())
}

fn date_it(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|d| d.format("%d/%m/%Y %H:%M").to_string())
        .unwrap_or_default()
}

/// Una bozza di risposta, risposta a tutti o inoltro sull'ultimo messaggio del thread.
pub fn reply_draft(c: &Connection, thread_id: &str, mode: ReplyMode, me: &str) -> Result<Draft> {
    let parse = |s: Option<String>| -> Vec<Addr> { s.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default() };
    let src: Src = c
        .query_row(
            "SELECT m.from_email, m.from_name, m.to_json, m.cc_json, m.reply_to_json, m.internal_date, m.subject,
                    m.rfc_message_id, m.references_hdr, coalesce(b.text_plain, m.snippet), m.rowid, m.id
             FROM messages m LEFT JOIN message_bodies b ON b.message_rowid = m.rowid
             WHERE m.thread_id = ?1 AND m.label_ids NOT LIKE '%\"DRAFT\"%'
             ORDER BY m.internal_date DESC, m.rowid DESC LIMIT 1",
            [thread_id],
            |r| {
                let email: Option<String> = r.get(0)?;
                Ok(Src {
                    rowid: r.get(10)?,
                    message_id: r.get(11)?,
                    from: email.map(|email| Addr { email, name: r.get(1).ok().flatten() }),
                    to: parse(r.get(2)?),
                    cc: parse(r.get(3)?),
                    reply_to: parse(r.get(4)?),
                    date: r.get(5)?,
                    subject: r.get(6)?,
                    rfc_id: r.get(7)?,
                    references: r.get(8)?,
                    text: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
                })
            },
        )
        .optional()?
        .ok_or(Error::NotFound)?;

    let is_me = |a: &Addr| a.email.eq_ignore_ascii_case(me);
    let from_me = src.from.as_ref().is_some_and(is_me);
    let base = strip_prefix_ci(&src.subject, &["re:", "r:", "fwd:", "fw:", "i:"]).unwrap_or(&src.subject).to_owned();
    let who = src.from.as_ref().map(|a| match &a.name {
        Some(n) => format!("{n} <{}>", a.email),
        None => a.email.clone(),
    });

    if mode == ReplyMode::Forward {
        let mut q = String::from("---------- Messaggio inoltrato ----------\n");
        if let Some(w) = &who {
            q.push_str(&format!("Da: {w}\n"));
        }
        q.push_str(&format!("Data: {}\nOggetto: {}\n", date_it(src.date), src.subject));
        if !src.to.is_empty() {
            q.push_str(&format!("A: {}\n", src.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>().join(", ")));
        }
        q.push('\n');
        q.push_str(src.text.trim());
        // Gli allegati del messaggio inoltrato partono con lui (si possono togliere nel composer).
        let attachments = c
            .prepare(
                "SELECT part_id, filename, mime_type, size FROM attachments
                 WHERE message_rowid = ?1 AND is_inline = 0 AND filename != '' ORDER BY rowid",
            )?
            .query_map([src.rowid], |r| {
                Ok(OutAttachment {
                    part_id: Some(r.get(0)?),
                    name: r.get(1)?,
                    mime_type: r.get(2)?,
                    size: r.get(3)?,
                    message_id: Some(src.message_id.clone()),
                    path: None,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(Draft { subject: format!("Fwd: {base}"), quoted: Some(q), attachments, ..Default::default() });
    }

    // Destinatari: rispondendo a un mio messaggio, gli stessi di prima; altrimenti Reply-To o mittente.
    let mut to: Vec<Addr> = if from_me {
        src.to.clone()
    } else if !src.reply_to.is_empty() {
        src.reply_to.clone()
    } else {
        src.from.clone().into_iter().collect()
    };
    to.retain(|a| !is_me(a));
    let mut cc = Vec::new();
    if mode == ReplyMode::ReplyAll {
        for a in src.to.iter().chain(&src.cc) {
            if !is_me(a) && !to.iter().chain(&cc).any(|x: &Addr| x.email.eq_ignore_ascii_case(&a.email)) {
                cc.push(a.clone());
            }
        }
    }
    let quoted = {
        let mut q = format!("Il {}, {} ha scritto:\n", date_it(src.date), who.unwrap_or_default());
        for l in src.text.trim_end().lines() {
            q.push_str("> ");
            q.push_str(l);
            q.push('\n');
        }
        q
    };
    let references = match (&src.references, &src.rfc_id) {
        (Some(r), Some(id)) => Some(format!("{} {id}", r.trim())),
        (None, Some(id)) => Some(id.clone()),
        (r, None) => r.clone(),
    };
    Ok(Draft {
        to,
        cc,
        bcc: Vec::new(),
        subject: format!("Re: {base}"),
        body: String::new(),
        thread_id: Some(thread_id.to_owned()),
        in_reply_to: src.rfc_id,
        references,
        quoted: Some(quoted),
        ..Default::default()
    })
}

/// Destinatari suggeriti: contatti per prefisso di nome o indirizzo, i più usati prima.
pub fn suggest_contacts(c: &Connection, q: &str, limit: i64) -> Result<Vec<Addr>> {
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let like = format!("{}%", q.replace(['%', '_'], ""));
    let word = format!("% {}%", q.replace(['%', '_'], ""));
    Ok(c
        .prepare(
            "SELECT email, name FROM contacts
             WHERE lower(email) LIKE ?1 OR lower(coalesce(name, '')) LIKE ?1 OR lower(coalesce(name, '')) LIKE ?2
             ORDER BY sent_count * 3 + received_count DESC, coalesce(last_sent_at, last_received_at, 0) DESC LIMIT ?3",
        )?
        .query_map(params![like, word, limit], |r| Ok(Addr { email: r.get(0)?, name: r.get(1)? }))?
        .collect::<Result<_, _>>()?)
}

// ───────────────────────────── coda di invio ─────────────────────────────

/// Mette in coda: parte dopo `delay_ms`, se nel frattempo non si annulla. Il messaggio si compone
/// qui, con la firma e gli allegati già letti.
pub fn enqueue(c: &Connection, d: &Draft, from: &Addr, delay_ms: i64, sig: Option<&Signature>, files: &[FilePart]) -> Result<(i64, i64)> {
    if d.to.is_empty() && d.cc.is_empty() && d.bcc.is_empty() {
        return Err(Error::other("manca il destinatario"));
    }
    let message_id = new_message_id(&from.email);
    let date = chrono::Local::now().to_rfc2822();
    let sig = sig.filter(|_| !d.no_signature);
    let raw = build_raw(d, from, &message_id, &date, sig, files);
    let send_at = now_ms() + delay_ms.max(0);
    c.execute(
        "INSERT INTO send_queue (draft, raw, thread_id, rfc_message_id, send_at, state, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6)",
        params![serde_json::to_string(d)?, raw, d.thread_id, message_id, send_at, now_ms()],
    )?;
    Ok((c.last_insert_rowid(), send_at))
}

/// Annulla un invio non ancora partito. Restituisce la bozza, per riaprirla.
pub fn cancel(c: &Connection, id: i64) -> Result<Option<Draft>> {
    // Il messaggio composto (con gli allegati può pesare decine di MB) non serve più.
    let n = c.execute("UPDATE send_queue SET state = 'cancelled', raw = '' WHERE id = ?1 AND state = 'pending'", [id])?;
    if n == 0 {
        return Ok(None);
    }
    let draft: String = c.query_row("SELECT draft FROM send_queue WHERE id = ?1", [id], |r| r.get(0))?;
    Ok(Some(serde_json::from_str(&draft)?))
}

/// "Invia ora": un invio ancora in attesa parte subito, senza aspettare la fine dell'annulla.
/// Falso se è già partito o è stato annullato.
pub fn send_now(c: &Connection, id: i64) -> Result<bool> {
    Ok(c.execute("UPDATE send_queue SET send_at = min(send_at, ?2) WHERE id = ?1 AND state = 'pending'", params![id, now_ms()])? == 1)
}

struct Pending {
    id: i64,
    raw: String,
    thread_id: Option<String>,
    rfc_id: String,
    attempts: i64,
}

/// Gli invii scaduti passano a `sending` in modo atomico: un invio non parte due volte.
fn claim(c: &mut Connection) -> Result<Vec<Pending>> {
    let tx = c.transaction()?;
    let rows: Vec<Pending> = tx
        .prepare("SELECT id, raw, thread_id, rfc_message_id, attempts FROM send_queue WHERE state = 'pending' AND send_at <= ?1 ORDER BY id")?
        .query_map([now_ms()], |r| Ok(Pending { id: r.get(0)?, raw: r.get(1)?, thread_id: r.get(2)?, rfc_id: r.get(3)?, attempts: r.get(4)? }))?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for p in rows {
        if tx.execute("UPDATE send_queue SET state = 'sending', attempts = attempts + 1 WHERE id = ?1 AND state = 'pending'", [p.id])? == 1 {
            out.push(p);
        }
    }
    tx.commit()?;
    Ok(out)
}

/// Dopo un crash durante l'invio: si riprova, ma controllando prima gli Inviati.
pub fn reset_sending(c: &Connection) -> Result<usize> {
    Ok(c.execute("UPDATE send_queue SET state = 'pending' WHERE state = 'sending'", [])?)
}

fn next_due(c: &Connection) -> Result<Option<i64>> {
    Ok(c.query_row("SELECT min(send_at) FROM send_queue WHERE state = 'pending'", [], |r| r.get(0))?)
}

/// Worker della coda di invio.
pub async fn run<G: GmailApi>(ctx: Arc<Ctx<G>>) {
    if let Err(e) = ctx.db.write_async(|c| reset_sending(c)).await {
        tracing::error!(error = %e, "coda di invio: ripristino fallito");
    }
    loop {
        if ctx.is_auth_required() {
            ctx.send_notify.notified().await;
            continue;
        }
        let due = ctx.db.write_async(|c| claim(c)).await.unwrap_or_else(|e| {
            tracing::error!(error = %e, "coda di invio: lettura fallita");
            Vec::new()
        });
        for p in due {
            let outcome: Result<String> = async {
                // Un tentativo precedente può essere arrivato a Gmail anche se non abbiamo visto la risposta.
                if p.attempts > 0 && ctx.gmail.has_message_id(&p.rfc_id).await? {
                    return Ok(String::new());
                }
                ctx.gmail.send_message(&p.raw, p.thread_id.as_deref()).await
            }
            .await;
            let id = p.id;
            match outcome {
                Ok(gmail_id) => {
                    let _ = ctx
                        .db
                        .write_async(move |c| {
                            c.execute(
                                "UPDATE send_queue SET state = 'sent', sent_at = ?2, gmail_id = nullif(?3, ''), last_error = NULL, raw = '' WHERE id = ?1",
                                params![id, now_ms(), gmail_id],
                            )?;
                            Ok(())
                        })
                        .await;
                    tracing::info!(id, "email inviata");
                    ctx.emit(CoreEvent::SendStatus { id, state: "sent".into(), error: None });
                    ctx.sync_trigger.notify_one();
                }
                Err(e) if e.is_transient() || matches!(e, Error::Network(_) | Error::AuthRevoked) => {
                    ctx.note_error(&e);
                    let wait = backoff_ms((p.attempts + 1) as u32, 120_000) as i64;
                    let msg = e.to_string();
                    let _ = ctx
                        .db
                        .write_async(move |c| {
                            c.execute(
                                "UPDATE send_queue SET state = 'pending', send_at = ?2, last_error = ?3 WHERE id = ?1",
                                params![id, now_ms() + wait, msg],
                            )?;
                            Ok(())
                        })
                        .await;
                    tracing::warn!(id, error = %e, "invio rimandato");
                    ctx.emit(CoreEvent::SendStatus { id, state: "retrying".into(), error: Some(e.to_string()) });
                }
                Err(e) => {
                    let msg = e.to_string();
                    let m2 = msg.clone();
                    let _ = ctx
                        .db
                        .write_async(move |c| {
                            c.execute("UPDATE send_queue SET state = 'failed', last_error = ?2 WHERE id = ?1", params![id, m2])?;
                            Ok(())
                        })
                        .await;
                    tracing::warn!(id, error = %e, "invio non riuscito");
                    ctx.emit(CoreEvent::SendStatus { id, state: "failed".into(), error: Some(msg) });
                }
            }
        }
        let next = ctx.db.read_async(|c| next_due(c)).await.ok().flatten();
        let wait = next.map(|t| Duration::from_millis((t - now_ms()).clamp(50, 60_000) as u64)).unwrap_or(Duration::from_secs(60));
        tokio::select! {
            _ = ctx.send_notify.notified() => {}
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_links_become_anchors_and_plain_text_links() {
        let parts = split_links("Vedi [il preventivo](https://esempio.it/p?a=1&b=2) e [x] (nota) e [mail](mailto:anna@esempio.it).");
        assert_eq!(
            parts,
            vec![
                ("Vedi ", None),
                ("il preventivo", Some("https://esempio.it/p?a=1&b=2")),
                (" e [x] (nota) e ", None),
                ("mail", Some("mailto:anna@esempio.it")),
                (".", None),
            ]
        );
        // Solo http, https e mailto; il testo non può essere vuoto.
        assert_eq!(split_links("[a](javascript:alert(1))"), vec![("[a](javascript:alert(1))", None)]);
        assert_eq!(split_links("[ ](https://e.it)"), vec![("[ ](https://e.it)", None)]);

        let d = Draft { body: "Ecco [il documento](https://esempio.it/doc%28v2%29) e https://esempio.it/x.".into(), ..Default::default() };
        assert!(full_body(&d, None).starts_with("Ecco il documento <https://esempio.it/doc%28v2%29> e https://esempio.it/x.\n"));
        let html = html_body(&d, None);
        assert!(
            html.contains(r#"Ecco <a href="https://esempio.it/doc%28v2%29">il documento</a> e <a href="https://esempio.it/x">https://esempio.it/x</a>."#),
            "{html}"
        );
        let d2 = Draft { body: "[<b>x</b>](https://e.it/?a=1&b=\"2\")".into(), ..Default::default() };
        assert!(html_body(&d2, None).contains(r#"<a href="https://e.it/?a=1&amp;b=&quot;2&quot;">&lt;b&gt;x&lt;/b&gt;</a>"#));
    }
    use crate::db::test_db;
    use crate::store::{self, testdata::thread};

    fn addr(n: &str, e: &str) -> Addr {
        Addr { name: Some(n.into()), email: e.into() }
    }

    fn decode(raw: &str) -> String {
        String::from_utf8(crate::util::b64url_decode(raw).unwrap()).unwrap()
    }

    /// Le parti del messaggio, decodificate: (tipo, nome del file, contenuto).
    fn parts(raw: &str) -> Vec<(String, Option<String>, Vec<u8>)> {
        fn walk(p: &mailparse::ParsedMail, out: &mut Vec<(String, Option<String>, Vec<u8>)>) {
            if p.subparts.is_empty() {
                let name = p.get_content_disposition().params.get("filename").cloned();
                out.push((p.ctype.mimetype.clone(), name, p.get_body_raw().unwrap()));
            }
            for s in &p.subparts {
                walk(s, out);
            }
        }
        let parsed = mailparse::parse_mail(raw.as_bytes()).unwrap();
        let mut out = Vec::new();
        walk(&parsed, &mut out);
        out
    }

    #[test]
    fn builds_a_valid_utf8_message() {
        let d = Draft {
            to: vec![addr("Anna Rossi", "anna@cliente.it"), Addr { name: None, email: "luca@x.it".into() }],
            cc: vec![addr("Rossi, Marco", "marco@x.it")],
            subject: "Preventivo più aggiornato".into(),
            body: "Ciao Anna,\nti mando il preventivo: https://esempio.it/p?a=1&b=2.\n".into(),
            in_reply_to: Some("<a@x>".into()),
            references: Some("<z@x> <a@x>".into()),
            ..Default::default()
        };
        let raw = decode(&build_raw(&d, &addr("Tommaso Seita", "tommaso@esempio.it"), "<id@esempio.it>", "Mon, 9 Nov 2026 10:00:00 +0100", None, &[]));
        assert!(raw.contains("From: Tommaso Seita <tommaso@esempio.it>\r\n"));
        assert!(raw.contains("To: Anna Rossi <anna@cliente.it>, luca@x.it\r\n"));
        assert!(raw.contains("Cc: \"Rossi, Marco\" <marco@x.it>\r\n"));
        assert!(raw.contains("Subject: =?UTF-8?B?"), "oggetto con accenti codificato");
        assert!(raw.contains("In-Reply-To: <a@x>\r\nReferences: <z@x> <a@x>\r\n"));
        // Il parser che usiamo per leggere la posta legge anche quella che scriviamo.
        let parsed = mailparse::parse_mail(raw.as_bytes()).unwrap();
        assert_eq!(parsed.headers.iter().find(|h| h.get_key() == "Subject").unwrap().get_value(), "Preventivo più aggiornato");
        let p = parts(&raw);
        assert_eq!(p.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["text/plain", "text/html"]);
        assert_eq!(String::from_utf8(p[0].2.clone()).unwrap(), "Ciao Anna,\r\nti mando il preventivo: https://esempio.it/p?a=1&b=2.\r\n");
        let html = String::from_utf8(p[1].2.clone()).unwrap();
        assert!(html.contains("<div>Ciao Anna,</div>"));
        assert!(html.contains("<a href=\"https://esempio.it/p?a=1&amp;b=2\">https://esempio.it/p?a=1&amp;b=2</a>.</div>"), "link senza il punto finale: {html}");
    }

    #[test]
    fn signature_quote_and_attachments() {
        let d = Draft {
            to: vec![addr("Anna", "anna@x.it")],
            subject: "Re: Fattura".into(),
            body: "Eccola <ok>".into(),
            quoted: Some("Il 09/11/2026 10:00, Anna <anna@x.it> ha scritto:\n> Mi mandi la fattura?\n> Grazie\n".into()),
            ..Default::default()
        };
        let sig = Signature { html: "<b>Tommaso Seita</b><br>Esempio".into(), text: "Tommaso Seita\nEsempio".into() };
        let files = [
            FilePart { name: "Fattura 20.pdf".into(), mime_type: "application/pdf".into(), data: b"%PDF-1.4 x".to_vec() },
            FilePart { name: "però.txt".into(), mime_type: "text/plain\r\nX-Evil: 1".into(), data: vec![0, 1, 2, 255] },
        ];
        let raw = decode(&build_raw(&d, &addr("Tommaso", "tommaso@esempio.it"), "<id@x>", "Mon, 9 Nov 2026 10:00:00 +0100", Some(&sig), &files));
        assert!(!raw.contains("X-Evil: 1\r\n"), "niente header iniettati dal tipo");
        assert!(raw.contains("filename=\"=?UTF-8?B?"), "nome non ASCII codificato");
        let p = parts(&raw);
        assert_eq!(p.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["text/plain", "text/html", "application/pdf", "text/plain"]);
        let text = String::from_utf8(p[0].2.clone()).unwrap();
        assert!(text.starts_with("Eccola <ok>\r\n\r\n-- \r\nTommaso Seita\r\nEsempio\r\n\r\nIl 09/11/2026"), "{text}");
        let html = String::from_utf8(p[1].2.clone()).unwrap();
        assert!(html.contains("<div>Eccola &lt;ok&gt;</div>"));
        assert!(html.contains("class=\"gmail_signature\" data-smartmail=\"gmail_signature\"><b>Tommaso Seita</b>"));
        assert!(html.contains("class=\"gmail_attr\">Il 09/11/2026 10:00, Anna &lt;anna@x.it&gt; ha scritto:<br></div>"));
        assert!(html.contains("padding-left:1ex\">Mi mandi la fattura?<br>Grazie</blockquote>"));
        assert!(html.find("gmail_signature").unwrap() < html.find("gmail_quote").unwrap(), "la firma sopra il citato");
        assert_eq!(p[2].1.as_deref(), Some("Fattura 20.pdf"));
        assert_eq!(p[2].2, b"%PDF-1.4 x");
        assert_eq!(p[3].2, [0, 1, 2, 255]);

        // Firma tolta nel composer: non parte.
        let (_d, db) = test_db();
        db.write(|c| {
            let (id, _) = enqueue(c, &Draft { no_signature: true, ..d.clone() }, &addr("T", "tommaso@esempio.it"), 0, Some(&sig), &[])?;
            let raw: String = c.query_row("SELECT raw FROM send_queue WHERE id = ?1", [id], |r| r.get(0))?;
            assert!(!decode(&raw).contains("gmail_signature\""));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn signature_of_my_address() {
        let list = [
            SendAs { send_as_email: "alias@esempio.it".into(), signature: Some("alias".into()), ..Default::default() },
            SendAs { send_as_email: "Tommaso@Esempio.it".into(), signature: Some("<p onclick=\"x()\">Tommaso</p><script>x()</script>".into()), ..Default::default() },
        ];
        let s = signature_from(&list, "tommaso@esempio.it");
        assert_eq!(s.html, "<p>Tommaso</p>", "ripulita");
        assert_eq!(s.text, "Tommaso");
        assert_eq!(signature_from(&[], "tommaso@esempio.it"), Signature::default());
    }

    #[test]
    fn reply_reply_all_and_forward_drafts() {
        let (_d, db) = test_db();
        db.write(|c| {
            let me = "tommaso@esempio.it";
            store::ensure_account(c, me, None, None)?;
            let mut m = crate::store::testdata::msg("m1", "t1", 1_000, "Anna Rossi <anna@cliente.it>", &["INBOX"], Some("Ci vediamo giovedì?"));
            // Destinatari: io e Luca; in copia Sara.
            let p = m.payload.as_mut().unwrap();
            p.headers.retain(|h| h.name != "To");
            p.headers.push(crate::gmail::Header { name: "To".into(), value: "Tommaso <tommaso@esempio.it>, Luca <luca@x.it>".into() });
            p.headers.push(crate::gmail::Header { name: "Cc".into(), value: "sara@x.it".into() });
            store::write_thread(c, &thread("t1", 1, vec![m]), true, me)?;

            let r = reply_draft(c, "t1", ReplyMode::Reply, me)?;
            assert_eq!(r.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["anna@cliente.it"]);
            assert!(r.cc.is_empty());
            assert_eq!(r.subject, "Re: Oggetto t1");
            assert_eq!(r.thread_id.as_deref(), Some("t1"));
            assert_eq!(r.in_reply_to.as_deref(), Some("<m1@x>"));
            assert!(r.quoted.as_deref().unwrap().contains("> Ci vediamo giovedì?"));

            let all = reply_draft(c, "t1", ReplyMode::ReplyAll, me)?;
            assert_eq!(all.cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["luca@x.it", "sara@x.it"], "senza di me");

            let f = reply_draft(c, "t1", ReplyMode::Forward, me)?;
            assert!(f.to.is_empty() && f.thread_id.is_none());
            assert_eq!(f.subject, "Fwd: Oggetto t1");
            assert!(f.quoted.as_deref().unwrap().contains("Messaggio inoltrato"));
            assert!(f.attachments.is_empty(), "il messaggio di prova non ha allegati");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn queue_can_be_cancelled_only_before_sending() {
        let (_d, db) = test_db();
        db.write(|c| {
            let from = addr("Tommaso", "tommaso@esempio.it");
            let d = Draft { to: vec![addr("Anna", "anna@x.it")], subject: "Ciao".into(), body: "x".into(), ..Default::default() };
            assert!(enqueue(c, &Draft::default(), &from, 0, None, &[]).is_err(), "senza destinatari non parte");
            let (a, _) = enqueue(c, &d, &from, 60_000, None, &[])?;
            assert_eq!(cancel(c, a)?, Some(d.clone()), "annullato riporta la bozza");
            let (b, _) = enqueue(c, &d, &from, 0, None, &[])?;
            let claimed = claim(c)?;
            assert_eq!(claimed.iter().map(|p| p.id).collect::<Vec<_>>(), [b]);
            assert!(claim(c)?.is_empty(), "non parte due volte");
            assert_eq!(cancel(c, b)?, None, "già in invio: non si annulla più");
            assert!(!send_now(c, b)?, "già in invio: niente da anticipare");

            // Invia ora: parte al prossimo giro, senza aspettare i 60 secondi.
            let (n, _) = enqueue(c, &d, &from, 60_000, None, &[])?;
            assert!(claim(c)?.is_empty());
            assert!(send_now(c, n)?);
            assert_eq!(claim(c)?.iter().map(|p| p.id).collect::<Vec<_>>(), [n]);
            Ok(())
        })
        .unwrap();
    }
}
