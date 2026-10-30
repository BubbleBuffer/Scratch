//! Helpers shared by the sync integration tests: a temp directory, a hub
//! served over HTTP, and a recording proxy in front of it. The proxy logs the
//! bytes that cross it (so encodings and payloads are checked as sent) and
//! can hide capabilities from requests or answers, to act as an older peer.
//!
//! Each test binary compiles this module on its own, so items one binary does
//! not use are allowed.

#![allow(dead_code)]

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use vault_core::*;
use vault_sync::{
    Hub, HubServer, SyncClient, CAPABILITY_HEADER, PURGE_CAPABILITY_HEADER,
    PURGE_DEVICE_CAPABILITY_HEADER,
};

pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("vault_sync_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    pub fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    pub fn open(&self) -> Arc<Vault> {
        Arc::new(Vault::open(self.root()).expect("open"))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---- The recording proxy ----

/// One request and its answer, as they crossed the proxy.
#[derive(Clone)]
pub struct Exchange {
    pub path: String,
    /// Request headers as the client sent them.
    pub req_headers: Vec<(String, String)>,
    /// Request headers as the hub received them (after any shim).
    pub hub_headers: Vec<(String, String)>,
    /// Request body as on the wire.
    pub req_body: Vec<u8>,
    /// Answer headers as the client received them.
    pub resp_headers: Vec<(String, String)>,
    /// Answer body as on the wire.
    pub resp_body: Vec<u8>,
}

/// Switches that make the hub or the client look like an older peer.
#[derive(Clone, Default)]
pub struct Shim {
    /// Drop the zstd capability header from requests: the hub never sees it,
    /// so it behaves as one that predates the capability.
    pub hide_capability: Arc<AtomicBool>,
    /// Drop the confirmation from answers, keeping any compression.
    pub hide_confirmation: Arc<AtomicBool>,
    /// Drop the purge-protocol header from answers (the health probe's
    /// advertisement): the client then speaks v1 bodies to this hub.
    pub hide_purge: Arc<AtomicBool>,
    /// Drop the purge-device header from answers: the health probe does not
    /// advertise device ids, so the client sends no flag.
    pub hide_device: Arc<AtomicBool>,
    /// Drop the purge-device header from requests: the hub sees a client
    /// that does not ask for device ids (a capability-off v2 peer).
    pub hide_device_request: Arc<AtomicBool>,
}

pub struct Proxy {
    pub url: String,
    pub addr: SocketAddr,
    pub shim: Shim,
    pub log: Arc<Mutex<Vec<Exchange>>>,
    pub stop: Arc<AtomicBool>,
    pub accept: Option<JoinHandle<()>>,
    pub relays: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // A connection wakes the accept loop, which sees the flag and exits.
        let _ = TcpStream::connect(self.addr);
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
        let relays: Vec<JoinHandle<()>> = self.relays.lock().unwrap().drain(..).collect();
        for t in relays {
            let _ = t.join();
        }
    }
}

/// A message's start line, headers and body (by Content-Length).
type Message = (String, Vec<(String, String)>, Vec<u8>);

/// A start line and its headers, without the body.
fn read_head(r: &mut impl BufRead) -> io::Result<(String, Vec<(String, String)>)> {
    let mut start = String::new();
    r.read_line(&mut start)?;
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Ok((start.trim_end().to_string(), headers))
}

/// A client request: its body is Content-Length bytes.
pub fn read_message(r: &mut impl BufRead) -> io::Result<Message> {
    let (start, headers) = read_head(r)?;
    let len = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0usize);
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok((start, headers, body))
}

