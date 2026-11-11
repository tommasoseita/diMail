//! Allegati (9 novembre): si scaricano da Gmail solo quando servono, cioè quando li apro o li salvo,
//! e poi restano nella cache dell'account (`accounts/<email>/attachments/`).
//!
//! Gmail dà gli allegati grandi con un `attachmentId` (`messages.attachments.get`, 5 unità) e quelli
//! piccoli dentro il messaggio. L'`attachmentId` può cambiare tra uno scaricamento e l'altro: se
//! Gmail non lo riconosce più, si rilegge il messaggio e si usa quello nuovo.
//!
//! I file scaricati hanno l'attributo di quarantena di macOS, come quelli di Mail: se un allegato è
//! un programma, Gatekeeper lo controlla prima di aprirlo.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::gmail::{GmailApi, MessagePart, Priority, ThreadFormat};
use crate::sync::Ctx;
use crate::util::{b64url_decode, now_ms};
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize)]
pub struct AttachmentFile {
    pub path: PathBuf,
    pub filename: String,
    pub mime_type: String,
}

struct Row {
    gmail_attachment_id: Option<String>,
    filename: String,
    mime_type: String,
    cache_path: Option<String>,
}

fn lookup(c: &Connection, message_id: &str, part_id: &str) -> Result<Option<Row>> {
    Ok(c
        .query_row(
            "SELECT a.gmail_attachment_id, a.filename, a.mime_type, a.cache_path
             FROM attachments a JOIN messages m ON m.rowid = a.message_rowid
             WHERE m.id = ?1 AND a.part_id = ?2",
            params![message_id, part_id],
            |r| Ok(Row { gmail_attachment_id: r.get(0)?, filename: r.get(1)?, mime_type: r.get(2)?, cache_path: r.get(3)? }),
        )
        .optional()?)
}

/// Nome di file sicuro: niente percorsi, caratteri di controllo o punti iniziali; l'estensione resta.
pub fn safe_name(filename: &str, part_id: &str) -> String {
    let cleaned: String = filename
        .chars()
        .map(|c| if c == '/' || c == '\\' || c == ':' || c.is_control() { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim();
    if cleaned.is_empty() {
        return format!("allegato-{}", part_id.replace(|c: char| !c.is_ascii_alphanumeric(), "-"));
    }
    if cleaned.chars().count() <= 150 {
        return cleaned.to_owned();
    }
    let (stem, ext) = match cleaned.rsplit_once('.') {
        Some((s, e)) if e.len() <= 10 => (s, format!(".{e}")),
        _ => (cleaned, String::new()),
    };
    format!("{}{ext}", stem.chars().take(140).collect::<String>())
}

/// La parte del messaggio con questo `partId`.
fn find_part<'a>(part: &'a MessagePart, part_id: &str) -> Option<&'a MessagePart> {
    if part.part_id.as_deref() == Some(part_id) {
        return Some(part);
    }
    part.parts.iter().find_map(|p| find_part(p, part_id))
}

/// Contenuto dell'allegato da Gmail.
async fn download<G: GmailApi>(ctx: &Ctx<G>, message_id: &str, part_id: &str, attachment_id: Option<&str>) -> Result<Vec<u8>> {
    if let Some(att) = attachment_id {
        match ctx.gmail.get_attachment(message_id, att).await {
            Ok(bytes) => return Ok(bytes),
            // ID non più valido: si rilegge il messaggio.
            Err(Error::NotFound | Error::Api { status: 400, .. }) => {}
            Err(e) => return Err(e),
        }
    }
    let msg = ctx
        .gmail
        .get_messages(&[message_id.to_owned()], ThreadFormat::Full, Priority::Interactive)
        .await?
        .into_iter()
        .next()
        .ok_or(Error::NotFound)??;
    let part = msg.payload.as_ref().and_then(|p| find_part(p, part_id)).ok_or(Error::NotFound)?;
    let body = part.body.as_ref().ok_or(Error::NotFound)?;
    if let Some(data) = &body.data {
        return b64url_decode(data);
    }
    let att = body.attachment_id.as_deref().ok_or(Error::NotFound)?;
    ctx.gmail.get_attachment(message_id, att).await
}

fn quarantine(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        let value = format!("0081;{:x};diMail;", now_ms() / 1000);
        let _ = std::process::Command::new("/usr/bin/xattr").arg("-w").arg("com.apple.quarantine").arg(value).arg(path).status();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = path;
}

