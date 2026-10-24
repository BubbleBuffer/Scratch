//! The sync client: `sync_once` runs the protocol against a hub; `start`
//! runs it in the background after local changes (debounced), on hub
//! notifications (long-poll) and periodically.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use vault_core::{version_includes, PageId, PageUpdate, Vault, VaultError};

use crate::encoding;
use crate::tls::{self, Trust};
use crate::wire::{
    ChangesResponse, PageBytes, PushRequest, PushRequestV2, PushResponse, SyncRequest,
    SyncRequestV2, SyncResponse, SyncResponseV2,
};
use crate::{
    now_ms, pct_decode, pct_encode, Result, SyncError, CAPABILITY_HEADER, DEVICE_HEADER,
    NAME_HEADER, PURGE_CAPABILITY, PURGE_CAPABILITY_HEADER, PURGE_DEVICE_CAPABILITY,
    PURGE_DEVICE_CAPABILITY_HEADER, ZSTD_CAPABILITY,
};

/// Bytes of updates per response and per push.
const BUDGET: u64 = 8 << 20;
/// Largest response body read (attachments come whole).
const MAX_BODY: u64 = 512 << 20;
/// Request rounds of one `sync_once` before giving up.
const MAX_ROUNDS: u32 = 10_000;

/// What one `sync_once` did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncReport {
    /// Every page's version was compared (first sync, or the hub restarted).
    pub full: bool,
    /// `POST /sync` requests made.
    pub rounds: u32,
    /// Pages new here, from the hub.
    pub created: u32,
    /// Pages here that received changes from the hub.
    pub changed: u32,
    /// Conflicts recorded here.
    pub conflicts: u32,
    /// Pages whose changes were sent to the hub.
    pub pushed: u32,
    /// Pages deleted here because they were purged elsewhere.
    pub purged: u32,
    /// Attachments uploaded to the hub.
    pub blobs_sent: u32,
    /// Bytes of response bodies received, as sent on the wire (compressed
    /// when the hub compressed them).
    pub bytes_down: u64,
    /// Bytes of request bodies sent, as on the wire (compressed when
    /// compressed).
    pub bytes_up: u64,
}

/// Timings of the background loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncOptions {
    /// Quiet time after a local change before syncing.
    pub debounce_ms: u64,
    /// Longest wait during continuous local changes.
    pub max_delay_ms: u64,
    /// Sync this often even without notifications.
    pub interval_ms: u64,
    /// How long each hub long-poll waits.
    pub long_poll_ms: u64,
    /// Download attachments missing here after each sync (else only on
    /// `fetch_attachment`).
    pub prefetch_attachments: bool,
}

impl Default for SyncOptions {
    fn default() -> Self {
        SyncOptions {
            debounce_ms: 500,
            max_delay_ms: 3000,
            interval_ms: 60_000,
            long_poll_ms: 25_000,
            prefetch_attachments: false,
        }
    }
}

/// The state of a background sync.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStatus {
    pub syncing: bool,
    /// When the last sync succeeded (ms since the epoch).
    pub last_ok_ms: Option<i64>,
    /// The last sync's error, cleared by the next success.
    pub last_error: Option<String>,
    pub last_report: Option<SyncReport>,
    pub syncs: u64,
}

/// Where the last complete sync left off.
#[derive(Clone, Copy)]
struct Base {
    hub_epoch: u64,
    hub_seq: u32,
    hub_purged: u32,
    local_purged: usize,
}

#[derive(Default)]
struct State {
    purge_v2: bool,
    /// The hub advertised the purge device capability on this sync's health
    /// check. Only meaningful with `purge_v2`: purge records then carry
    /// their device ids on `/sync`.
    purge_device: bool,
    /// One warning for this client's fixed hub connection, including retries
    /// and full-sync resets. A new client connection warns again.
    warned_legacy_hub: bool,
    /// The hub confirmed the zstd capability in this `sync_once`. Reset at
    /// each `sync_once`, so an old hub is not remembered as one that lacks it.
    zstd: bool,
    base: Option<Base>,
    /// Attachments the hub is known to have.
    hub_blobs: HashSet<String>,
    /// Per page, a version the hub is known to include. A delta sync sends
    /// the pages whose local version this does not cover.
    known: HashMap<PageId, Vec<u8>>,
}