/// An upstream answer. A chunked body (tiny_http sends large answers that
/// way) is decoded, and the answer is normalised to Content-Length, so the
/// logged body is the entity body the caller's `SyncReport` counts.
pub fn read_answer(r: &mut impl BufRead) -> io::Result<Message> {
    let (start, mut headers) = read_head(r)?;
    let chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")
    });
    if !chunked {
        let len = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0usize);
        let mut body = vec![0u8; len];
        r.read_exact(&mut body)?;
        return Ok((start, headers, body));
    }
    let mut body = Vec::new();
    loop {
        let mut size = String::new();
        r.read_line(&mut size)?;
        // The size line is hex, optionally followed by `;extensions`.
        let size = size.trim_end().split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size, 16)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad chunk size"))?;
        if size == 0 {
            break;
        }
        let at = body.len();
        body.resize(at + size, 0);
        r.read_exact(&mut body[at..])?;
        // The CRLF that ends each chunk.
        let mut crlf = [0u8; 2];
        r.read_exact(&mut crlf)?;
    }
    // Trailers, up to the blank line.
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 || line.trim_end().is_empty() {
            break;
        }
    }
    headers.retain(|(k, _)| {
        !k.eq_ignore_ascii_case("transfer-encoding") && !k.eq_ignore_ascii_case("content-length")
    });
    headers.push(("Content-Length".to_string(), body.len().to_string()));
    Ok((start, headers, body))
}

fn relay(
    client: TcpStream,
    upstream: SocketAddr,
    shim: &Shim,
    log: &Mutex<Vec<Exchange>>,
) -> io::Result<()> {
    let mut from = BufReader::new(client.try_clone()?);
    let (start, headers, body) = read_message(&mut from)?;
    let path = start.split_whitespace().nth(1).unwrap_or("").to_string();
    let hide = shim.hide_capability.load(Ordering::SeqCst);
    let hide_device_req = shim.hide_device_request.load(Ordering::SeqCst);
    let hub_headers: Vec<(String, String)> = headers
        .iter()
        .filter(|(k, _)| {
            !(k.eq_ignore_ascii_case("connection")
                || (hide && k.eq_ignore_ascii_case(CAPABILITY_HEADER))
                || (hide_device_req && k.eq_ignore_ascii_case(PURGE_DEVICE_CAPABILITY_HEADER)))
        })
        .cloned()
        .collect();
    let mut up = TcpStream::connect(upstream)?;
    let mut head = format!("{start}\r\n");
    for (k, v) in &hub_headers {
        head += &format!("{k}: {v}\r\n");
    }
    head += "Connection: close\r\n\r\n";
    up.write_all(head.as_bytes())?;
    up.write_all(&body)?;
    let (status_line, rh, rbody) = read_answer(&mut BufReader::new(up))?;
    let hide_confirm = shim.hide_confirmation.load(Ordering::SeqCst);
    let hide_purge = shim.hide_purge.load(Ordering::SeqCst);
    let hide_device = shim.hide_device.load(Ordering::SeqCst);
    // The headers the client gets: the log records these, not the upstream's.
    let sent: Vec<(String, String)> = rh
        .into_iter()
        .filter(|(k, _)| {
            !(k.eq_ignore_ascii_case("connection")
                || (hide_confirm && k.eq_ignore_ascii_case(CAPABILITY_HEADER))
                || (hide_purge && k.eq_ignore_ascii_case(PURGE_CAPABILITY_HEADER))
                || (hide_device && k.eq_ignore_ascii_case(PURGE_DEVICE_CAPABILITY_HEADER)))
        })
        .collect();
    let mut out = format!("{status_line}\r\n");
    for (k, v) in &sent {
        out += &format!("{k}: {v}\r\n");
    }
    out += "Connection: close\r\n\r\n";
    // Logged before the answer is written: a caller that has the answer
    // always finds its entry in the log.
    log.lock().unwrap().push(Exchange {
        path,
        req_headers: headers,
        hub_headers,
        req_body: body,
        resp_headers: sent,
        resp_body: rbody.clone(),
    });
    let mut client = client;
    client.write_all(out.as_bytes())?;
    client.write_all(&rbody)?;
    Ok(())
}

pub fn start_proxy(upstream: SocketAddr) -> Proxy {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let shim = Shim::default();
    let log = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let relays = Arc::new(Mutex::new(Vec::new()));
    let (sh, lg, st, rl) = (shim.clone(), log.clone(), stop.clone(), relays.clone());
    let accept = std::thread::spawn(move || {
        for stream in listener.incoming() {
            if st.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else { break };
            let (sh, lg) = (sh.clone(), lg.clone());
            let t = std::thread::spawn(move || {
                let _ = relay(stream, upstream, &sh, &lg);
            });
            rl.lock().unwrap().push(t);
        }
    });
    Proxy {
        url: format!("http://{addr}"),
        addr,
        shim,
        log,
        stop,
        accept: Some(accept),
        relays,
    }
}