/// L'allegato su disco: dalla cache, o scaricato adesso.
pub async fn fetch<G: GmailApi>(ctx: &Ctx<G>, cache_dir: &Path, message_id: &str, part_id: &str) -> Result<AttachmentFile> {
    let (mid, pid) = (message_id.to_owned(), part_id.to_owned());
    let row = ctx
        .db
        .read_async(move |c| lookup(c, &mid, &pid))
        .await?
        .ok_or_else(|| Error::other("allegato non trovato"))?;
    if let Some(p) = &row.cache_path
        && Path::new(p).is_file()
    {
        return Ok(AttachmentFile { path: PathBuf::from(p), filename: row.filename, mime_type: row.mime_type });
    }
    let bytes = download(ctx, message_id, part_id, row.gmail_attachment_id.as_deref()).await?;
    let dir = cache_dir.join(message_id.replace(|c: char| !c.is_ascii_alphanumeric(), "_")).join(part_id.replace('.', "_"));
    std::fs::create_dir_all(&dir).map_err(|e| Error::other(format!("cartella degli allegati: {e}")))?;
    let path = dir.join(safe_name(&row.filename, part_id));
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, &bytes).map_err(|e| Error::other(format!("allegato non salvato: {e}")))?;
    std::fs::rename(&tmp, &path).map_err(|e| Error::other(format!("allegato non salvato: {e}")))?;
    quarantine(&path);
    let (mid, pid, p) = (message_id.to_owned(), part_id.to_owned(), path.to_string_lossy().into_owned());
    ctx.db
        .write_async(move |c| {
            c.execute(
                "UPDATE attachments SET cache_path = ?3 WHERE part_id = ?2 AND message_rowid = (SELECT rowid FROM messages WHERE id = ?1)",
                params![mid, pid, p],
            )?;
            Ok(())
        })
        .await?;
    tracing::info!(byte = bytes.len(), "allegato scaricato");
    Ok(AttachmentFile { path, filename: row.filename, mime_type: row.mime_type })
}

/// Il tipo MIME dall'estensione, per i file del Mac allegati alle email.
pub fn guess_mime(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "ics" => "text/calendar",
        "json" => "application/json",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "key" => "application/vnd.apple.keynote",
        "pages" => "application/vnd.apple.pages",
        "numbers" => "application/vnd.apple.numbers",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
}

/// Un file del Mac da allegare (scelto o trascinato nel composer): nome, dimensione e tipo.
pub fn local_file(path: &Path) -> Result<crate::send::OutAttachment> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let meta = std::fs::metadata(path).map_err(|e| Error::other(format!("«{name}» non si legge: {e}")))?;
    if !meta.is_file() {
        return Err(Error::other(format!("«{name}» è una cartella: comprimila e allega lo zip")));
    }
    Ok(crate::send::OutAttachment {
        mime_type: guess_mime(&name).to_owned(),
        size: meta.len() as i64,
        path: Some(path.to_string_lossy().into_owned()),
        name,
        ..Default::default()
    })
}

/// Una copia in `dir` (di solito Download), senza sovrascrivere: "nome (2).pdf" se c'è già.
pub fn copy_to(file: &AttachmentFile, dir: &Path) -> Result<PathBuf> {
    let name = safe_name(&file.filename, "1");
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_owned(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let mut target = dir.join(&name);
    let mut n = 2;
    while target.exists() {
        target = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    std::fs::copy(&file.path, &target).map_err(|e| Error::other(format!("copia non riuscita: {e}")))?;
    quarantine(&target);
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe() {
        assert_eq!(safe_name("Fattura 20-2026.pdf", "1"), "Fattura 20-2026.pdf");
        assert_eq!(safe_name("../../etc/passwd", "1"), "_.._etc_passwd");
        assert_eq!(safe_name("  ", "0.1"), "allegato-0-1");
        assert_eq!(safe_name(".nascosto", "1"), "nascosto");
        let long = format!("{}.pdf", "a".repeat(300));
        let s = safe_name(&long, "1");
        assert!(s.ends_with(".pdf") && s.chars().count() <= 150);
    }

    #[test]
    fn copies_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.pdf");
        std::fs::write(&src, b"x").unwrap();
        let f = AttachmentFile { path: src, filename: "Fattura.pdf".into(), mime_type: "application/pdf".into() };
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        assert_eq!(copy_to(&f, &out).unwrap().file_name().unwrap(), "Fattura.pdf");
        assert_eq!(copy_to(&f, &out).unwrap().file_name().unwrap(), "Fattura (2).pdf");
    }
}
