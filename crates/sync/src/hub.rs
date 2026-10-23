//! The hub: serves one vault to paired devices. [`Hub::handle`] answers a
//! transport-free [`HubRequest`]; [`Hub::serve`] runs it over plain HTTP
//! (tiny_http), one thread per request.
//!
//! Routes (all but `/health` need `Authorization: Bearer <token>` and the
//! client vault's device id in `X-Device`):
//! - `POST /sync`: [`SyncRequest`] -> [`SyncResponse`]
//! - `POST /push`: [`PushRequest`] -> [`PushResponse`]
//! - `GET /changes?since=<epoch>:<seq>&wait=<ms>`: long-poll ->
//!   [`ChangesResponse`]
//! - `GET /blobs/<id>`, `PUT /blobs/<id>` (name in `X-Name`)
//! - `GET /health`: `vault-hub`
//! - `GET /open/<page id>[?block=<block id>]`: the bounce page to
//!   `notes://page/...`, with no token (see [`crate::open`])
//!
//! `/sync` and `/push` also take the `zstd-v1` capability (see
//! [`crate::encoding`]): a request that advertises it gets the confirmation
//! header, and a compressed answer when that helps.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tiny_http::{Header, Request, Response, Server};
use vault_core::{version_includes, Change, PageId, PageUpdate, Vault, VaultError};

use crate::auth::{PairedDevice, Tokens};
use crate::encoding;
use crate::wire::{
    ChangesResponse, PageBytes, PushRequest, PushRequestV2, PushResponse, SyncRequest,
    SyncRequestV2, SyncResponse, SyncResponseV2, PURGE_PROTOCOL,
};
use crate::{
    pct_decode, pct_encode, random_u64, Result, SyncError, DEVICE_HEADER, NAME_HEADER,
    PURGE_CAPABILITY, PURGE_CAPABILITY_HEADER, PURGE_DEVICE_CAPABILITY,
    PURGE_DEVICE_CAPABILITY_HEADER,
};

/// Largest request body accepted (attachments are sent whole).
const MAX_BODY: u64 = 512 << 20;
/// Longest long-poll wait.
const MAX_WAIT_MS: u64 = 60_000;
/// Budget of a sync response when the request sets none.
const DEFAULT_BUDGET: u64 = 8 << 20;

/// A hub serving one vault. Create it with [`Hub::new`], pair devices with
/// [`Hub::pair`] and start serving with [`Hub::serve`].
pub struct Hub {
    vault: Arc<Vault>,
    tokens: Tokens,
    epoch: u64,
    device: String,
}

/// A running server; [`HubServer::stop`] (or dropping it) stops accepting
/// requests and ends long-polls.
pub struct HubServer {
    addr: SocketAddr,
    server: Arc<Server>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl HubServer {
    /// The address the server listens on (with the real port if 0 was
    /// asked for).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for HubServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// One request to the hub, independent of the HTTP server carrying it
/// (the tiny_http server here, or `notes-hub`'s axum server).
pub struct HubRequest<'a> {
    /// `GET`, `POST`, `PUT`, ...
    pub method: String,
    /// The URL path, for example `/changes`.
    pub path: String,
    /// The query string without `?` (empty if none).
    pub query: String,
    /// Request headers; names match case-insensitively.
    pub headers: Vec<(String, String)>,
    /// The body, read only once the request is authorized (and capped at
    /// 512 MB).
    pub body: Box<dyn Read + 'a>,
    /// The body's length when the transport knows it (`Content-Length`).
    pub body_len: Option<u64>,
    /// When set, ends a long-poll early (a stopping server).
    pub cancel: Option<&'a AtomicBool>,
}

impl HubRequest<'_> {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Reads the body, decompressed when `body_zstd` (the negotiation has
    /// already been checked by [`encoding::request_negotiation`]). Both the
    /// wire body and the decoded body are capped at 512 MB.
    fn read_body(&mut self, body_zstd: bool) -> Result<Vec<u8>> {
        if self.body_len.is_some_and(|n| n > MAX_BODY) {
            return Err(SyncError::Status(413, "request too large".into()));
        }
        let mut body = Vec::new();
        (&mut self.body).take(MAX_BODY + 1).read_to_end(&mut body)?;
        if body.len() as u64 > MAX_BODY {
            return Err(SyncError::Status(413, "request too large".into()));
        }
        count_bytes!("hub.request.wire_bytes", body.len());
        if body_zstd {
            return measure!(
                "hub.request.zstd_decompress",
                encoding::decompress(&body, MAX_BODY)
            );
        }
        Ok(body)
    }
}

