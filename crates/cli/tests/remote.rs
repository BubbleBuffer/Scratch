//! app ... against a fake app endpoint: a TcpListener on 127.0.0.1:0 that
//! answers the v1 protocol and records what the CLI sent.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{json, Value};

/// A token of the shape the app writes (64 hex characters).
fn token() -> String {
    "0123456789abcdef".repeat(4)
}

/// Runs one command line and returns its output; panics on an error.
fn run_ok(args: &[&str]) -> String {
    let owned: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let mut out = Vec::new();
    match scratch_cli::run(&owned, &mut out) {
        Ok(()) => String::from_utf8(out).unwrap(),
        Err(error) => panic!("scratch-cli {owned:?}: {error:#}"),
    }
}

/// Runs one command line and returns its error; panics if it succeeds.
fn run_err(args: &[&str]) -> String {
    let owned: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let mut out = Vec::new();
    match scratch_cli::run(&owned, &mut out) {
        Ok(()) => panic!("scratch-cli {owned:?} succeeded"),
        Err(error) => format!("{error:#}"),
    }
}

/// One request as the fake app saw it.
#[derive(Clone, Debug)]
struct Request {
    command: String,
    token: Option<String>,
    body: Value,
}

/// A fake app: a loopback listener with a recorded request log and an
/// endpoint.json in a temporary data folder.
struct App {
    dir: tempfile::TempDir,
    port: u16,
    token: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl App {
    fn start(
        token: String,
        reply: impl Fn(&Request) -> (u16, Value) + Send + Sync + 'static,
    ) -> App {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = tempfile::tempdir().unwrap();
        write_endpoint(dir.path(), port, &token);
        let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
        let log = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                let (status, body) = reply(&request);
                log.lock().unwrap().push(request);
                respond(&mut stream, status, &body);
            }
        });
        App {
            dir,
            port,
            token,
            requests,
        }
    }

    /// The --data-dir value for this app.
    fn path(&self) -> &str {
        self.dir.path().to_str().unwrap()
    }

    /// Replaces the endpoint file's token (a stale or wrong one).
    fn rewrite_token(&self, token: &str) {
        write_endpoint(self.dir.path(), self.port, token);
    }

    fn take_requests(&self) -> Vec<Request> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

fn write_endpoint(data_dir: &Path, port: u16, token: &str) {
    let folder = data_dir.join("remote");
    std::fs::create_dir_all(&folder).unwrap();
    let body = json!({
        "port": port,
        "token": token,
        "pid": 4242,
        "version": "1.2.3",
    });
    std::fs::write(folder.join("endpoint.json"), body.to_string()).unwrap();
}

fn respond(stream: &mut TcpStream, status: u16, body: &Value) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let body = body.to_string();
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        if let Some(split) = find(&raw, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&raw[..split]).to_string();
            let length = head
                .split("\r\n")
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if raw.len() >= split + 4 + length {
                let body = &raw[split + 4..split + 4 + length];
                return Some(parse_request(&head, body));
            }
        }
        let read = stream.read(&mut buf).ok()?;
        if read == 0 {
            return None;
        }
        raw.extend_from_slice(&buf[..read]);
    }
}

fn parse_request(head: &str, body: &[u8]) -> Request {
    let mut lines = head.split("\r\n");
    let path = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .nth(1)
        .unwrap_or_default();
    let command = path.rsplit('/').next().unwrap_or_default().to_string();
    let mut token = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("authorization") {
            token = value.trim().strip_prefix("Bearer ").map(str::to_string);
        }
    }
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(body).unwrap_or(Value::Null)
    };
    Request {
        command,
        token,
        body,
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|window| window == needle)
}

fn status_body() -> Value {
    json!({
        "ok": true,
        "version": "1.0.0",
        "pid": 99,
        "vault": { "open": true, "name": "Notes" },
        "page": { "id": "p1", "title": "Inbox" },
    })
}

#[test]
fn missing_endpoint_file_is_not_running() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_str().unwrap();
    let error = run_err(&["app", "--data-dir", data_dir, "status"]);
    assert!(
        error.contains("Scratch is not running (no endpoint at"),
        "{error}"
    );
    assert!(error.contains("endpoint.json"), "{error}");
    assert!(error.contains("start the app first"), "{error}");
}