/// Syncs one vault with a hub. `Send + Sync`; share it behind an `Arc`.
pub struct SyncClient {
    vault: Arc<Vault>,
    http: Http,
    budget: u64,
    state: Mutex<State>,
}

/// The HTTP side, cloneable for the long-poll thread.
#[derive(Clone)]
struct Http {
    url: String,
    token: String,
    device: String,
    agent: ureq::Agent,
    /// Pinned hub certificates and the last untrusted chain, shared with
    /// the long-poll agent.
    trust: Arc<Trust>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// No pooled connections: a request never lands on a connection left
/// over from a hub that has stopped (its server threads would hold it
/// without answering), and a sync makes only a few requests.
/// TLS verifies with `trust`'s pins (see `tls`).
fn agent(timeout: Duration, trust: &Arc<Trust>) -> ureq::Agent {
    let config = ureq::config::Config::builder()
        .http_status_as_error(false)
        .max_idle_connections(0)
        .max_idle_connections_per_host(0)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(timeout))
        .build();
    tls::agent(config, trust)
}

/// The zstd capability for one request: whether it advertises it, and
/// whether its body is compressed (only after the hub confirmed it).
#[derive(Clone, Copy, Default)]
struct Zstd {
    advertise: bool,
    compress: bool,
}

/// One exchange: the answer's body (decoded), its name header, and the
/// bytes on the wire each way.
struct Reply {
    purge_v2: bool,
    purge_device: bool,
    body: Vec<u8>,
    name: Option<String>,
    /// The request body as sent (compressed or not).
    up: u64,
    /// The answer body as received (compressed or not).
    down: u64,
    /// The hub confirmed the zstd capability.
    confirmed: bool,
}

