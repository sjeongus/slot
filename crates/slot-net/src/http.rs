//! Just enough HTTP/1.1 to talk to Syncthing's REST API on localhost.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json(&self) -> Option<crate::json::Json> {
        crate::json::Json::parse(&self.body)
    }
}

pub fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    api_key: &str,
    body: Option<&str>,
    timeout: Duration,
) -> std::io::Result<Response> {
    send(
        addr,
        &addr.to_string(),
        method,
        path,
        Some(api_key),
        body,
        timeout,
    )
}

/// A plain GET to a public host, for lookups that need nothing secret.
pub fn get(host: &str, port: u16, path: &str, timeout: Duration) -> Option<Response> {
    use std::net::ToSocketAddrs;
    let addrs = (host, port).to_socket_addrs().ok()?;
    for addr in addrs.filter(SocketAddr::is_ipv4) {
        if let Ok(r) = send(addr, host, "GET", path, None, None, timeout) {
            return r.ok().then_some(r);
        }
    }
    None
}

fn send(
    addr: SocketAddr,
    host: &str,
    method: &str,
    path: &str,
    api_key: Option<&str>,
    body: Option<&str>,
    timeout: Duration,
) -> std::io::Result<Response> {
    let mut s = TcpStream::connect_timeout(&addr, timeout)?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    let body = body.unwrap_or("");
    let key = api_key
        .map(|k| format!("X-API-Key: {k}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{key}User-Agent: slot\r\n\
         Connection: close\r\nAccept: application/json\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes())?;
    s.write_all(body.as_bytes())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)?;
    parse(&raw).ok_or_else(|| std::io::Error::other("malformed HTTP response"))
}

pub fn parse(raw: &[u8]) -> Option<Response> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..split]).ok()?;
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let chunked = lines.any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });
    let rest = &raw[split + 4..];
    let body = if chunked {
        dechunk(rest)?
    } else {
        rest.to_vec()
    };
    Some(Response {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn dechunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = b.windows(2).position(|w| w == b"\r\n")?;
        let size_text = std::str::from_utf8(&b[..eol]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        b = &b[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(b.get(..size)?);
        b = b.get(size + 2..)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_plain_body() {
        let r = parse(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").unwrap();
        assert!(r.ok());
        assert_eq!(r.body, "{}");
    }

    #[test]
    fn reads_a_chunked_body() {
        let r = parse(
            b"HTTP/1.1 403 Forbidden\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(r.status, 403);
        assert!(!r.ok());
        assert_eq!(r.body, "abcde");
    }

    #[test]
    fn talks_to_a_local_server() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut buf = [0u8; 1024];
            let n = c.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"myID\":1}\n")
                .unwrap();
            req
        });
        let r = request(
            addr,
            "GET",
            "/rest/system/status",
            "k",
            None,
            Duration::from_secs(2),
        )
        .unwrap();
        let req = t.join().unwrap();
        assert!(req.starts_with("GET /rest/system/status HTTP/1.1\r\n"));
        assert!(req.contains("X-API-Key: k\r\n"));
        assert_eq!(r.json().unwrap().get("myID"), &crate::json::Json::Num(1.0));
    }
}
