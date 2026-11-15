//! Browser sign-in to laicpmsdb for the standalone desktop app (RFC 8252
//! loopback redirect).
//!
//! `sign_in_browser` listens once on `127.0.0.1:<free port>`, opens the
//! system browser at `<server>/app/#/connect?app=grainatlas&callback=…&state=…`
//! and waits up to five minutes for laicpmsdb's page to send the browser back
//! to `http://127.0.0.1:<port>/grainatlas/callback?code=…&state=…`. The code
//! (single use, two minutes) goes to the page, which trades it at
//! `POST /v1/auth/exchange`; no token passes through here. Requests whose
//! `state` does not match are refused and the wait goes on.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const CALLBACK_PATH: &str = "/grainatlas/callback";
const WAIT: Duration = Duration::from_secs(300);

static CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Debug, PartialEq)]
enum Answer {
    Code(String),
    Error(String),
    /// Not for us (wrong path) or a wrong state: answer and keep waiting.
    Ignore,
}

/// Start a browser sign-in and return the one-time code.
#[tauri::command]
pub async fn sign_in_browser(server: String, state: String) -> Result<String, String> {
    let server = server.trim().trim_end_matches('/').to_string();
    if !(server.starts_with("https://") || server.starts_with("http://")) || server.chars().any(char::is_whitespace) {
        return Err("Set the laicpmsdb server first (http:// or https://).".into());
    }
    if !(16..=128).contains(&state.len()) || !state.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Invalid sign-in state.".into());
    }
    CANCEL.store(false, Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || run(&server, &state))
        .await
        .map_err(|e| e.to_string())?
}

/// Stop a waiting sign-in (the Cancel button).
#[tauri::command]
pub fn sign_in_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

fn run(server: &str, state: &str) -> Result<String, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("Could not listen for the sign-in: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let url = connect_url(server, &format!("http://127.0.0.1:{port}{CALLBACK_PATH}"), state);
    open_browser(&url)?;
    let deadline = Instant::now() + WAIT;
    loop {
        if CANCEL.load(Ordering::SeqCst) {
            return Err("Sign-in cancelled.".into());
        }
        if Instant::now() > deadline {
            return Err("Sign-in timed out after five minutes; try again.".into());
        }
        match listener.accept() {
            Ok((stream, _)) => match serve(stream, state) {
                Answer::Code(code) => return Ok(code),
                Answer::Error(e) => return Err(e),
                Answer::Ignore => {}
            },
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(150)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn connect_url(server: &str, callback: &str, state: &str) -> String {
    format!(
        "{server}/app/#/connect?app=grainatlas&callback={}&state={}",
        encode(callback),
        encode(state)
    )
}

fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(url).status();
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).status();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let status = std::process::Command::new("xdg-open").arg(url).status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("Could not open the browser ({s}).")),
        Err(e) => Err(format!("Could not open the browser: {e}")),
    }
}

/// Read one request, answer it with a short page, and say what it meant.
fn serve(mut stream: TcpStream, state: &str) -> Answer {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];
    while buf.len() < 16 * 1024 && !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let request = String::from_utf8_lossy(&buf);
    let target = request.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("");
    let answer = parse_callback(target, state);
    let (status, body) = match &answer {
        Answer::Code(_) => ("200 OK", "GrainAtlas is signed in. You can close this tab and go back to GrainAtlas."),
        Answer::Error(_) => ("200 OK", "GrainAtlas was not signed in. You can close this tab."),
        Answer::Ignore => ("404 Not Found", "Not found."),
    };
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>GrainAtlas</title><body style=\"font:16px system-ui;margin:3em\"><p>{body}</p>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{html}",
        html.len()
    );
    let _ = stream.flush();
    answer
}

fn parse_callback(target: &str, state: &str) -> Answer {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != CALLBACK_PATH {
        return Answer::Ignore;
    }
    let mut code = None;
    let mut error = None;
    let mut got_state = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = decode(v);
        match k {
            "code" => code = Some(v),
            "error" => error = Some(v),
            "state" => got_state = Some(v),
            _ => {}
        }
    }
    if got_state.as_deref() != Some(state) {
        return Answer::Ignore;
    }
    match (code, error) {
        (Some(c), _) if !c.is_empty() => Answer::Code(c),
        (_, Some(e)) if e == "cancelled" => Answer::Error("Sign-in cancelled in the browser.".into()),
        (_, Some(e)) => Answer::Error(format!("Sign-in failed ({e}).")),
        _ => Answer::Error("The browser returned no sign-in code.".into()),
    }
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: &str = "abcdefghijklmnop_1234";

    #[test]
    fn code_with_matching_state() {
        assert_eq!(
            parse_callback(&format!("/grainatlas/callback?code=x%2By&state={STATE}"), STATE),
            Answer::Code("x+y".into())
        );
    }

    #[test]
    fn wrong_state_or_path_is_ignored() {
        assert_eq!(parse_callback("/grainatlas/callback?code=c&state=other", STATE), Answer::Ignore);
        assert_eq!(parse_callback("/grainatlas/callback?code=c", STATE), Answer::Ignore);
        assert_eq!(parse_callback(&format!("/favicon.ico?state={STATE}"), STATE), Answer::Ignore);
    }

    #[test]
    fn cancel_and_errors() {
        assert_eq!(
            parse_callback(&format!("/grainatlas/callback?error=cancelled&state={STATE}"), STATE),
            Answer::Error("Sign-in cancelled in the browser.".into())
        );
        assert!(matches!(parse_callback(&format!("/grainatlas/callback?state={STATE}"), STATE), Answer::Error(_)));
    }

    #[test]
    fn connect_url_encodes_the_callback() {
        assert_eq!(
            connect_url("https://h.example", "http://127.0.0.1:5555/grainatlas/callback", STATE),
            format!("https://h.example/app/#/connect?app=grainatlas&callback=http%3A%2F%2F127.0.0.1%3A5555%2Fgrainatlas%2Fcallback&state={STATE}")
        );
    }

    #[test]
    fn decode_handles_short_escapes() {
        assert_eq!(decode("a%2"), "a%2");
        assert_eq!(decode("%41%42"), "AB");
    }

    #[test]
    fn serves_one_request_over_loopback() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(s, "GET /grainatlas/callback?code=abc&state={STATE} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        });
        let (stream, _) = listener.accept().unwrap();
        assert_eq!(serve(stream, STATE), Answer::Code("abc".into()));
        assert!(client.join().unwrap().contains("signed in"));
    }
}