/// The hub's answer to a [`HubRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubResponse {
    pub status: u16,
    pub content_type: String,
    /// Other headers (`X-Name` on attachments).
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HubResponse {
    fn ok(body: Vec<u8>) -> HubResponse {
        HubResponse {
            status: 200,
            content_type: "application/octet-stream".into(),
            headers: Vec::new(),
            body,
        }
    }
    fn error(e: &SyncError) -> HubResponse {
        let status = match e {
            SyncError::Status(s, _) => *s,
            SyncError::Protocol(_) => 400,
            SyncError::Vault(m) if m.starts_with("not found") => 404,
            _ => 500,
        };
        let msg = match e {
            SyncError::Status(_, m) => m.clone(),
            e => e.to_string(),
        };
        HubResponse {
            status,
            content_type: "text/plain; charset=utf-8".into(),
            headers: Vec::new(),
            body: msg.into_bytes(),
        }
    }
}

/// The answer to `GET /open/<page>` with `query` (see [`crate::open`]).
fn open_response(page: &str, query: &str) -> HubResponse {
    match crate::open::bounce_page(page, crate::open::block_query(query).as_deref()) {
        Ok(html) => HubResponse {
            status: 200,
            content_type: crate::open::CONTENT_TYPE.into(),
            headers: vec![("Cache-Control".into(), crate::open::CACHE_CONTROL.into())],
            body: html.into_bytes(),
        },
        Err(msg) => HubResponse {
            status: 400,
            content_type: "text/plain; charset=utf-8".into(),
            headers: Vec::new(),
            body: format!("{msg}\n").into_bytes(),
        },
    }
}

/// The answer to a `/sync` or `/push` request: confirms the zstd capability
/// and compresses the body when the request advertised it.
fn answer(body: Vec<u8>, advertised: bool) -> Result<HubResponse> {
    let (body, headers) = measure!(
        "hub.response.zstd_compress",
        encoding::answer(body, advertised)?
    );
    count_bytes!("hub.response.wire_bytes", body.len());
    Ok(HubResponse {
        headers,
        ..HubResponse::ok(body)
    })
}

/// Reads `X-Purge-Device` before any body is read: absent means off, exactly
/// one `1` means on. Any other value, or a repeated header, is refused, so
/// the layout is never guessed.
fn purge_device_flag(headers: &[(String, String)]) -> Result<bool> {
    let values: Vec<&str> = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case(PURGE_DEVICE_CAPABILITY_HEADER))
        .map(|(_, v)| v.trim())
        .collect();
    match values.as_slice() {
        [] => Ok(false),
        [v] if *v == PURGE_DEVICE_CAPABILITY => Ok(true),
        _ => Err(SyncError::Protocol(format!(
            "malformed {PURGE_DEVICE_CAPABILITY_HEADER}"
        ))),
    }
}

fn not_found(e: VaultError) -> SyncError {
    match e {
        VaultError::NotFound(m) => SyncError::Status(404, m),
        e => e.into(),
    }
}

impl Hub {
    /// A hub for `vault`, with the pairing tokens kept in the vault
    /// (`.vault/hub/tokens.json`).
    pub fn new(vault: Arc<Vault>) -> Result<Arc<Hub>> {
        let tokens = Tokens::open(&vault.root())?;
        Ok(Arc::new(Hub {
            device: vault.device_id(),
            vault,
            tokens,
            epoch: random_u64()?,
        }))
    }

    pub fn vault(&self) -> &Arc<Vault> {
        &self.vault
    }

    /// Creates a pairing token for a device named `label`. The token is
    /// shown once (only its hash is stored); the device's `SyncClient`
    /// uses it.
    pub fn pair(&self, label: &str) -> Result<String> {
        self.tokens.pair(label)
    }

    /// After a login: a token for the vault with device id `device`,
    /// named `name` (made unique with a ` (2)`-style suffix), replacing the
    /// device's earlier tokens. Returns the token (shown once) and the
    /// label used.
    pub fn login_device(&self, device: &str, name: &str) -> Result<(String, String)> {
        self.tokens.login(name, device, &self.device)
    }

    /// Drops `token` (a device signing out). Returns whether it existed.
    pub fn sign_out(&self, token: &str) -> Result<bool> {
        self.tokens.revoke(token)
    }

