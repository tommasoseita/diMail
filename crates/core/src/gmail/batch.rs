//! Batch HTTP di Gmail (`multipart/mixed`), al massimo 50 richieste per batch (sez. 5.2).
//! Ogni richiesta interna conta per la quota come se fosse fatta da sola.

use crate::{Error, Result};

pub const MAX_BATCH: usize = 50;

#[derive(Clone)]
pub struct SubRequest {
    pub method: &'static str,
    /// Percorso assoluto, es. `/gmail/v1/users/me/threads/abc?format=full`.
    pub path: String,
    pub json_body: Option<String>,
}

#[derive(Debug)]
pub struct SubResponse {
    pub status: u16,
    pub retry_after: Option<std::time::Duration>,
    pub body: String,
}

pub fn build(boundary: &str, reqs: &[SubRequest]) -> String {
    let mut out = String::new();
    for (i, r) in reqs.iter().enumerate() {
        out.push_str("--");
        out.push_str(boundary);
        out.push_str("\r\nContent-Type: application/http\r\n");
        out.push_str(&format!("Content-ID: <item-{i}>\r\n\r\n"));
        out.push_str(&format!("{} {} HTTP/1.1\r\n", r.method, r.path));
        match &r.json_body {
            Some(body) => {
                out.push_str("Content-Type: application/json; charset=UTF-8\r\n");
                out.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
                out.push_str(body);
                out.push_str("\r\n");
            }
            None => out.push_str("\r\n"),
        }
    }
    out.push_str("--");
    out.push_str(boundary);
    out.push_str("--\r\n");
    out
}

pub fn boundary_from_content_type(ct: &str) -> Option<String> {
    ct.split(';').map(str::trim).find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.trim().eq_ignore_ascii_case("boundary").then(|| v.trim().trim_matches('"').to_owned())
    })
}

/// Separa intestazioni e corpo, accettando sia CRLF sia LF.
fn split_head(s: &str) -> (&str, &str) {
    let crlf = s.find("\r\n\r\n").map(|i| (i, 4));
    let lf = s.find("\n\n").map(|i| (i, 2));
    let pick = match (crlf, lf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    };
    match pick {
        Some((i, n)) => (&s[..i], &s[i + n..]),
        None => (s, ""),
    }
}

/// Restituisce le risposte nell'ordine delle richieste (`n` = numero di richieste inviate).
pub fn parse(content_type: &str, body: &str, n: usize) -> Result<Vec<SubResponse>> {
    let boundary = boundary_from_content_type(content_type)
        .ok_or_else(|| Error::other("risposta batch senza boundary"))?;
    let delim = format!("--{boundary}");
    let mut slots: Vec<Option<SubResponse>> = (0..n).map(|_| None).collect();
    let mut order = 0usize;

    for raw in body.split(delim.as_str()).skip(1) {
        if raw.starts_with("--") {
            break; // chiusura
        }
        let raw = raw.trim_start_matches(['\r', '\n']);
        let (part_headers, http) = split_head(raw);
        let index = part_headers
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.trim().eq_ignore_ascii_case("content-id").then(|| v.trim().to_owned())
            })
            .and_then(|v| v.trim_matches(['<', '>']).rsplit("item-").next().and_then(|n| n.parse::<usize>().ok()))
            .unwrap_or(order);
        order += 1;

        let (resp_head, resp_body) = split_head(http);
        let mut lines = resp_head.lines();
        let status = lines
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .ok_or_else(|| Error::other("risposta batch senza stato HTTP"))?;
        let retry_after = lines.find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("retry-after")
                .then(|| v.trim().parse::<u64>().ok().map(std::time::Duration::from_secs))
                .flatten()
        });
        if index < n {
            slots[index] = Some(SubResponse {
                status,
                retry_after,
                body: resp_body.trim_end_matches(['\r', '\n']).to_owned(),
            });
        }
    }

    Ok(slots
        .into_iter()
        .map(|s| {
            s.unwrap_or(SubResponse { status: 503, retry_after: None, body: String::from("{}") })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_get_and_post_parts() {
        let body = build(
            "b1",
            &[
                SubRequest { method: "GET", path: "/gmail/v1/users/me/threads/t1?format=full".into(), json_body: None },
                SubRequest {
                    method: "POST",
                    path: "/gmail/v1/users/me/threads/t2/modify".into(),
                    json_body: Some(r#"{"removeLabelIds":["INBOX"]}"#.into()),
                },
            ],
        );
        assert!(body.starts_with("--b1\r\nContent-Type: application/http\r\nContent-ID: <item-0>\r\n\r\nGET /gmail/v1/users/me/threads/t1?format=full HTTP/1.1\r\n\r\n"));
        assert!(body.contains("POST /gmail/v1/users/me/threads/t2/modify HTTP/1.1\r\nContent-Type: application/json; charset=UTF-8\r\nContent-Length: 28\r\n\r\n{\"removeLabelIds\":[\"INBOX\"]}\r\n"));
        assert!(body.ends_with("--b1--\r\n"));
    }

    #[test]
    fn parses_out_of_order_responses() {
        let resp = "--batch_x\r\nContent-Type: application/http\r\nContent-ID: <response-item-1>\r\n\r\n\
HTTP/1.1 404 Not Found\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{\"error\":{\"code\":404}}\r\n\
--batch_x\r\nContent-Type: application/http\r\nContent-ID: <response-item-0>\r\n\r\n\
HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{\n  \"id\": \"t1\"\n}\r\n\
--batch_x--\r\n";
        let out = parse("multipart/mixed; boundary=batch_x", resp, 3).unwrap();
        assert_eq!(out[0].status, 200);
        assert_eq!(out[0].body, "{\n  \"id\": \"t1\"\n}");
        assert_eq!(out[1].status, 404);
        // Risposta mancante: trattata come errore transitorio.
        assert_eq!(out[2].status, 503);
    }

    #[test]
    fn reads_retry_after() {
        let resp = "--b\r\nContent-ID: <response-item-0>\r\n\r\nHTTP/1.1 429 Too Many Requests\r\nRetry-After: 7\r\n\r\n{}\r\n--b--";
        let out = parse("multipart/mixed; boundary=\"b\"", resp, 1).unwrap();
        assert_eq!(out[0].status, 429);
        assert_eq!(out[0].retry_after, Some(std::time::Duration::from_secs(7)));
    }
}