/// A hub served over HTTP, a proxy in front of it, and devices syncing
/// through the proxy.
pub struct Net {
    pub hub_dir: TempDir,
    pub hub_vault: Arc<Vault>,
    pub hub: Arc<Hub>,
    pub server: HubServer,
    pub proxy: Proxy,
}

pub struct Device {
    pub dir: TempDir,
    pub token: String,
    pub client: SyncClient,
}

impl Net {
    pub fn new() -> Net {
        Net::on(TempDir::new("hub"))
    }

    /// A hub on the vault in `hub_dir`, which may already hold pairings and
    /// pages from an earlier hub.
    pub fn on(hub_dir: TempDir) -> Net {
        let hub_vault = hub_dir.open();
        let hub = Hub::new(hub_vault.clone()).unwrap();
        let server = hub.serve("127.0.0.1:0").unwrap();
        let proxy = start_proxy(server.addr());
        Net {
            hub_dir,
            hub_vault,
            hub,
            server,
            proxy,
        }
    }

    pub fn device(&self, label: &str) -> Device {
        let dir = TempDir::new(label);
        let token = self.hub.pair_legacy_token(label).unwrap();
        let client = SyncClient::new(dir.open(), &self.proxy.url, &token);
        Device { dir, token, client }
    }

    /// Every exchange logged after the first `from` of them.
    pub fn since(&self, from: usize) -> Vec<Exchange> {
        self.proxy.log.lock().unwrap()[from..].to_vec()
    }

    pub fn count(&self) -> usize {
        self.proxy.log.lock().unwrap().len()
    }

    /// Stops the hub and the proxy and opens the hub's vault again, as after
    /// a restart. Pairing tokens live in the vault, so the devices' tokens
    /// still work. The caller serves the returned hub again.
    pub fn shutdown_hub(self) -> TempDir {
        let Net {
            hub_dir,
            hub_vault,
            hub,
            server,
            proxy,
        } = self;
        drop(proxy);
        hub_vault.flush().unwrap();
        drop(server);
        drop(hub);
        drop(hub_vault);
        hub_dir
    }
}

impl Device {
    pub fn v(&self) -> &Vault {
        self.client.vault()
    }

    pub fn sync(&self) -> vault_sync::SyncReport {
        self.client.sync_once().unwrap()
    }
}

pub fn header<'a>(h: &'a [(String, String)], name: &str) -> Option<&'a str> {
    h.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// The sync exchanges: every exchange but the health probe, whose body is
/// not a protocol message.
pub fn syncs(ex: &[Exchange]) -> Vec<&Exchange> {
    // The health probe and the device-key handshake are not sync exchanges.
    ex.iter()
        .filter(|e| e.path != "/health" && !e.path.starts_with("/auth/"))
        .collect()
}

/// Wire bytes (request, answer) of some exchanges.
/// The device-key handshake (`/auth/*`) is not counted in sync reports.
pub fn wire_totals(ex: &[Exchange]) -> (u64, u64) {
    ex.iter().filter(|e| !e.path.starts_with("/auth/")).fold((0, 0), |(up, down), e| {
        (
            up + e.req_body.len() as u64,
            down + e.resp_body.len() as u64,
        )
    })
}

pub fn decode(body: &[u8]) -> Vec<u8> {
    zstd::stream::decode_all(body).expect("a valid zstd frame")
}

/// A request body as the protocol reads it: decoded when the client sent it
/// zstd-compressed.
pub fn request_message(e: &Exchange) -> Vec<u8> {
    if header(&e.req_headers, "Content-Encoding") == Some("zstd") {
        decode(&e.req_body)
    } else {
        e.req_body.clone()
    }
}

/// An answer body as the protocol reads it: decoded when compressed.
pub fn answer_message(e: &Exchange) -> Vec<u8> {
    if header(&e.resp_headers, "Content-Encoding") == Some("zstd") {
        decode(&e.resp_body)
    } else {
        e.resp_body.clone()
    }
}
