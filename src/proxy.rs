//! The team server relays the calls of a space's AIs (docs/collaboration-serveur.md, step 3).
//!
//! The members never get the company's key: each gets a token of their own (`Db::proxy_token`)
//! and their agent (Claude Code, Codex) calls the team server at `/ia/<ai>/…`, on the same port
//! as the space. For each request the server checks the token (a member taken off the space is
//! cut off at once), counts the call (and stops at the daily quota, if one is set), then sends
//! it on to the company's model with the real key and passes the answer back as it comes (the
//! models answer bit by bit).
//!
//! Plain HTTP/1.1 relaying, one request per connection (`Connection: close` both ways).

use crate::team::Db;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Largest request body taken (a long conversation with its files).
const MAX_BODY: usize = 64 * 1024 * 1024;

/// A request's head: method, path, headers (names in lower case).
pub struct Head {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Is it the start of a WebSocket connection (the space's own traffic)?
    pub fn is_websocket(&self) -> bool {
        self.header("upgrade")
            .is_some_and(|u| u.eq_ignore_ascii_case("websocket"))
    }
}

/// Parse a request head (up to the empty line). None when it isn't one.
pub fn parse_head(bytes: &[u8]) -> Option<Head> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let path = first.next()?.to_string();
    let headers = lines
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect();
    Some(Head {
        method,
        path,
        headers,
    })
}

/// `/ia/<slug>/<rest>` → (slug, "/<rest>" with its query).
pub fn ai_path(path: &str) -> Option<(String, String)> {
    let rest = path.strip_prefix("/ia/")?;
    let (slug, after) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if slug.is_empty() {
        return None;
    }
    let after = if after.starts_with('?') {
        format!("/{after}")
    } else {
        after.to_string()
    };
    Some((slug.to_string(), after))
}

/// The name of an AI in its path: lower case, letters and digits, dashes between.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

fn answer(stream: &mut TcpStream, status: &str, msg: &str) {
    let body = serde_json::json!({"error": {"type": "orchestra", "message": msg}}).to_string();
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

/// The request's body: `already` (read with the head) then the rest, by length or in chunks.
fn read_body(stream: &mut TcpStream, head: &Head, mut already: Vec<u8>) -> Result<Vec<u8>, String> {
    if head
        .header("transfer-encoding")
        .is_some_and(|t| t.to_ascii_lowercase().contains("chunked"))
    {
        // Read everything chunk by chunk.
        let mut raw = already;
        let mut out = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            // Parse what we have.
            loop {
                let Some(eol) = raw.windows(2).position(|w| w == b"\r\n") else {
                    break;
                };
                let size_line = std::str::from_utf8(&raw[..eol]).map_err(|e| e.to_string())?;
                let size =
                    usize::from_str_radix(size_line.split(';').next().unwrap_or("").trim(), 16)
                        .map_err(|e| e.to_string())?;
                if size == 0 {
                    return Ok(out);
                }
                if raw.len() < eol + 2 + size + 2 {
                    break;
                }
                out.extend_from_slice(&raw[eol + 2..eol + 2 + size]);
                raw.drain(..eol + 2 + size + 2);
                if out.len() > MAX_BODY {
                    return Err("too large".into());
                }
            }
            let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("cut short".into());
            }
            raw.extend_from_slice(&buf[..n]);
        }
    }
    let len: usize = head
        .header("content-length")
        .and_then(|l| l.parse().ok())
        .unwrap_or(0);
    if len > MAX_BODY {
        return Err("too large".into());
    }
    let mut buf = [0u8; 16 * 1024];
    while already.len() < len {
        let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("cut short".into());
        }
        already.extend_from_slice(&buf[..n]);
    }
    already.truncate(len);
    Ok(already)
}

/// `https://host:port/prefix` → (TLS, host, port, prefix without its last slash).
pub fn upstream(base: &str) -> Option<(bool, String, u16, String)> {
    let (tls, rest) = if let Some(r) = base.strip_prefix("https://") {
        (true, r)
    } else {
        (false, base.strip_prefix("http://")?)
    };
    let (hostport, prefix) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].trim_end_matches('/')),
        None => (rest, ""),
    };
    let default = if tls { 443 } else { 80 };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if !h.contains(']') || h.ends_with(']') => (h, p.parse().ok()?),
        _ => (hostport, default),
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return None;
    }
    Some((tls, host.to_string(), port, prefix.to_string()))
}

/// Headers never passed on: the connection's own, and the member's token.
const DROPPED: [&str; 9] = [
    "host",
    "authorization",
    "x-api-key",
    "connection",
    "keep-alive",
    "content-length",
    "transfer-encoding",
    "proxy-authorization",
    "proxy-connection",
];

