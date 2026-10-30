//! `vault_sync`: syncs vaults through a hub over HTTP or HTTPS.
//!
//! - [`Hub`] serves one vault: [`Hub::handle`] answers a transport-free
//!   [`HubRequest`]; [`Hub::serve`] runs it on a plain-HTTP tiny_http
//!   server (`vault-hub <vault-dir> --listen <addr>`, or embedded in the
//!   desktop app, with Tailscale for transport security). `notes-hub`
//!   (`crates/hub`) serves it over axum behind a TLS proxy. Devices get a
//!   token from [`Hub::pair`] or, on `notes-hub`, from a login
//!   ([`Hub::login_device`]).
//! - [`SyncClient`] syncs a vault with a hub: [`SyncClient::sync_once`], or
//!   [`SyncClient::start`] for a background loop.
//! - [`wire`] is the protocol's message encoding.
//!
//! Design, protocol and limits: `wiki/sync/sync-state.md`.

// Instrumentation for the `storage-metrics` feature. Without it, each macro
// is its plain expression or nothing, so default builds are unchanged.
// Defined before the modules so they see them.

/// Times `$body` under a static label (exclusive and inclusive, per core).
#[cfg(feature = "storage-metrics")]
macro_rules! measure {
    ($label:literal, $body:expr) => {{
        let _span = vault_core::storage_metrics::span($label);
        $body
    }};
}
#[cfg(not(feature = "storage-metrics"))]
macro_rules! measure {
    ($label:literal, $body:expr) => {
        $body
    };
}

/// Times the rest of the enclosing block under a static label.
#[cfg(feature = "storage-metrics")]
macro_rules! span_guard {
    ($label:literal) => {
        let _span = vault_core::storage_metrics::span($label);
    };
}
#[cfg(not(feature = "storage-metrics"))]
macro_rules! span_guard {
    ($label:literal) => {};
}

/// Adds `$n` bytes to a static byte counter.
#[cfg(feature = "storage-metrics")]
macro_rules! count_bytes {
    ($label:literal, $n:expr) => {
        vault_core::storage_metrics::add_bytes($label, ($n) as u64)
    };
}
#[cfg(not(feature = "storage-metrics"))]
macro_rules! count_bytes {
    ($label:literal, $n:expr) => {
        // The byte-count expression is not evaluated in ordinary builds.
    };
}

mod auth;
mod client;
mod encoding;
mod hub;
pub mod open;
pub mod tls;
pub mod wire;

use std::fmt;

pub use auth::PairedDevice;
pub use client::{BackgroundSync, SyncClient, SyncOptions, SyncReport};
pub use hub::{Hub, HubRequest, HubResponse, HubServer};
pub use open::{bounce_page, bounce_page_with};
pub use tls::CertificateInfo;

/// Header carrying the client vault's device id on every request.
pub const DEVICE_HEADER: &str = "X-Device";
/// Header carrying an attachment's file name (percent-encoded).
pub const NAME_HEADER: &str = "X-Name";
/// Header advertising a transport capability on `/sync` and `/push`, and
/// the hub's confirmation of it on their answers.
pub const CAPABILITY_HEADER: &str = "X-Sync-Capability";
/// The zstd capability's exact token. Once the hub confirms it, `/sync` and
/// `/push` bodies may be zstd-compressed.
pub const ZSTD_CAPABILITY: &str = "zstd-v1";

/// Negotiates causal purge recovery before the first sync mutation.
pub const PURGE_CAPABILITY_HEADER: &str = "X-Purge-Protocol";
pub const PURGE_CAPABILITY: &str = "2";

/// Boolean capability for the purging device's id in protocol v2 purge
/// records. The hub advertises it on `/health` next to [`PURGE_CAPABILITY`].
/// A client sends `X-Purge-Device: 1` on a protocol v2 `/sync` only after
/// that health check; the flag gates the record layout, the wire version
/// does not change.
pub const PURGE_DEVICE_CAPABILITY_HEADER: &str = "X-Purge-Device";
pub const PURGE_DEVICE_CAPABILITY: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// Connecting to or talking with the hub failed.
    Http(String),
    /// The hub answered with an error status (401: bad token, 403: the
    /// token is another device's, 409: this vault is a copy of the hub's
    /// or of another paired device's).
    Status(u16, String),
    /// A message could not be read.
    Protocol(String),
    /// The vault failed.
    Vault(String),
    Io(String),
    /// The hub's TLS certificate has an unknown issuer (self-signed or a
    /// private CA) and is not pinned. `sha256` is the end-entity
    /// certificate's fingerprint, uppercase hex pairs separated by `:`;
    /// `SyncClient::untrusted_certificate` has the chain.
    UntrustedCertificate { sha256: String },
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::Http(m) => write!(f, "hub connection: {m}"),
            SyncError::Status(s, m) => write!(f, "hub answered {s}: {m}"),
            SyncError::Protocol(m) => write!(f, "sync protocol: {m}"),
            SyncError::Vault(m) => write!(f, "vault: {m}"),
            SyncError::Io(m) => write!(f, "io: {m}"),
            // Callers classify by this prefix; it must not start with
            // `hub connection` (shown as offline).
            SyncError::UntrustedCertificate { sha256 } => {
                write!(f, "untrusted certificate: {sha256}")
            }
        }
    }
}

impl std::error::Error for SyncError {}

impl From<vault_core::VaultError> for SyncError {
    fn from(e: vault_core::VaultError) -> Self {
        SyncError::Vault(e.to_string())
    }
}

impl From<std::io::Error> for SyncError {
    fn from(e: std::io::Error) -> Self {
        SyncError::Io(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, SyncError>;

/// Percent-encodes a header value (non-ASCII and control characters, `%`).
pub(crate) fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_graphic() && b != b'%' || b == b' ' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub(crate) fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 32 random bytes as hex.
pub(crate) fn random_hex() -> Result<String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| SyncError::Io(format!("random: {e}")))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Compares two byte strings in time that depends only on their lengths.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y));
    std::hint::black_box(diff) == 0
}

pub(crate) fn random_u64() -> Result<u64> {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).map_err(|e| SyncError::Io(format!("random: {e}")))?;
    Ok(u64::from_le_bytes(b))
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encoding_round_trips() {
        for s in ["plain.png", "Grüße 100%.pdf", "a\nb", "%", "x%4"] {
            let e = pct_encode(s);
            assert!(e.bytes().all(|b| b.is_ascii_graphic() || b == b' '));
            assert_eq!(pct_decode(&e), s);
        }
    }
}
