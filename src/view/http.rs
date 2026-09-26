//! Just enough HTTP/1.1 for a local viewer: requests without bodies,
//! fixed-length responses, and one long-lived event stream.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
}

impl Request {
    pub fn param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The first header called `name`, ignoring case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

const MAX_HEAD: usize = 16 * 1024;

/// How long a request's head may take to arrive, all told. (A trickle of
/// bytes would otherwise hold a connection open indefinitely.)
const HEAD_DEADLINE: Duration = Duration::from_secs(5);

pub fn read_request(stream: &mut TcpStream) -> io::Result<Request> {
    read_request_within(stream, HEAD_DEADLINE)
}

fn read_request_within(stream: &mut TcpStream, limit: Duration) -> io::Result<Request> {
    let deadline = std::time::Instant::now() + limit;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "request too slow"));
        }
        stream.set_read_timeout(Some(left))?;
        let n = stream.read(&mut chunk)?;
        if n == 0 || buf.len() > MAX_HEAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "incomplete request",
            ));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&buf);
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let method = first.next().unwrap_or_default().to_string();
    let target = first.next().unwrap_or("/");
    let headers = lines
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();

    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let query = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect();
    Ok(Request {
        method,
        path: percent_decode(path),
        query,
        headers,
    })
}

pub fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    extra: &[(&str, &str)],
    body: &[u8],
) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n\
         Connection: close\r\n",
        reason(status),
        body.len()
    );
    for (name, value) in extra {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

pub fn start_event_stream(stream: &mut TcpStream) -> io::Result<()> {
    stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\
          X-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\nretry: 2000\n\n",
    )?;
    stream.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "Error",
    }
}

pub fn percent_decode(s: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let decoded = match bytes[i] {
            b'%' if i + 2 < bytes.len() => hex(bytes[i + 1]).zip(hex(bytes[i + 2])),
            _ => None,
        };
        match (bytes[i], decoded) {
            (_, Some((hi, lo))) => {
                out.push(hi << 4 | lo);
                i += 3;
            }
            (b'+', None) => {
                out.push(b' ');
                i += 1;
            }
            (b, None) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R7: a client sending its request a byte at a time can't hold the
    /// connection (and its thread) past the deadline.
    #[test]
    fn a_trickled_request_is_cut_off() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(addr).unwrap();
            for _ in 0..40 {
                if stream.write_all(b"G").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let (mut server, _) = listener.accept().unwrap();
        let start = std::time::Instant::now();
        let result = read_request_within(&mut server, Duration::from_millis(300));
        assert!(result.is_err());
        assert!(
            start.elapsed() < Duration::from_millis(1500),
            "{:?}",
            start.elapsed()
        );
        drop(server);
        client.join().unwrap();
    }

    #[test]
    fn decodes_query_values() {
        assert_eq!(
            percent_decode("claude-code%3Aabc%2Fdef"),
            "claude-code:abc/def"
        );
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