/// Serve one HTTP request that came to the team server (not a WebSocket).
pub fn serve_http(mut stream: TcpStream, head: Head, rest: Vec<u8>, db: &Arc<Mutex<Db>>) {
    let Some((slug, path)) = ai_path(&head.path) else {
        answer(&mut stream, "404 Not Found", "unknown path");
        return;
    };
    // The member's token, as their agent sends it.
    let token = head
        .header("authorization")
        .and_then(|a| a.strip_prefix("Bearer ").or(a.strip_prefix("bearer ")))
        .or(head.header("x-api-key"))
        .unwrap_or("")
        .trim()
        .to_string();
    let grant = match db.lock() {
        Ok(d) => d.proxy_grant(&token, &slug),
        Err(_) => Err("server error".into()),
    };
    let (ai, key) = match grant {
        Ok(g) => g,
        Err(e) => {
            answer(&mut stream, "403 Forbidden", &e);
            return;
        }
    };
    let body = match read_body(&mut stream, &head, rest) {
        Ok(b) => b,
        Err(e) => {
            answer(&mut stream, "400 Bad Request", &e);
            return;
        }
    };
    let Some((tls, host, port, prefix)) = upstream(&ai.base_url) else {
        answer(
            &mut stream,
            "502 Bad Gateway",
            "the space's AI has an invalid address",
        );
        return;
    };
    // The request as it goes on: the company's key in the same place the member's token was.
    let mut out = format!(
        "{} {prefix}{path} HTTP/1.1\r\nHost: {host}\r\n",
        head.method
    );
    for (k, v) in head
        .headers
        .iter()
        .filter(|(k, _)| !DROPPED.contains(&k.as_str()))
    {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    if head.header("x-api-key").is_some() && head.header("authorization").is_none() {
        out.push_str(&format!("x-api-key: {key}\r\n"));
    } else {
        out.push_str(&format!("Authorization: Bearer {key}\r\n"));
    }
    out.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    let relayed = relay(&mut stream, tls, &host, port, out.as_bytes(), &body);
    if let Err(e) = relayed {
        answer(
            &mut stream,
            "502 Bad Gateway",
            &format!("the space's AI: {e}"),
        );
    }
}

/// Send the request to the company's model and pass its answer back as it comes.
fn relay(
    client: &mut TcpStream,
    tls: bool,
    host: &str,
    port: u16,
    head: &[u8],
    body: &[u8],
) -> Result<(), String> {
    let addrs: Vec<_> = std::net::ToSocketAddrs::to_socket_addrs(&(host, port))
        .map_err(|e| e.to_string())?
        .collect();
    let mut last = "address not found".to_string();
    let mut tcp = None;
    for a in &addrs {
        match TcpStream::connect_timeout(a, Duration::from_secs(10)) {
            Ok(s) => {
                tcp = Some(s);
                break;
            }
            Err(e) => last = e.to_string(),
        }
    }
    let tcp = tcp.ok_or(last)?;
    // Models may think for a long while before the first byte.
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(600)));
    let _ = tcp.set_write_timeout(Some(Duration::from_secs(60)));
    let mut up: Box<dyn ReadWrite> = if tls {
        let name =
            rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|e| e.to_string())?;
        let conn = rustls::ClientConnection::new(crate::team::tls_config(), name)
            .map_err(|e| e.to_string())?;
        Box::new(rustls::StreamOwned::new(conn, tcp))
    } else {
        Box::new(tcp)
    };
    up.write_all(head).map_err(|e| e.to_string())?;
    up.write_all(body).map_err(|e| e.to_string())?;
    up.flush().map_err(|e| e.to_string())?;
    let mut buf = [0u8; 16 * 1024];
    let mut sent_any = false;
    loop {
        let n = match up.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            // TLS servers often end without a closing message: the answer is complete.
            Err(e) if sent_any && e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) if !sent_any => return Err(e.to_string()),
            Err(_) => break,
        };
        client.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        client.flush().map_err(|e| e.to_string())?;
        sent_any = true;
    }
    if !sent_any {
        return Err("no answer".into());
    }
    Ok(())
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_names_and_addresses() {
        assert_eq!(
            ai_path("/ia/modele-maison/v1/messages?beta=true"),
            Some(("modele-maison".into(), "/v1/messages?beta=true".into()))
        );
        assert_eq!(ai_path("/ia/x?y=1"), Some(("x".into(), "/?y=1".into())));
        assert_eq!(ai_path("/autre"), None);
        assert_eq!(ai_path("/ia/"), None);
        assert_eq!(slug("Modèle maison (v2)"), "mod-le-maison-v2");
        assert_eq!(
            upstream("https://ia.example/v1/"),
            Some((true, "ia.example".into(), 443, "/v1".into()))
        );
        assert_eq!(
            upstream("http://10.0.0.5:4000"),
            Some((false, "10.0.0.5".into(), 4000, "".into()))
        );
        assert_eq!(upstream("ftp://x"), None);
        let h = parse_head(b"POST /ia/x/v1 HTTP/1.1\r\nHost: a\r\nX-Api-Key: k\r\n\r\n").unwrap();
        assert_eq!(h.header("x-api-key"), Some("k"));
        assert!(!h.is_websocket());
    }
}