impl Http {
    /// `purge_device`: send `X-Purge-Device: 1`, for a protocol v2 `/sync`
    /// whose hub confirmed the capability on `/health`.
    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        name: Option<&str>,
        zstd: Zstd,
        purge_device: bool,
    ) -> Result<Reply> {
        let url = format!("{}{}", self.url, path);
        let auth = format!("Bearer {}", self.token);
        let compressed = match body {
            Some(b) if zstd.compress => measure!("client.request.zstd_compress", encoding::compress(b)?),
            _ => None,
        };
        let wire: &[u8] = compressed.as_deref().or(body).unwrap_or(&[]);
        let up = wire.len() as u64;
        count_bytes!("client.request.raw_bytes", body.map_or(0, |b| b.len()));
        count_bytes!("client.request.wire_bytes", up);
        let generation = self.trust.generation();
        let resp = measure!("client.http.send", match method {
            "GET" => self
                .agent
                .get(&url)
                .header("Authorization", &auth)
                .header(DEVICE_HEADER, &self.device)
                .call(),
            m => {
                let mut rb = if m == "PUT" {
                    self.agent.put(&url)
                } else {
                    self.agent.post(&url)
                };
                rb = rb
                    .header("Authorization", &auth)
                    .header(DEVICE_HEADER, &self.device)
                    .header("Content-Type", "application/octet-stream");
                if zstd.advertise {
                    rb = rb.header(CAPABILITY_HEADER, ZSTD_CAPABILITY);
                }
                if purge_device {
                    rb = rb.header(PURGE_DEVICE_CAPABILITY_HEADER, PURGE_DEVICE_CAPABILITY);
                }
                if compressed.is_some() {
                    rb = rb.header("Content-Encoding", "zstd");
                }
                if let Some(n) = name {
                    rb = rb.header(NAME_HEADER, pct_encode(n));
                }
                rb.send(wire)
            }
        });
        let mut resp = match resp {
            Ok(r) => r,
            // The handshake met an unpinned certificate with an unknown issuer.
            Err(e) => {
                return Err(match self.trust.untrusted_since(generation) {
                    Some(sha256) => SyncError::UntrustedCertificate { sha256 },
                    None => SyncError::Http(e.to_string()),
                })
            }
        };
        self.trust.clear();
        let status = resp.status().as_u16();
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
            .collect();
        let name = resp
            .headers()
            .get(NAME_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(pct_decode);
        let raw = measure!(
            "client.http.body_read",
            resp.body_mut()
                .with_config()
                .limit(MAX_BODY)
                .read_to_vec()
                .map_err(|e| SyncError::Http(e.to_string()))?
        );
        if status != 200 {
            return Err(SyncError::Status(
                status,
                String::from_utf8_lossy(&raw).into_owned(),
            ));
        }
        // The answer's encoding is checked before any decoding.
        let answer = encoding::answer_negotiation(&headers, zstd.advertise)?;
        let down = raw.len() as u64;
        count_bytes!("client.response.wire_bytes", down);
        let body = if answer.zstd {
            measure!("client.response.zstd_decompress", encoding::decompress(&raw, MAX_BODY)?)
        } else {
            raw
        };
        Ok(Reply {
            purge_v2: headers.iter().any(|(k, v)| {
                k.eq_ignore_ascii_case(PURGE_CAPABILITY_HEADER) && v == PURGE_CAPABILITY
            }),
            purge_device: headers.iter().any(|(k, v)| {
                k.eq_ignore_ascii_case(PURGE_DEVICE_CAPABILITY_HEADER)
                    && v == PURGE_DEVICE_CAPABILITY
            }),
            body,
            name,
            up,
            down,
            confirmed: answer.confirmed,
        })
    }

    fn changes(&self, since: Option<(u64, u32)>, wait_ms: u64) -> Result<ChangesResponse> {
        let q = match since {
            Some((e, s)) => format!("/changes?since={e}:{s}&wait={wait_ms}"),
            None => "/changes".to_string(),
        };
        let r = self.call("GET", &q, None, None, Zstd::default(), false)?;
        ChangesResponse::decode(&r.body)
    }
}

/// Local page versions as a map.
fn local_versions(v: &Vault) -> Result<HashMap<PageId, Vec<u8>>> {
    Ok(v.sync_page_versions()?
        .into_iter()
        .map(|p| (p.page, p.version))
        .collect())
}

impl SyncClient {
    /// A client syncing `vault` with the hub at `url` (for example
    /// `http://100.101.102.103:8787`), using the token from pairing.
    pub fn new(vault: Arc<Vault>, url: &str, token: &str) -> SyncClient {
        SyncClient::with_trusted(vault, url, token, Vec::new())
    }

    /// Like [`SyncClient::new`], also accepting a hub whose certificate has
    /// an unknown issuer (self-signed or a private CA) when its end-entity
    /// SHA-256 is in `trusted` (hex in any case, with or without `:`).
    /// Publicly trusted certificates are accepted as with `new`.
    pub fn with_trusted(
        vault: Arc<Vault>,
        url: &str,
        token: &str,
        trusted: Vec<String>,
    ) -> SyncClient {
        let trust = Trust::new(&trusted);
        SyncClient {
            http: Http {
                url: url.trim_end_matches('/').to_string(),
                token: token.to_string(),
                device: vault.device_id(),
                agent: agent(Duration::from_secs(120), &trust),
                trust,
            },
            vault,
            budget: BUDGET,
            state: Mutex::new(State::default()),
        }
    }