#[test]
fn refused_connection_is_not_running() {
    let dir = tempfile::tempdir().unwrap();
    // A port nothing listens on any more: bind, then let it go.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    write_endpoint(dir.path(), port, &token());
    let data_dir = dir.path().to_str().unwrap();
    let error = run_err(&["app", "--data-dir", data_dir, "status"]);
    assert!(
        error.contains("Scratch is not running (no endpoint at"),
        "{error}"
    );
}

#[test]
fn malformed_endpoint_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("remote");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("endpoint.json"), "{\"port\": ").unwrap();
    let data_dir = dir.path().to_str().unwrap();
    let error = run_err(&["app", "--data-dir", data_dir, "status"]);
    assert!(error.contains("malformed endpoint file"), "{error}");
}

#[test]
fn token_header_is_sent_and_checked() {
    let expected = token();
    let checked = expected.clone();
    let app = App::start(expected, move |request| {
        if request.token.as_deref() == Some(checked.as_str()) {
            (200, status_body())
        } else {
            (401, json!({ "ok": false, "error": "rejected token" }))
        }
    });
    run_ok(&["app", "--data-dir", app.path(), "status"]);
    let requests = app.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].command, "status");
    assert_eq!(requests[0].token.as_deref(), Some(app.token.as_str()));
    assert!(!app.token.is_empty());

    // A wrong token in the endpoint file: the app rejects it and its own
    // message comes through.
    app.rewrite_token("wrong-token");
    let error = run_err(&["app", "--data-dir", app.path(), "status"]);
    assert!(error.contains("rejected token"), "{error}");
    assert!(!error.contains("Scratch is not running"), "{error}");
}

#[test]
fn status_text_and_json() {
    let app = App::start(token(), |_| (200, status_body()));
    let text = run_ok(&["app", "--data-dir", app.path(), "status"]);
    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        vec![
            "version: 1.0.0",
            "pid: 99",
            "vault: Notes",
            "page: p1 \"Inbox\"",
        ]
    );

    let json_out = run_ok(&["app", "--data-dir", app.path(), "--json", "status"]);
    assert_eq!(json_out.lines().count(), 1, "{json_out:?}");
    let parsed: Value = serde_json::from_str(json_out.trim()).unwrap();
    assert_eq!(parsed, status_body());

    let closed = App::start(token(), |_| {
        (
            200,
            json!({
                "ok": true,
                "version": "2.0.0",
                "pid": 7,
                "vault": { "open": false, "name": null },
                "page": null,
            }),
        )
    });
    let text = run_ok(&["app", "--data-dir", closed.path(), "status"]);
    assert!(text.contains("vault: none"), "{text}");
    assert!(text.contains("page: none"), "{text}");
}

#[test]
fn open_and_navigate_send_path_and_body() {
    let app = App::start(token(), |request| match request.command.as_str() {
        "open" => (200, json!({ "ok": true, "page": { "id": "p9", "title": "Inbox" } })),
        _ => (200, json!({ "ok": true, "route": "home" })),
    });
    assert_eq!(
        run_ok(&["app", "--data-dir", app.path(), "open", "My Page"]),
        "ok\n"
    );
    assert_eq!(
        run_ok(&["app", "--data-dir", app.path(), "navigate", "home"]),
        "ok\n"
    );
    let requests = app.take_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].command, "open");
    assert_eq!(requests[0].body, json!({ "page": "My Page" }));
    assert_eq!(requests[1].command, "navigate");
    assert_eq!(requests[1].body, json!({ "route": "home" }));
}

#[test]
fn tap_type_and_key_send_path_and_body() {
    let app = App::start(token(), |_| (200, json!({ "ok": true })));
    assert_eq!(
        run_ok(&["app", "--data-dir", app.path(), "tap", "save-button"]),
        "ok\n"
    );
    assert_eq!(
        run_ok(&["app", "--data-dir", app.path(), "type", "title-field", "Hello world"]),
        "ok\n"
    );
    assert_eq!(
        run_ok(&["app", "--data-dir", app.path(), "key", "ctrl+k"]),
        "ok\n"
    );
    let requests = app.take_requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].command, "tap");
    assert_eq!(requests[0].body, json!({ "id": "save-button" }));
    assert_eq!(requests[1].command, "type");
    assert_eq!(
        requests[1].body,
        json!({ "id": "title-field", "text": "Hello world", "replace": true })
    );
    assert_eq!(requests[2].command, "key");
    assert_eq!(requests[2].body, json!({ "combo": "ctrl+k" }));
}

