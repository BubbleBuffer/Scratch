//! vault_sync for Dart: the sync client with its background loop, and the
//! hub a desktop app embeds. Only type conversion happens here.

use std::sync::{Arc, Mutex};

use flutter_rust_bridge::frb;
use vault_sync::{BackgroundSync, Hub, HubServer, SyncClient};

use crate::api::types::*;
use crate::api::vault::NotesVault;
use crate::convert::*;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Default background-loop timings (vault_sync's defaults).
#[frb(sync)]
pub fn default_sync_options() -> SyncOptions {
    let d = vault_sync::SyncOptions::default();
    SyncOptions {
        debounce_ms: d.debounce_ms as u32,
        max_delay_ms: d.max_delay_ms as u32,
        interval_ms: d.interval_ms as u32,
        long_poll_ms: d.long_poll_ms as u32,
        prefetch_attachments: d.prefetch_attachments,
    }
}

/// Syncs one vault with a hub at `url` (for example `http://100.101.102.103:8787`)
/// using a pairing `token`.
#[frb(opaque)]
pub struct NotesSync {
    client: Arc<SyncClient>,
    background: Mutex<Option<BackgroundSync>>,
}

impl NotesSync {
    #[frb(sync)]
    pub fn new(vault: &NotesVault, url: String, token: String) -> NotesSync {
        NotesSync {
            client: Arc::new(SyncClient::new(vault.inner.clone(), &url, &token)),
            background: Mutex::new(None),
        }
    }

    /// Like `new`, also accepting a hub whose certificate has an unknown
    /// issuer (self-signed or a private CA) when its SHA-256 fingerprint is
    /// in `trusted` (hex in any case, with or without `:`). A sync against an
    /// unpinned one fails with an error starting `untrusted certificate: `;
    /// `untrusted_certificate` then has the chain.
    #[frb(sync)]
    pub fn with_trusted(
        vault: &NotesVault,
        url: String,
        token: String,
        trusted: Vec<String>,
    ) -> NotesSync {
        NotesSync {
            client: Arc::new(SyncClient::with_trusted(vault.inner.clone(), &url, &token, trusted)),
            background: Mutex::new(None),
        }
    }

    /// The chain (leaf first) of the last handshake that failed with an
    /// untrusted certificate; cleared by the next successful request.
    #[frb(sync)]
    pub fn untrusted_certificate(&self) -> Option<Vec<CertificateInfo>> {
        self.client.untrusted_certificate().map(|chain| {
            chain
                .into_iter()
                .map(|c| CertificateInfo {
                    subject: c.subject,
                    issuer: c.issuer,
                    not_before_ms: c.not_before_ms,
                    not_after_ms: c.not_after_ms,
                    sha256: c.sha256,
                    serial: c.serial,
                    self_signed: c.self_signed,
                })
                .collect()
        })
    }

    /// One complete sync (rounds until both sides are even).
    pub fn sync_once(&self) -> Result<SyncReport, String> {
        Ok(sync_report(self.client.sync_once().map_err(err)?))
    }

    pub fn fetch_attachment(&self, id: String) -> Result<(), String> {
        self.client.fetch_attachment(&id).map_err(err)
    }

    pub fn fetch_missing_attachments(&self) -> Result<u32, String> {
        self.client.fetch_missing_attachments().map_err(err)
    }

    /// Starts the background loop (restarts it if running).
    pub fn start(&self, options: SyncOptions) {
        let bg = self.client.start(sync_options_to(options));
        let old = lock(&self.background).replace(bg);
        if let Some(old) = old {
            old.stop();
        }
    }

    /// Stops the background loop after the sync in progress.
    pub fn stop(&self) {
        let old = lock(&self.background).take();
        if let Some(old) = old {
            old.stop();
        }
    }

    /// Asks the background loop to sync now (for example on resume).
    #[frb(sync)]
    pub fn sync_now(&self) {
        if let Some(bg) = lock(&self.background).as_ref() {
            bg.sync_now();
        }
    }

    #[frb(sync)]
    pub fn status(&self) -> SyncStatus {
        let g = lock(&self.background);
        match g.as_ref() {
            None => SyncStatus::default(),
            Some(bg) => {
                let s = bg.status();
                SyncStatus {
                    running: true,
                    syncing: s.syncing,
                    last_ok_ms: s.last_ok_ms,
                    last_error: s.last_error,
                    last_report: s.last_report.map(sync_report),
                    syncs: s.syncs as i64,
                }
            }
        }
    }
}

/// The hub embedded in the desktop app: serves this vault to paired devices.
#[frb(opaque)]
pub struct NotesHub {
    hub: Arc<Hub>,
    server: Mutex<Option<HubServer>>,
}

impl NotesHub {
    /// A hub for `vault` (pairing tokens live in `.vault/hub/tokens.json`).
    pub fn new(vault: &NotesVault) -> Result<NotesHub, String> {
        Ok(NotesHub { hub: Hub::new(vault.inner.clone()).map_err(err)?, server: Mutex::new(None) })
    }

    /// Serves on `addr` (for example `0.0.0.0:8787`, or `127.0.0.1:0` for a
    /// free port) and returns the bound address. Restarts if running.
    pub fn start(&self, addr: String) -> Result<String, String> {
        self.stop();
        let server = self.hub.serve(&addr).map_err(err)?;
        let bound = server.addr().to_string();
        // The page links the tools return open through this hub's /open route
        // on the loopback address (the hub's port is only known now).
        vault_tools::set_link_base(Some(&format!("http://127.0.0.1:{}", server.addr().port())));
        *lock(&self.server) = Some(server);
        Ok(bound)
    }

    pub fn stop(&self) {
        let old = lock(&self.server).take();
        if let Some(s) = old {
            s.stop();
        }
    }

    /// The bound address, if serving.
    #[frb(sync)]
    pub fn address(&self) -> Option<String> {
        lock(&self.server).as_ref().map(|s| s.addr().to_string())
    }

    /// A new pairing token for a device named `label` (shown once).
    pub fn pair(&self, label: String) -> Result<String, String> {
        self.hub.pair(&label).map_err(err)
    }

    /// Revokes the tokens labelled `label`; returns how many.
    pub fn unpair(&self, label: String) -> Result<u32, String> {
        Ok(self.hub.unpair(&label).map_err(err)? as u32)
    }

    pub fn devices(&self) -> Vec<PairedDevice> {
        self.hub
            .devices()
            .into_iter()
            .map(|d| PairedDevice { label: d.label, device: d.device, created_ms: d.created_ms })
            .collect()
    }
}