    /// Revokes the tokens labelled `label`; returns how many.
    pub fn unpair(&self, label: &str) -> Result<usize> {
        self.tokens.unpair(label)
    }

    pub fn devices(&self) -> Vec<PairedDevice> {
        self.tokens.devices()
    }

    /// Marks `device` as a host (it connected to the host relay).
    pub fn mark_host(&self, device: &str) -> Result<()> {
        self.tokens.mark_host(device)
    }

    /// Renames `device`; the name must be unique among hosts, ignoring
    /// case. Returns the name used.
    pub fn rename_device(&self, device: &str, name: &str) -> Result<String> {
        self.tokens.rename(device, name)
    }

    /// Serves on `addr` (for example `100.101.102.103:8787`, or
    /// `127.0.0.1:0` for any free port) until the returned server is
    /// stopped or dropped.
    pub fn serve(self: &Arc<Self>, addr: &str) -> Result<HubServer> {
        let server =
            Server::http(addr).map_err(|e| SyncError::Io(format!("listen {addr}: {e}")))?;
        let local = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| SyncError::Io("not an IP address".into()))?;
        let server = Arc::new(server);
        let stopping = Arc::new(AtomicBool::new(false));
        let (hub, srv, stop) = (self.clone(), server.clone(), stopping.clone());
        let thread = std::thread::Builder::new()
            .name("vault-hub".into())
            .spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let req = match srv.recv() {
                        Ok(r) => r,
                        Err(_) => break,
                    };
                    let (hub, stop) = (hub.clone(), stop.clone());
                    let _ = std::thread::Builder::new()
                        .name("vault-hub-req".into())
                        .spawn(move || hub.respond(req, &stop));
                }
            })?;
        Ok(HubServer {
            addr: local,
            server,
            stopping,
            thread: Some(thread),
        })
    }

    /// Adapts a tiny_http request to [`Hub::handle`].
    fn respond(&self, mut req: Request, stopping: &AtomicBool) {
        let url = req.url().to_string();
        let (path, query) = url.split_once('?').unwrap_or((&url, ""));
        let headers = req
            .headers()
            .iter()
            .map(|h| {
                (
                    h.field.as_str().as_str().to_string(),
                    h.value.as_str().to_string(),
                )
            })
            .collect();
        let method = req.method().as_str().to_string();
        let body_len = req.body_length().map(|n| n as u64);
        let reply = self.handle(HubRequest {
            method,
            path: path.to_string(),
            query: query.to_string(),
            headers,
            body: Box::new(req.as_reader()),
            body_len,
            cancel: Some(stopping),
        });
        let mut resp = Response::from_data(reply.body).with_status_code(reply.status);
        let content_type = ("Content-Type".to_string(), reply.content_type);
        for (k, v) in std::iter::once(content_type).chain(reply.headers) {
            if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
                resp = resp.with_header(h);
            }
        }
        let _ = req.respond(resp);
    }

    /// Answers one request: routing, the token and device check, and the
    /// sync handlers. Blocking (a `/changes` long-poll waits up to 60 s),
    /// so async servers call it on a blocking thread.
    pub fn handle(&self, req: HubRequest<'_>) -> HubResponse {
        match measure!("hub.handle", self.route(req)) {
            Ok(r) => r,
            Err(e) => HubResponse::error(&e),
        }
    }

    fn route(&self, mut req: HubRequest<'_>) -> Result<HubResponse> {
        let method = req.method.to_ascii_uppercase();
        let path = req.path.clone();
        if method == "GET" && path == "/health" {
            let mut response = HubResponse::ok(b"vault-hub".to_vec());
            response
                .headers
                .push((PURGE_CAPABILITY_HEADER.into(), PURGE_CAPABILITY.into()));
            response.headers.push((
                PURGE_DEVICE_CAPABILITY_HEADER.into(),
                PURGE_DEVICE_CAPABILITY.into(),
            ));
            return Ok(response);
        }
        // The bounce page for `notes://` links: public, like /health, and
        // one segment only (the same route as notes-hub's axum router).
        if method == "GET" {
            if let Some(page) = path
                .strip_prefix("/open/")
                .filter(|p| !p.is_empty() && !p.contains('/'))
            {
                return Ok(open_response(&pct_decode(page), &req.query));
            }
        }
        let token = req
            .header("Authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| SyncError::Status(401, "missing token".into()))?
            .trim()
            .to_string();
        let device = req.header(DEVICE_HEADER).unwrap_or("").to_string();
        measure!(
            "hub.auth.check",
            self.tokens.check(&token, &device, &self.device)?
        );
        // The negotiation is checked before any body is read or decoded.
        match (method.as_str(), path.as_str()) {
            ("POST", "/sync") => {
                let n = encoding::request_negotiation(&req.headers, true)?;
                let device = purge_device_flag(&req.headers)?;
                let body = req.read_body(n.body_zstd)?;
                if body.first() == Some(&PURGE_PROTOCOL) {
                    // Only a protocol v2 body has purge records to carry ids.
                    let request = measure!(
                        "hub.request.decode",
                        SyncRequestV2::decode_with_device(&body, device)?
                    );
                    let r = measure!("hub.sync", self.sync_v2(request)?);
                    return answer(
                        measure!("hub.response.encode", r.encode_with_device(device)),
                        n.advertised,
                    );
                }
                if device {
                    return Err(SyncError::Protocol(format!(
                        "{PURGE_DEVICE_CAPABILITY_HEADER} needs a protocol {PURGE_PROTOCOL} body"
                    )));
                }
                let request = measure!("hub.request.decode", SyncRequest::decode(&body)?);
                let r = measure!("hub.sync", self.sync(request)?);
                answer(measure!("hub.response.encode", r.encode()), n.advertised)
            }
            ("POST", "/push") => {
                let n = encoding::request_negotiation(&req.headers, true)?;
                let body = req.read_body(n.body_zstd)?;
                if body.first() == Some(&PURGE_PROTOCOL) {
                    let r = self.push_v2(PushRequestV2::decode(&body)?)?;
                    return answer(r.encode(), n.advertised);
                }
                let r = self.push(PushRequest::decode(&body)?)?;
                answer(r.encode(), n.advertised)
            }
            ("GET", "/changes") => {
                let never = AtomicBool::new(false);
                let stopping = req.cancel.unwrap_or(&never);
                Ok(HubResponse::ok(self.changes(&req.query, stopping).encode()))
            }
            ("GET", p) if p.starts_with("/blobs/") => self.get_blob(&p[7..]),
            ("PUT", p) if p.starts_with("/blobs/") => {
                // Attachments are never compressed.
                encoding::request_negotiation(&req.headers, false)?;
                let name = req.header(NAME_HEADER).map(pct_decode).unwrap_or_default();
                let body = req.read_body(false)?;
                self.vault
                    .store_attachment(p[7..].to_string(), name, body)
                    .map_err(|e| match e {
                        VaultError::Invalid(m) => SyncError::Status(400, m),
                        e => e.into(),
                    })?;
                Ok(HubResponse::ok(Vec::new()))
            }
            _ => Err(SyncError::Status(404, format!("no route {method} {path}"))),
        }
    }

    /// The hub's current change feed position.
    fn seq(&self) -> u32 {
        self.vault.poll_sync_changes(u32::MAX).seq
    }

    fn sync(&self, req: SyncRequest) -> Result<SyncResponse> {
        let v = &self.vault;
        let seq = self.seq();
        if !req.purged.is_empty() {
            v.import_purged(req.purged.clone())?;
        }
        let mut resp = SyncResponse {
            epoch: self.epoch,
            seq,
            ..SyncResponse::default()
        };
        // Pages changed here since the client's base (None: compare all).
        let mut changed: Option<HashSet<PageId>> = None;
        if !req.full {
            let batch = match req.base {
                Some((epoch, since)) if epoch == self.epoch && since <= seq => {
                    Some(v.poll_sync_changes(since))
                }
                _ => None,
            };
            match batch {
                Some(b) if !b.reset => {
                    let ids = b
                        .changes
                        .into_iter()
                        .filter_map(|c| match c {
                            Change::PageChanged(id) | Change::PageCreated(id) => Some(id),
                            _ => None,
                        })
                        .collect();
                    changed = Some(ids);
                }
                _ => {
                    resp.reset = true;
                    return Ok(resp);
                }
            }
        }
        let purged_all = v.purged_pages();
        let known = (req.hub_purged_known as usize).min(purged_all.len());
        resp.purged = purged_all[known..].to_vec();
        resp.purged_total = purged_all.len() as u32;
        let mine: HashMap<PageId, Vec<u8>> =
            measure!("hub.sync.page_versions", v.sync_page_versions()?)
                .into_iter()
                .map(|p| (p.page, p.version))
                .collect();
        let theirs: HashMap<&str, &[u8]> = req
            .versions
            .iter()
            .map(|(p, b)| (p.as_str(), b.as_slice()))
            .collect();
        // Pages to send: where the client's version lacks something here.
        let mut to_send: Vec<(&String, &[u8])> = Vec::new();
        match &changed {
            None => {
                for (id, hv) in &mine {
                    let cv = theirs.get(id.as_str()).copied().unwrap_or(&[]);
                    if !version_includes(cv, hv)? {
                        to_send.push((id, cv));
                    }
                }
            }
            Some(ids) => {
                for (id, cv) in &req.versions {
                    if let Some(hv) = mine.get(id) {
                        if !version_includes(cv, hv)? {
                            to_send.push((id, cv.as_slice()));
                        }
                    }
                }
                let mut extra: PageBytes = Vec::new();
                for id in ids {
                    if theirs.contains_key(id.as_str()) {
                        continue;
                    }
                    if let Some(hv) = mine.get(id) {
                        extra.push((id.clone(), hv.clone()));
                    }
                }
                extra.sort();
                resp.changed = extra;
            }
        }
        to_send.sort();
        let budget = if req.budget == 0 {
            DEFAULT_BUDGET
        } else {
            req.budget
        };
        let mut used = 0u64;
        for (id, cv) in to_send {
            if used >= budget {
                resp.more = true;
                break;
            }
            let bytes = match measure!(
                "hub.sync.export_updates",
                v.export_updates(id.clone(), cv.to_vec())
            ) {
                Ok(b) => b,
                // Purged since the versions were read.
                Err(VaultError::NotFound(_)) => continue,
                Err(e) => return Err(e.into()),
            };
            count_bytes!("hub.sync.export_bytes", bytes.len());
            used += bytes.len() as u64 + id.len() as u64;
            resp.updates.push((id.clone(), bytes));
            resp.versions.push((id.clone(), mine[id].clone()));
        }
        // Pages to receive: where the client has something this lacks.
        for (id, cv) in &req.versions {
            if cv.is_empty() {
                continue;
            }
            let hv = mine.get(id).map(Vec::as_slice).unwrap_or(&[]);
            if !version_includes(hv, cv)? {
                resp.need.push((id.clone(), hv.to_vec()));
            }
        }
        for b in &req.blobs {
            if v.attachment_path(b.clone()).is_err() {
                resp.need_blobs.push(b.clone());
            }
        }
        Ok(resp)
    }

    /// V2 exchanges active tombstones and applied restoration proofs before
    /// page data. Unknown proofs always request the editor's full history.
    fn sync_v2(&self, mut req: SyncRequestV2) -> Result<SyncResponseV2> {
        let v = &self.vault;
        v.import_purge_records(req.purges)?;
        req.request.purged.clear();
        let mut response = self.sync(req.request.clone())?;
        if response.reset {
            return Ok(SyncResponseV2 {
                response,
                ..Default::default()
            });
        }
        let mut incoming_restore = HashSet::new();
        for record in &req.restores {
            if v.needs_restore_snapshot(record)? {
                incoming_restore.insert(record.page.clone());
                response.need.retain(|(id, _)| id != &record.page);
                response.need.push((record.page.clone(), Vec::new()));
            }
        }
        // Sending a purge-side delta before receiving an authoritative
        // snapshot would contaminate the editing device's whole page.
        response
            .updates
            .retain(|(id, _)| !incoming_restore.contains(id));
        response
            .versions
            .retain(|(id, _)| !incoming_restore.contains(id));
        response
            .changed
            .retain(|(id, _)| !incoming_restore.contains(id));

        let purges = v.purge_records();
        let sync_versions: HashMap<_, _> = v
            .sync_page_versions()?
            .into_iter()
            .map(|version| (version.page, version.version))
            .collect();
        for (id, version) in &req.request.versions {
            if version.is_empty() || incoming_restore.contains(id) {
                continue;
            }
            if let Some(purge) = purges.iter().find(|p| &p.page == id) {
                let stale = match &purge.version {
                    Some(pv) => version_includes(pv, version)?,
                    None => false,
                };
                // Conflict/peer operations beyond a tombstone are retained
                // metadata, too. Once the archive covers the supplied version,
                // asking for another full snapshot cannot add anything.
                let already_known = match sync_versions.get(id) {
                    Some(known) => version_includes(known, version)?,
                    None => false,
                };
                if !stale && !already_known {
                    response.need.retain(|(page, _)| page != id);
                    response.need.push((id.clone(), Vec::new()));
                }
            }
        }
        let mut restores = Vec::new();
        let budget = if req.request.budget == 0 {
            DEFAULT_BUDGET
        } else {
            req.request.budget
        };
        for record in v.restore_records() {
            // Historical proofs remain durable across re-purges. An archive
            // snapshot is metadata transport, never a new restoration proof.
            if purges.iter().any(|purge| purge.page == record.page)
                || incoming_restore.contains(&record.page)
                || req.restores.contains(&record)
            {
                continue;
            }
            response.updates.retain(|(id, _)| id != &record.page);
            response.versions.retain(|(id, _)| id != &record.page);
            let used: u64 = response
                .updates
                .iter()
                .map(|(id, bytes)| (id.len() + bytes.len()) as u64)
                .sum();
            if !response.updates.is_empty() && used >= budget {
                response.more = true;
                continue;
            }
            let bytes = match v.export_updates(record.page.clone(), Vec::new()) {
                Ok(bytes) => bytes,
                Err(VaultError::NotFound(_)) => continue,
                Err(e) => return Err(e.into()),
            };
            let version = v
                .sync_page_versions()?
                .into_iter()
                .find(|p| p.page == record.page)
                .ok_or_else(|| SyncError::Protocol("restored page disappeared".into()))?
                .version;
            response.updates.push((record.page.clone(), bytes));
            response.versions.push((record.page.clone(), version));
            restores.push(record);
        }
        response.purged.clear();
        response.purged_total = 0;
        Ok(SyncResponseV2 {
            response,
            purges,
            restores,
        })
    }

    fn push_v2(&self, req: PushRequestV2) -> Result<PushResponse> {
        let mut result = PushResponse::default();
        let mut records: HashMap<_, _> = req
            .restores
            .into_iter()
            .map(|record| (record.page.clone(), record))
            .collect();
        for (page, bytes) in req.request.updates {
            let r = match records.remove(&page) {
                Some(record) => self.vault.import_restored_updates(record, bytes)?,
                None => self.vault.import_updates(page, bytes)?,
            };
            result.created += r.created;
            result.changed += r.changed;
            result.conflicts += r.conflicts;
        }
        if !records.is_empty() {
            return Err(SyncError::Protocol(
                "restore proof has no full snapshot".into(),
            ));
        }
        Ok(result)
    }

    fn push(&self, req: PushRequest) -> Result<PushResponse> {
        let r = self.vault.import_updates_batch_legacy(
            req.updates
                .into_iter()
                .map(|(page, bytes)| PageUpdate { page, bytes })
                .collect(),
        )?;
        Ok(PushResponse {
            created: r.created,
            changed: r.changed,
            conflicts: r.conflicts,
        })
    }

    fn changes(&self, query: &str, stopping: &AtomicBool) -> ChangesResponse {
        let mut since = None;
        let mut wait = 25_000u64;
        for kv in query.split('&') {
            match kv.split_once('=') {
                Some(("since", s)) => {
                    if let Some((e, q)) = s.split_once(':') {
                        if let (Ok(e), Ok(q)) = (e.parse::<u64>(), q.parse::<u32>()) {
                            since = Some((e, q));
                        }
                    }
                }
                Some(("wait", w)) => wait = w.parse().unwrap_or(wait).min(MAX_WAIT_MS),
                _ => {}
            }
        }
        let now = ChangesResponse {
            epoch: self.epoch,
            seq: self.seq(),
        };
        let Some((epoch, seq)) = since else {
            return now;
        };
        if epoch != self.epoch || seq != now.seq {
            return now;
        }
        // Wait in slices, so a stopping server ends its long-polls.
        let deadline = Instant::now() + Duration::from_millis(wait);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || stopping.load(Ordering::SeqCst) {
                break;
            }
            let slice = left.min(Duration::from_millis(250));
            let b = self.vault.wait_sync_changes(seq, slice.as_millis() as u32);
            if b.seq != seq {
                break;
            }
        }
        ChangesResponse {
            epoch: self.epoch,
            seq: self.seq(),
        }
    }

    fn get_blob(&self, id: &str) -> Result<HubResponse> {
        let path = self
            .vault
            .attachment_path(id.to_string())
            .map_err(not_found)?;
        let info = self
            .vault
            .attachment_info(id.to_string())
            .map_err(not_found)?;
        let body = std::fs::read(path)?;
        Ok(HubResponse {
            status: 200,
            content_type: info.mime,
            headers: vec![(NAME_HEADER.to_string(), pct_encode(&info.name))],
            body,
        })
    }
}