#[test]
fn screenshot_decodes_png_base64_and_writes_the_file() {
    let app = App::start(token(), |request| {
        assert_eq!(request.command, "screenshot");
        (
            200,
            json!({
                "ok": true,
                "width": 3,
                "height": 2,
                "png_base64": "iVBORw==",
            }),
        )
    });
    let out_path = app.dir.path().join("shot.png");
    let out = run_ok(&[
        "app",
        "--data-dir",
        app.path(),
        "screenshot",
        out_path.to_str().unwrap(),
    ]);
    assert_eq!(std::fs::read(&out_path).unwrap(), b"\x89PNG");
    assert_eq!(out, format!("wrote {} (3x2)\n", out_path.display()));
    assert_eq!(app.take_requests()[0].body, json!({ "pixel_ratio": 1.0 }));
}

#[test]
fn tree_prints_one_line_per_node() {
    let nodes = json!([
        {
            "id": "n1", "role": "button", "label": "Save", "value": "", "hint": "",
            "actions": ["tap"], "enabled": true, "focused": false,
            "rect": { "x": 0.0, "y": 0.0, "w": 10.0, "h": 10.0 },
        },
        {
            "id": "title", "role": "textfield", "label": "Title", "value": "Hello",
            "hint": "", "actions": ["tap", "setText"], "enabled": true, "focused": true,
            "rect": { "x": 0.0, "y": 20.0, "w": 100.0, "h": 20.0 },
        },
    ]);
    let app = App::start(token(), move |_| {
        (200, json!({ "ok": true, "nodes": nodes.clone() }))
    });
    let out = run_ok(&["app", "--data-dir", app.path(), "tree"]);
    assert_eq!(
        out,
        "n1 button \"Save\" (tap)\ntitle textfield \"Title\" [Hello] (tap, setText)\n"
    );
}

#[test]
fn failed_reply_carries_the_apps_message() {
    let app = App::start(token(), |_| {
        (
            404,
            json!({ "ok": false, "error": "no page called Nope" }),
        )
    });
    let error = run_err(&["app", "--data-dir", app.path(), "open", "Nope"]);
    assert!(error.contains("no page called Nope"), "{error}");
    assert!(!error.contains("HTTP 404"), "{error}");
}

#[test]
fn unknown_subcommand_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_str().unwrap();
    let error = run_err(&["app", "--data-dir", data_dir, "frobnicate"]);
    assert!(error.contains("unknown app command"), "{error}");
    assert!(error.contains("scratch-cli app status"), "{error}");

    let error = run_err(&["app", "--data-dir", data_dir, "type", "only-an-id"]);
    assert!(error.contains("unknown app command"), "{error}");
}

#[test]
fn chunked_reply_is_read() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dir = tempfile::tempdir().unwrap();
    write_endpoint(dir.path(), port, &token());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let _ = read_request(&mut stream);
            let body = "{\"ok\":true,\"version\":\"9.9.9\"}";
            let (first, second) = body.split_at(10);
            let response = format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n{:x}\r\n{}\r\n0\r\n\r\n",
                first.len(),
                first,
                second.len(),
                second
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    let out = run_ok(&["app", "--data-dir", dir.path().to_str().unwrap(), "status"]);
    assert!(out.contains("version: 9.9.9"), "{out}");
}

#[test]
fn focus_sends_the_id_and_prints_the_resulting_state() {
    let app = App::start(token(), |_| (200, json!({ "ok": true, "focused": true })));
    assert_eq!(
        run_ok(&["app", "--data-dir", app.path(), "focus", "card-1"]),
        "focused\n"
    );
    let requests = app.take_requests();
    assert_eq!(requests[0].command, "focus");
    assert_eq!(requests[0].body, json!({ "id": "card-1" }));

    let app = App::start(token(), |_| (200, json!({ "ok": true, "focused": false })));
    assert_eq!(
        run_ok(&["app", "--data-dir", app.path(), "focus", "card-1"]),
        "not focused\n"
    );
    let json_out = run_ok(&["app", "--data-dir", app.path(), "focus", "card-1", "--json"]);
    assert_eq!(json_out, "{\"focused\":false,\"ok\":true}\n");
}