    /// Bytes of updates per response and per push (default 8 MB).
    pub fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes.max(1);
    }

    pub fn vault(&self) -> &Arc<Vault> {
        &self.vault
    }

    /// The chain (leaf first) the hub presented in the last handshake that
    /// failed with [`SyncError::UntrustedCertificate`]; cleared by the next
    /// successful request.
    pub fn untrusted_certificate(&self) -> Option<Vec<tls::CertificateInfo>> {
        self.http
            .trust
            .chain()
            .map(|c| tls::CertificateInfo::chain(&c))
    }

    /// Syncs with the hub until both have every change: sends local
    /// changes and purges, imports the hub's, and uploads attachments the
    /// hub lacks. The first sync compares every page; later ones only the
    /// pages changed on either side since. One sync runs at a time.
    pub fn sync_once(&self) -> Result<SyncReport> {
        span_guard!("client.sync_once");
        let mut st = lock(&self.state);
        // Renegotiated every sync: the first request is plain.
        st.zstd = false;
        let mut report = SyncReport::default();
        // Harmless discovery first: a v1 exchange could delete the offline
        // editor's copy before a capability advertised in its reply is read.
        let health = self
            .http
            .call("GET", "/health", None, None, Zstd::default(), false)?;
        report.bytes_up += health.up;
        report.bytes_down += health.down;
        if !health.purge_v2 && !st.warned_legacy_hub {
            tracing::warn!(
                "Purge recovery is off against this hub (protocol v1); edits to permanently deleted pages may be dropped"
            );
            st.warned_legacy_hub = true;
        }
        if st.purge_v2 != health.purge_v2 {
            st.base = None;
        }
        st.purge_v2 = health.purge_v2;
        // Device ids ride only on protocol v2, so the flag needs both.
        st.purge_device = health.purge_v2 && health.purge_device;
        let mut restarted = false;
        let mut activity_checked = false;
        loop {
            match self.run(&mut st, &mut report)? {
                true => {
                    // Desktop app open registers the device. Storage-only
                    // callers and phones do not become PC runners by syncing.
                    // Stamp activity only after a successful exchange, then
                    // send that ordinary page change in this same sync call.
                    if !activity_checked {
                        activity_checked = true;
                        let id = vault_core::node_record_id(&self.vault.device_id());
                        match self.vault.load_page(id) {
                            Ok(_) if self.vault.publish_device_activity()? => continue,
                            Ok(_) | Err(vault_core::VaultError::NotFound(_)) => {}
                            Err(e) => return Err(e.into()),
                        }
                    }
                    return Ok(report);
                }
                false if !restarted => {
                    // The hub restarted or our base is too old.
                    st.base = None;
                    restarted = true;
                }
                false => {
                    return Err(SyncError::Protocol(
                        "the hub asked for a full sync twice".into(),
                    ))
                }
            }
        }
    }

    /// One sync from `st.base`; false if the hub answered `reset`.
    fn run(&self, st: &mut State, report: &mut SyncReport) -> Result<bool> {
        let v = &self.vault;
        let local_purged = v.purged_pages();
        let base = st.base;
        // Pages to compare in the first round (None: all): those with
        // local changes the hub is not known to have.
        let mut pages: Option<HashSet<PageId>> = None;
        if base.is_some() {
            let mut ids = HashSet::new();
            for (id, lv) in local_versions(v)? {
                let covered = match st.known.get(&id) {
                    Some(k) => *k == lv || version_includes(k, &lv)?,
                    None => false,
                };
                if !covered {
                    ids.insert(id);
                }
            }
            pages = Some(ids);
        }
        let full = pages.is_none();
        if full {
            st.hub_blobs.clear();
            st.known.clear();
        }
        report.full |= full;
        let mut purged = match (&base, full) {
            (Some(b), false) => local_purged[b.local_purged.min(local_purged.len())..].to_vec(),
            _ => local_purged.clone(),
        };
        let mut blobs: Vec<String> = v
            .attachment_ids()?
            .into_iter()
            .filter(|b| !st.hub_blobs.contains(b))
            .collect();
        let mut hub_purged = match (&base, full) {
            (Some(b), false) => b.hub_purged,
            _ => 0,
        };
        let mut first: Option<(u64, u32)> = None;
        loop {
            report.rounds += 1;
            if report.rounds > MAX_ROUNDS {
                return Err(SyncError::Protocol("sync does not converge".into()));
            }
            let local = measure!("client.local_versions", local_versions(v)?);
            let versions: PageBytes = match &pages {
                None => local.iter().map(|(k, x)| (k.clone(), x.clone())).collect(),
                Some(ids) => {
                    let gone: HashSet<String> = v.purged_pages().into_iter().collect();
                    ids.iter()
                        // V2 can advertise a purged page's conflict archive
                        // after its tombstone deleted our old active baseline.
                        // Keep that explicit comparison even with no local
                        // history: an empty version requests the full archive.
                        .filter(|id| st.purge_v2 || local.contains_key(*id) || !gone.contains(*id))
                        .map(|id| (id.clone(), local.get(id).cloned().unwrap_or_default()))
                        .collect()
                }
            };
            let sent: Vec<PageId> = versions.iter().map(|(k, _)| k.clone()).collect();
            let req = SyncRequest {
                full,
                base: base.map(|b| (b.hub_epoch, b.hub_seq)),
                versions,
                purged: std::mem::take(&mut purged),
                hub_purged_known: hub_purged,
                blobs: std::mem::take(&mut blobs),
                budget: self.budget,
            };
            let before_restores = v.restore_records();
            let body = measure!("client.request.encode", if st.purge_v2 {
                let mut request = req.clone();
                request.purged.clear();
                let purged_now: HashSet<_> = v.purged_pages().into_iter().collect();
                SyncRequestV2 {
                    request,
                    purges: v.purge_records(),
                    restores: before_restores.iter()
                        .filter(|record| !purged_now.contains(&record.page))
                        .cloned().collect(),
                }
                .encode_with_device(st.purge_device)
            } else {
                req.encode()
            });
            let zstd = Zstd {
                advertise: true,
                compress: st.zstd,
            };
            let reply = self.http.call(
                "POST",
                "/sync",
                Some(&body),
                None,
                zstd,
                st.purge_v2 && st.purge_device,
            )?;
            report.bytes_up += reply.up;
            report.bytes_down += reply.down;
            st.zstd = reply.confirmed;
            let (mut resp, purge_records, restore_records) = measure!("client.response.decode", if st.purge_v2 {
                let reply = SyncResponseV2::decode_with_device(&reply.body, st.purge_device)?;
                (reply.response, reply.purges, reply.restores)
            } else {
                (SyncResponse::decode(&reply.body)?, Vec::new(), Vec::new())
            });
            if resp.reset {
                return Ok(false);
            }
            first.get_or_insert((resp.epoch, resp.seq));
            hub_purged = resp.purged_total;
            let mut retained = HashSet::new();
            if st.purge_v2 {
                report.purged += v.import_purge_records(purge_records)?;
                retained.extend(
                    v.restore_records()
                        .into_iter()
                        .filter(|record| !before_restores.contains(record))
                        .map(|record| record.page),
                );
                let mut proofs: HashMap<_, _> = restore_records
                    .into_iter()
                    .map(|record| (record.page.clone(), record))
                    .collect();
                let mut ordinary = Vec::new();
                for (page, bytes) in std::mem::take(&mut resp.updates) {
                    if let Some(record) = proofs.remove(&page) {
                        let r = v.import_restored_updates(record, bytes)?;
                        report.created += r.created;
                        report.changed += r.changed;
                        report.conflicts += r.conflicts;
                    } else if !retained.contains(&page) {
                        ordinary.push((page, bytes));
                    }
                }
                if !proofs.is_empty() {
                    return Err(SyncError::Protocol(
                        "restore proof has no full snapshot".into(),
                    ));
                }
                resp.updates = ordinary;
            } else {
                report.purged += v.import_purged(resp.purged)?;
            }
            if !resp.updates.is_empty() {
                let updates = resp
                    .updates
                    .into_iter()
                    .map(|(page, bytes)| PageUpdate { page, bytes })
                    .collect();
                let r = measure!(
                    "client.import_batch",
                    if st.purge_v2 {
                        v.import_updates_batch(updates)?
                    } else {
                        v.import_updates_batch_legacy(updates)?
                    }
                );
                report.created += r.created;
                report.changed += r.changed;
                report.conflicts += r.conflicts;
            }
            self.push(&resp.need, report, st.zstd, st.purge_v2)?;
            for b in &resp.need_blobs {
                self.put_blob(b, report)?;
            }
            st.hub_blobs.extend(req.blobs);
            // The hub now has at least what was sent (it either had it or
            // got it from the push) and what it sent.
            for (id, lv) in req.versions {
                if !lv.is_empty() {
                    st.known.insert(id, lv);
                }
            }
            for (id, hv) in resp.versions {
                st.known.insert(id, hv);
            }
            // Next round: pages changed on the hub that this lacks, and,
            // if the budget cut the updates short, this round's pages.
            let mut next: HashSet<PageId> = HashSet::new();
            next.extend(retained);
            if st.purge_v2 {
                // Imports can create a recovery operation locally. Compare
                // it again rather than claiming the hub already has it.
                let local = local_versions(v)?;
                for record in v.restore_records() {
                    let covered = match (local.get(&record.page), st.known.get(&record.page)) {
                        (Some(lv), Some(hv)) => version_includes(hv, lv)?,
                        (Some(_), None) => false,
                        _ => true,
                    };
                    if !covered {
                        next.insert(record.page);
                    }
                }
            }
            if !resp.changed.is_empty() {
                let local = local_versions(v)?;
                for (id, hv) in &resp.changed {
                    let lv = local.get(id).map(Vec::as_slice).unwrap_or(&[]);
                    if !version_includes(lv, hv)? {
                        next.insert(id.clone());
                    }
                }
            }
            if resp.more {
                if full {
                    continue;
                }
                next.extend(sent);
            }
            if next.is_empty() {
                break;
            }
            pages = Some(next);
        }
        let (hub_epoch, hub_seq) = first.expect("one round");
        st.base = Some(Base {
            hub_epoch,
            hub_seq,
            hub_purged,
            local_purged: local_purged.len(),
        });
        Ok(true)
    }

    /// Sends the changes the hub asked for, in budget-sized requests.
    /// `compress`: the hub confirmed the zstd capability in this sync.
    fn push(
        &self,
        need: &PageBytes,
        report: &mut SyncReport,
        compress: bool,
        purge_v2: bool,
    ) -> Result<()> {
        let mut batch: PageBytes = Vec::new();
        let mut size = 0u64;
        let purged: HashSet<_> = self.vault.purged_pages().into_iter().collect();
        let restored: HashSet<_> = if purge_v2 {
            self.vault
                .restore_records()
                .into_iter()
                .filter(|r| !purged.contains(&r.page))
                .map(|r| r.page)
                .collect()
        } else {
            HashSet::new()
        };
        for (id, hv) in need {
            let since = if restored.contains(id) {
                Vec::new()
            } else {
                hv.clone()
            };
            let bytes = match self.vault.export_updates(id.clone(), since) {
                Ok(b) => b,
                Err(VaultError::NotFound(_)) => continue,
                Err(e) => return Err(e.into()),
            };
            size += bytes.len() as u64;
            batch.push((id.clone(), bytes));
            if size >= self.budget {
                self.send_push(std::mem::take(&mut batch), report, compress, purge_v2)?;
                size = 0;
            }
        }
        if !batch.is_empty() {
            self.send_push(batch, report, compress, purge_v2)?;
        }
        Ok(())
    }

    fn send_push(
        &self,
        updates: PageBytes,
        report: &mut SyncReport,
        compress: bool,
        purge_v2: bool,
    ) -> Result<()> {
        let n = updates.len() as u32;
        let body = if purge_v2 {
            let purged: HashSet<_> = self.vault.purged_pages().into_iter().collect();
            let restores = self
                .vault
                .restore_records()
                .into_iter()
                .filter(|record| !purged.contains(&record.page)
                    && updates.iter().any(|(page, _)| page == &record.page))
                .collect();
            PushRequestV2 {
                request: PushRequest { updates },
                restores,
            }
            .encode()
        } else {
            PushRequest { updates }.encode()
        };
        let zstd = Zstd {
            advertise: true,
            compress,
        };
        // `/push` carries no purge records, so it has no device layout.
        let r = self.http.call("POST", "/push", Some(&body), None, zstd, false)?;
        report.bytes_up += r.up;
        report.bytes_down += r.down;
        PushResponse::decode(&r.body)?;
        report.pushed += n;
        Ok(())
    }

    fn put_blob(&self, id: &str, report: &mut SyncReport) -> Result<()> {
        let path = self.vault.attachment_path(id.to_string())?;
        let name = self.vault.attachment_info(id.to_string())?.name;
        let bytes = std::fs::read(path)?;
        let r = self.http.call(
            "PUT",
            &format!("/blobs/{id}"),
            Some(&bytes),
            Some(&name),
            Zstd::default(),
            false,
        )?;
        report.bytes_up += r.up;
        report.blobs_sent += 1;
        Ok(())
    }

    /// Downloads an attachment from the hub (attachments are fetched
    /// lazily: a page can reference one that is not here yet).
    pub fn fetch_attachment(&self, id: &str) -> Result<()> {
        if self.vault.attachment_path(id.to_string()).is_ok() {
            return Ok(());
        }
        let r = self
            .http
            .call("GET", &format!("/blobs/{id}"), None, None, Zstd::default(), false)?;
        self.vault
            .store_attachment(id.to_string(), r.name.unwrap_or_default(), r.body)?;
        Ok(())
    }

    /// Downloads every attachment that pages here use but that is missing.
    /// Returns how many were fetched.
    pub fn fetch_missing_attachments(&self) -> Result<u32> {
        let mut n = 0;
        for id in self.vault.missing_attachments()? {
            self.fetch_attachment(&id)?;
            n += 1;
        }
        Ok(n)
    }

    /// Syncs in the background until the returned handle is stopped or
    /// dropped: once at start, after local changes (debounced), when the
    /// hub reports changes, every `interval_ms`, and after failures with
    /// backoff.
    pub fn start(self: &Arc<Self>, opts: SyncOptions) -> BackgroundSync {
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(SyncStatus::default()));
        let (tx, rx) = mpsc::channel::<Trigger>();
        let mut threads = Vec::new();

        // Local changes.
        {
            let (vault, stop, tx) = (self.vault.clone(), stop.clone(), tx.clone());
            // Subscribe before either thread can sync or start() can return.
            // A delayed watcher must still observe edits after that sync.
            let mut seq = vault.poll_sync_changes(u32::MAX).seq;
            threads.push(
                std::thread::Builder::new()
                    .name("sync-local".into())
                    .spawn(move || {
                        while !stop.load(Ordering::SeqCst) {
                            let b = vault.wait_sync_changes(seq, 250);
                            if b.seq != seq {
                                seq = b.seq;
                                if tx.send(Trigger::Local(seq)).is_err() {
                                    break;
                                }
                            }
                        }
                    })
                    .expect("spawn"),
            );
        }
        // Hub notifications. Not joined on stop: it ends after its current
        // long-poll and holds no vault.
        {
            let (mut http, stop, tx) = (self.http.clone(), stop.clone(), tx.clone());
            let wait = opts.long_poll_ms;
            http.agent = agent(Duration::from_millis(wait + 15_000), &http.trust);
            let _ = std::thread::Builder::new()
                .name("sync-remote".into())
                .spawn(move || {
                    let mut known: Option<(u64, u32)> = None;
                    let mut backoff = Duration::from_secs(1);
                    while !stop.load(Ordering::SeqCst) {
                        match http.changes(known, wait) {
                            Ok(c) => {
                                backoff = Duration::from_secs(1);
                                let now = (c.epoch, c.seq);
                                // The first answer triggers too: the hub may
                                // have changed after the loop's first sync.
                                if known != Some(now) && tx.send(Trigger::Remote).is_err() {
                                    break;
                                }
                                known = Some(now);
                            }
                            Err(_) => {
                                let until = Instant::now() + backoff;
                                while Instant::now() < until && !stop.load(Ordering::SeqCst) {
                                    std::thread::sleep(Duration::from_millis(100));
                                }
                                backoff = (backoff * 2).min(Duration::from_secs(60));
                            }
                        }
                    }
                });
        }
        // The sync loop.
        {
            let (client, stop, status) = (self.clone(), stop.clone(), status.clone());
            threads.push(
                std::thread::Builder::new()
                    .name("sync-loop".into())
                    .spawn(move || client.run_loop(rx, &stop, &status, &opts))
                    .expect("spawn"),
            );
        }
        BackgroundSync {
            stop,
            tx,
            status,
            threads,
        }
    }

    fn run_loop(
        &self,
        rx: mpsc::Receiver<Trigger>,
        stop: &AtomicBool,
        status: &Mutex<SyncStatus>,
        opts: &SyncOptions,
    ) {
        let interval = Duration::from_millis(opts.interval_ms.max(1));
        let debounce = Duration::from_millis(opts.debounce_ms);
        let max_delay = Duration::from_millis(opts.max_delay_ms);
        let mut backoff = Duration::from_secs(1);
        // Local changes up to this seq are covered by the last sync.
        let mut covered: u32 = 0;
        // First and latest local change not synced yet.
        let mut pending: Option<(Instant, Instant)> = None;
        let mut force = true;
        let mut retry_at: Option<Instant> = None;
        let mut last_sync = Instant::now();
        loop {
            let now = Instant::now();
            let mut deadline = last_sync + interval;
            if let Some((first, latest)) = pending {
                deadline = deadline.min((latest + debounce).min(first + max_delay));
            }
            if let Some(r) = retry_at {
                deadline = deadline.min(r);
            }
            if force {
                deadline = now;
            }
            if deadline > now {
                match rx.recv_timeout(deadline - now) {
                    Ok(Trigger::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                    Ok(Trigger::Local(seq)) => {
                        if seq > covered {
                            let t = Instant::now();
                            pending = Some(pending.map_or((t, t), |(f, _)| (f, t)));
                        }
                        continue;
                    }
                    // A hub change or a request: sync now.
                    Ok(Trigger::Remote) | Ok(Trigger::Now) => {}
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
            if stop.load(Ordering::SeqCst) {
                return;
            }
            let seq = self.vault.poll_sync_changes(u32::MAX).seq;
            lock(status).syncing = true;
            let result = self.sync_once().and_then(|r| {
                if opts.prefetch_attachments {
                    self.fetch_missing_attachments()?;
                }
                Ok(r)
            });
            force = false;
            pending = None;
            last_sync = Instant::now();
            let mut s = lock(status);
            s.syncing = false;
            s.syncs += 1;
            match result {
                Ok(r) => {
                    covered = seq;
                    s.last_ok_ms = Some(now_ms());
                    s.last_error = None;
                    s.last_report = Some(r);
                    backoff = Duration::from_secs(1);
                    retry_at = None;
                }
                Err(e) => {
                    s.last_error = Some(e.to_string());
                    retry_at = Some(Instant::now() + backoff);
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }
        }
    }
}

enum Trigger {
    /// A local change (the vault's feed seq after it).
    Local(u32),
    Remote,
    Now,
    Stop,
}

/// A running background sync; stops when dropped.
pub struct BackgroundSync {
    stop: Arc<AtomicBool>,
    tx: Sender<Trigger>,
    status: Arc<Mutex<SyncStatus>>,
    threads: Vec<JoinHandle<()>>,
}

impl BackgroundSync {
    pub fn status(&self) -> SyncStatus {
        lock(&self.status).clone()
    }

    /// Syncs as soon as possible (for example when the app comes to the
    /// foreground).
    pub fn sync_now(&self) {
        let _ = self.tx.send(Trigger::Now);
    }

    /// Stops after the sync in progress, if any.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Trigger::Stop);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for BackgroundSync {
    fn drop(&mut self) {
        self.shutdown();
    }
}
