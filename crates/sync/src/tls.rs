//! TLS for every hub client (sync, the agent and host relay websockets): a
//! certificate verifier that also accepts certificates the user trusted for
//! the hub, and the details of a certificate a hub presented.
//!
//! The verifier first runs rustls' webpki verifier over the bundled webpki
//! roots (the roots ureq uses by default), so a publicly trusted hub behaves
//! as before. Only when that fails with an unknown issuer (a self-signed
//! certificate, a private CA or a TLS-inspecting proxy) does [`Trust`] get a
//! say, and the chain is accepted when either
//! - an issuer stored for this hub's origin validates it: full webpki
//!   verification with that issuer as the only root, the name and validity
//!   included, so a proxy re-minting the leaf keeps working; the issuer is
//!   never used for another origin or host; or
//! - the end-entity certificate's SHA-256 is pinned (entries saved before
//!   issuers were kept, and chains without the issuer).
//!
//! Otherwise the presented chain is recorded in [`Trust`] and the error
//! stands. Every other verification error (expired, wrong name under a
//! public issuer, ...) is returned unchanged. Handshake signatures are
//! always checked by the webpki verifier.
//!
//! ureq 3.4.2 (the newest 3.x on crates.io as of 2026-10-19) takes no custom
//! rustls verifier or `ClientConfig` through its `TlsConfig`, so the agent is
//! built with `Agent::with_parts` and the connector chain below. That uses
//! ureq's `unversioned` transport API, which has no semver guarantee: ureq is
//! pinned to `=3.4.2` in Cargo.toml, and moving it means re-checking this
//! file against ureq's `src/tls/rustls.rs`.
//!
//! The chain mirrors ureq's `DefaultConnector` minus SOCKS (sync never uses a
//! SOCKS proxy): `ConnectProxyConnector` (HTTP CONNECT proxies keep working)
//! -> `TcpConnector` -> [`PinningConnector`].
//!
//! [`PinningConnector`] and [`TlsTransport`] are adapted from ureq 3.4.2's
//! `RustlsConnector` / `RustlsTransport` (src/tls/rustls.rs,
//! https://github.com/algesten/ureq), Copyright (c) 2019 Martin Algesten,
//! licensed under MIT or Apache-2.0.

use std::fmt;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::{
    CertificateError, ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore,
    SignatureScheme, StreamOwned, ALL_VERSIONS,
};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{
    Buffers, ConnectProxyConnector, ConnectionDetails, Connector, Either, LazyBuffers, NextTimeout,
    TcpConnector, Transport, TransportAdapter,
};

// ---- Fingerprints ----

/// SHA-256 of `der`.
pub(crate) fn sha256(der: &[u8]) -> [u8; 32] {
    Sha256::digest(der).into()
}

/// A fingerprint as uppercase hex pairs separated by `:`.
pub(crate) fn format_fingerprint(fp: &[u8]) -> String {
    fp.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// A SHA-256 fingerprint in any case, with or without `:` (spaces are
/// ignored too). `None` if it is not 32 bytes of hex.
pub(crate) fn parse_fingerprint(s: &str) -> Option<[u8; 32]> {
    let hex: Vec<u8> = s
        .bytes()
        .filter(|b| *b != b':' && !b.is_ascii_whitespace())
        .collect();
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in hex.chunks(2).enumerate() {
        let s = std::str::from_utf8(pair).ok()?;
        out[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

// ---- Trust: pins, issuer anchors and the last untrusted chain ----

/// A certificate the user trusted for one hub origin, as the app keeps it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedCertificate {
    /// `scheme://host:port` the certificate was trusted for.
    pub origin: String,
    /// SHA-256 of the end-entity certificate (hex in any case, with or
    /// without `:`), accepted for an unknown issuer.
    pub sha256: String,
    /// DER of the certificate that issued the end entity (the first
    /// intermediate the hub sent), when it sent one: a trust anchor for
    /// `origin` only.
    pub issuer_der: Option<Vec<u8>>,
}

/// `scheme://host:port` of `url` and its host, lowercased, with the default
/// port filled in (80 for http, 443 for https); `ws`/`wss` count as
/// `http`/`https`. A URL without a scheme is taken as http, as hub
/// addresses are. The same rules as the app's `originOf`.
pub fn origin_of(url: &str) -> Option<(String, String)> {
    let u = url.trim();
    if u.is_empty() {
        return None;
    }
    let (scheme, rest) = match u.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => ("http".to_string(), u),
    };
    let scheme = match scheme.as_str() {
        "wss" => "https".to_string(),
        "ws" => "http".to_string(),
        _ => scheme,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let (host, port) = if let Some(r) = authority.strip_prefix('[') {
        let (h, after) = r.split_once(']')?;
        (h, after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return None;
    }
    let port: u16 = match port {
        Some(p) if !p.is_empty() => p.parse().ok()?,
        _ => match scheme.as_str() {
            "https" => 443,
            "http" => 80,
            _ => 0,
        },
    };
    let host = host.to_ascii_lowercase();
    let shown = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.clone()
    };
    Some((format!("{scheme}://{shown}:{port}"), host))
}

/// A stored issuer, trusted as a root for one host only.
struct Anchor {
    host: ServerName<'static>,
    /// webpki over this one root: path, validity and the name are checked.
    verifier: Arc<WebPkiServerVerifier>,
}

/// What one hub client trusts beyond the public roots, and the chain its
/// last rejected handshake presented. Shared by the client's connections
/// and their verifiers. Built per hub URL with [`Trust::for_hub`].
pub struct Trust {
    pins: Vec<[u8; 32]>,
    anchors: Vec<Anchor>,
    /// End entity first, then intermediates, as DER.
    untrusted: Mutex<Option<Vec<Vec<u8>>>>,
    /// Bumped each time a chain is recorded.
    recorded: AtomicU64,
}

fn parse_pin(t: &str) -> Option<[u8; 32]> {
    let p = parse_fingerprint(t);
    if p.is_none() {
        tracing::warn!("ignoring a trusted certificate fingerprint that is not SHA-256 hex");
    }
    p
}

impl Trust {
    /// Leaf pins from `trusted` only; entries that are not SHA-256
    /// fingerprints are skipped with a warning.
    pub fn new(trusted: &[String]) -> Arc<Trust> {
        Trust::build(trusted.iter().filter_map(|t| parse_pin(t)).collect(), Vec::new())
    }

    /// The trust of a client of the hub at `url`: each entry's leaf pin,
    /// and the issuer of each entry trusted for `url`'s origin as an extra
    /// root for that origin's host. Issuers stored for other origins are
    /// ignored, and so is an issuer that is not a usable certificate.
    pub fn for_hub(url: &str, trusted: &[TrustedCertificate]) -> Arc<Trust> {
        let pins = trusted
            .iter()
            .filter_map(|t| parse_pin(&t.sha256))
            .collect();
        let origin = origin_of(url);
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut anchors = Vec::new();
        for t in trusted {
            let Some(der) = t.issuer_der.as_ref().filter(|d| !d.is_empty()) else {
                continue;
            };
            let Some((origin, host)) = origin.as_ref() else {
                continue;
            };
            if origin_of(&t.origin).map(|(o, _)| o).as_ref() != Some(origin) {
                continue;
            }
            let Ok(name) = ServerName::try_from(host.clone()) else {
                continue;
            };
            let mut roots = RootCertStore::empty();
            if roots.add(CertificateDer::from(der.clone())).is_err() {
                tracing::warn!("ignoring a trusted issuer that is not a usable certificate");
                continue;
            }
            match WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                .build()
            {
                Ok(verifier) => anchors.push(Anchor {
                    host: name,
                    verifier,
                }),
                Err(e) => tracing::warn!("ignoring a trusted issuer: {e}"),
            }
        }
        Trust::build(pins, anchors)
    }

    fn build(pins: Vec<[u8; 32]>, anchors: Vec<Anchor>) -> Arc<Trust> {
        Arc::new(Trust {
            pins,
            anchors,
            untrusted: Mutex::new(None),
            recorded: AtomicU64::new(0),
        })
    }

    /// A marker to pass to [`Trust::untrusted_since`].
    pub fn generation(&self) -> u64 {
        self.recorded.load(Ordering::SeqCst)
    }

    /// The leaf fingerprint of a chain recorded after `generation`.
    pub fn untrusted_since(&self, generation: u64) -> Option<String> {
        if self.generation() == generation {
            return None;
        }
        let g = self.untrusted.lock().unwrap_or_else(|p| p.into_inner());
        g.as_ref()
            .and_then(|c| c.first())
            .map(|leaf| format_fingerprint(&sha256(leaf)))
    }

    /// Forgets the recorded chain (after a successful request).
    pub fn clear(&self) {
        let mut g = self.untrusted.lock().unwrap_or_else(|p| p.into_inner());
        *g = None;
    }

    /// The recorded chain, leaf first, as DER.
    pub fn chain(&self) -> Option<Vec<Vec<u8>>> {
        self.untrusted
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn record(&self, chain: Vec<Vec<u8>>) {
        *self.untrusted.lock().unwrap_or_else(|p| p.into_inner()) = Some(chain);
        self.recorded.fetch_add(1, Ordering::SeqCst);
    }

    /// Whether an issuer stored for `server_name`'s host validates the
    /// chain (full webpki: the path to that issuer, validity, the name).
    fn anchored(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> bool {
        self.anchors.iter().any(|a| {
            same_host(&a.host, server_name)
                && a.verifier
                    .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
                    .is_ok()
        })
    }
}

/// The same host: DNS names ignoring case and a trailing dot, IP
/// addresses by value.
fn same_host(a: &ServerName<'_>, b: &ServerName<'_>) -> bool {
    match (a, b) {
        (ServerName::DnsName(x), ServerName::DnsName(y)) => x
            .as_ref()
            .trim_end_matches('.')
            .eq_ignore_ascii_case(y.as_ref().trim_end_matches('.')),
        _ => a == b,
    }
}

/// The rustls config of every hub client (sync over ureq, the agent and
/// host relay websockets): the bundled webpki roots, then for an unknown
/// issuer `trust`'s issuer anchors and leaf pins (see the module docs).
pub fn client_config(trust: &Arc<Trust>) -> Arc<ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    // Fails only on an empty root store; the bundled roots are not.
    let webpki = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .expect("webpki verifier over the bundled roots");
    let verifier = PinningVerifier {
        webpki,
        trust: trust.clone(),
    };
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(ALL_VERSIONS)
        .expect("all TLS versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Arc::new(config)
}

/// A ureq agent with `config` whose TLS uses `trust`'s pinning verifier.
pub(crate) fn agent(config: ureq::config::Config, trust: &Arc<Trust>) -> ureq::Agent {
    let connector =
        ().chain(ConnectProxyConnector::default())
            .chain(TcpConnector::default())
            .chain(PinningConnector {
                trust: trust.clone(),
                config: OnceLock::new(),
            });
    ureq::Agent::with_parts(config, connector, DefaultResolver::default())
}

// ---- The verifier ----

struct PinningVerifier {
    webpki: Arc<WebPkiServerVerifier>,
    trust: Arc<Trust>,
}

impl fmt::Debug for PinningVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PinningVerifier")
            .field("pins", &self.trust.pins.len())
            .field("anchors", &self.trust.anchors.len())
            .finish()
    }
}

impl ServerCertVerifier for PinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.webpki.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Err(rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer)) => {
                if self.trust.pins.contains(&sha256(end_entity))
                    || self.trust.anchored(
                        end_entity,
                        intermediates,
                        server_name,
                        ocsp_response,
                        now,
                    )
                {
                    return Ok(ServerCertVerified::assertion());
                }
                let chain = std::iter::once(end_entity)
                    .chain(intermediates)
                    .map(|c| c.as_ref().to_vec())
                    .collect();
                self.trust.record(chain);
                Err(rustls::Error::InvalidCertificate(
                    CertificateError::UnknownIssuer,
                ))
            }
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.webpki.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.webpki.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.webpki.supported_verify_schemes()
    }
}

// ---- The connector (adapted from ureq's RustlsConnector) ----

/// Wraps the chained transport in rustls with the pinning verifier when the
/// request is `https` and the transport is not TLS already.
struct PinningConnector {
    trust: Arc<Trust>,
    /// Built on the first TLS connection.
    config: OnceLock<Arc<ClientConfig>>,
}

impl fmt::Debug for PinningConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PinningConnector").finish()
    }
}

impl<In: Transport> Connector<In> for PinningConnector {
    type Out = Either<In, TlsTransport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        let Some(transport) = chained else {
            return Ok(None);
        };
        if !details.needs_tls() || transport.is_tls() {
            return Ok(Some(Either::A(transport)));
        }
        let host = details
            .uri
            .authority()
            .ok_or(ureq::Error::Tls("uri authority for tls"))?
            .host();
        // An IPv6 literal comes in brackets.
        let host = host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(host);
        let name = ServerName::try_from(host)
            .map_err(|_| ureq::Error::Tls("Rustls invalid dns name error"))?
            .to_owned();
        let mut conn = ClientConnection::new(
            self.config
                .get_or_init(|| client_config(&self.trust))
                .clone(),
            name,
        )?;
        let mut sock = TransportAdapter::new(transport.boxed());
        sock.set_timeout(details.timeout);
        conn.complete_io(&mut sock)?;
        let buffers = LazyBuffers::new(
            details.config.input_buffer_size(),
            details.config.output_buffer_size(),
        );
        Ok(Some(Either::B(TlsTransport {
            buffers,
            stream: StreamOwned { conn, sock },
        })))
    }
}

struct TlsTransport {
    buffers: LazyBuffers,
    stream: StreamOwned<ClientConnection, TransportAdapter>,
}

impl fmt::Debug for TlsTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsTransport")
            .field("chained", &self.stream.sock.inner())
            .finish()
    }
}

impl Transport for TlsTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.stream.get_mut().set_timeout(timeout);
        let output = &self.buffers.output()[..amount];
        self.stream.write_all(output)?;
        Ok(())
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.stream.get_mut().set_timeout(timeout);
        let input = self.buffers.input_append_buf();
        let amount = self.stream.read(input)?;
        self.buffers.input_appended(amount);
        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        self.stream.get_mut().get_mut().is_open()
    }

    fn is_tls(&self) -> bool {
        true
    }
}

// ---- Certificate details ----

/// What a certificate says, for showing it before trusting it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateInfo {
    /// RFC 4514 style (most significant RDN last), for example
    /// `CN=hub.example,O=Example`. For a certificate that could not be
    /// parsed: `unparsable certificate: <reason>`, with only `sha256` set.
    pub subject: String,
    pub issuer: String,
    /// Validity start and end (ms since the epoch).
    pub not_before_ms: i64,
    pub not_after_ms: i64,
    /// SHA-256 of the DER, uppercase hex pairs separated by `:`.
    pub sha256: String,
    /// Serial number, uppercase hex.
    pub serial: String,
    /// Subject and issuer are the same name.
    pub self_signed: bool,
}

impl CertificateInfo {
    /// The details of one DER certificate.
    pub fn from_der(der: &[u8]) -> Result<CertificateInfo, String> {
        let (_, cert) = x509_parser::parse_x509_certificate(der).map_err(|e| e.to_string())?;
        let v = cert.validity();
        Ok(CertificateInfo {
            subject: rfc4514(cert.subject()),
            issuer: rfc4514(cert.issuer()),
            not_before_ms: v.not_before.timestamp().saturating_mul(1000),
            not_after_ms: v.not_after.timestamp().saturating_mul(1000),
            sha256: format_fingerprint(&sha256(der)),
            serial: cert
                .tbs_certificate
                .raw_serial()
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect(),
            self_signed: cert.subject().as_raw() == cert.issuer().as_raw(),
        })
    }

    /// The details of a chain (leaf first). A certificate that cannot be
    /// parsed keeps its place with its fingerprint and the reason.
    pub fn chain(ders: &[Vec<u8>]) -> Vec<CertificateInfo> {
        ders.iter()
            .map(|d| {
                CertificateInfo::from_der(d).unwrap_or_else(|e| CertificateInfo {
                    subject: format!("unparsable certificate: {e}"),
                    issuer: String::new(),
                    not_before_ms: 0,
                    not_after_ms: 0,
                    sha256: format_fingerprint(&sha256(d)),
                    serial: String::new(),
                    self_signed: false,
                })
            })
            .collect()
    }
}

/// A name in RFC 4514 form: RDNs in reverse order, joined by `,`;
/// multi-valued RDNs joined by `+`.
fn rfc4514(name: &x509_parser::x509::X509Name<'_>) -> String {
    let registry = x509_parser::objects::oid_registry();
    name.iter_rdn()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|rdn| {
            rdn.iter()
                .map(|atv| {
                    let oid = atv.attr_type();
                    let key = x509_parser::objects::oid2abbrev(oid, registry)
                        .map(str::to_string)
                        .unwrap_or_else(|_| oid.to_id_string());
                    let value = match atv.as_str() {
                        Ok(s) => escape_value(s),
                        Err(_) => format!(
                            "#{}",
                            atv.attr_value()
                                .as_bytes()
                                .iter()
                                .map(|b| format!("{b:02X}"))
                                .collect::<String>()
                        ),
                    };
                    format!("{key}={value}")
                })
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// RFC 4514 section 2.4 escaping.
fn escape_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let last = s.chars().count().saturating_sub(1);
    for (i, c) in s.chars().enumerate() {
        let special = matches!(c, '"' | '+' | ',' | ';' | '<' | '>' | '\\')
            || (i == 0 && (c == '#' || c == ' '))
            || (i == last && c == ' ');
        if special {
            out.push('\\');
            out.push(c);
        } else if c == '\0' {
            out.push_str("\\00");
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, DnType, KeyPair, SerialNumber};

    fn params(cn: &str) -> CertificateParams {
        let mut p = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
        p.distinguished_name = rcgen::DistinguishedName::new();
        p.distinguished_name.push(DnType::CommonName, cn);
        p.not_before = rcgen::date_time_ymd(2025, 1, 1);
        p.not_after = rcgen::date_time_ymd(2030, 1, 1);
        p.serial_number = Some(SerialNumber::from(vec![0x01, 0x02, 0xAB]));
        p
    }

    fn assert_fingerprint_format(fp: &str) {
        assert_eq!(fp.len(), 32 * 3 - 1, "{fp}");
        for (i, pair) in fp.split(':').enumerate() {
            assert!(i < 32, "{fp}");
            assert_eq!(pair.len(), 2, "{fp}");
            assert!(
                pair.chars()
                    .all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c)),
                "{fp}"
            );
        }
    }

    #[test]
    fn self_signed_details() {
        let key = KeyPair::generate().unwrap();
        let mut p = params("hub.test");
        p.distinguished_name
            .push(DnType::OrganizationName, "Scratch, Inc");
        let cert = p.self_signed(&key).unwrap();
        let info = CertificateInfo::from_der(cert.der()).unwrap();
        assert_eq!(info.subject, "O=Scratch\\, Inc,CN=hub.test");
        assert_eq!(info.issuer, info.subject);
        assert!(info.self_signed);
        assert_eq!(info.not_before_ms, 1_735_689_600_000);
        assert_eq!(info.not_after_ms, 1_893_456_000_000);
        assert_eq!(info.serial, "0102AB");
        assert_fingerprint_format(&info.sha256);
        assert_eq!(info.sha256, format_fingerprint(&sha256(cert.der())));
    }

    #[test]
    fn ca_signed_details() {
        let ca_key = KeyPair::generate().unwrap();
        let mut ca = params("Test CA");
        ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_cert = ca.self_signed(&ca_key).unwrap();
        let key = KeyPair::generate().unwrap();
        let leaf = params("hub.test")
            .signed_by(&key, &ca_cert, &ca_key)
            .unwrap();
        let info = CertificateInfo::from_der(leaf.der()).unwrap();
        assert_eq!(info.subject, "CN=hub.test");
        assert_eq!(info.issuer, "CN=Test CA");
        assert!(!info.self_signed);
        let chain = CertificateInfo::chain(&[leaf.der().to_vec(), ca_cert.der().to_vec()]);
        assert_eq!(chain.len(), 2);
        assert!(chain[1].self_signed);
    }

    #[test]
    fn unparsable_keeps_fingerprint() {
        assert!(CertificateInfo::from_der(b"not a certificate").is_err());
        let chain = CertificateInfo::chain(&[b"junk".to_vec()]);
        assert!(chain[0].subject.starts_with("unparsable certificate: "));
        assert_eq!(chain[0].sha256, format_fingerprint(&sha256(b"junk")));
    }

    #[test]
    fn fingerprints_parse_in_any_form() {
        let fp = sha256(b"x");
        let colon = format_fingerprint(&fp);
        assert_fingerprint_format(&colon);
        assert_eq!(parse_fingerprint(&colon), Some(fp));
        assert_eq!(parse_fingerprint(&colon.to_lowercase()), Some(fp));
        assert_eq!(parse_fingerprint(&colon.replace(':', "")), Some(fp));
        assert_eq!(parse_fingerprint("AB:CD"), None);
        assert_eq!(parse_fingerprint(&"zz".repeat(32)), None);
    }

    #[test]
    fn escaping() {
        assert_eq!(escape_value(" a+b "), "\\ a\\+b\\ ");
        assert_eq!(escape_value("#x"), "\\#x");
        assert_eq!(escape_value("plain"), "plain");
    }

    // ---- Trust: issuer anchors and leaf pins, over in-memory handshakes ----

    use rustls::{ServerConfig, ServerConnection};
    use rustls_pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

    /// A CA (self-signed root, or an intermediate under one).
    struct Ca {
        cert: rcgen::Certificate,
        key: KeyPair,
    }

    fn ca_params(cn: &str) -> CertificateParams {
        let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
        p.distinguished_name = rcgen::DistinguishedName::new();
        p.distinguished_name.push(DnType::CommonName, cn);
        p.distinguished_name.push(DnType::OrganizationName, "Example Corp");
        p.distinguished_name
            .push(DnType::OrganizationalUnitName, "Network Security");
        p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        p
    }

    fn root_ca(cn: &str) -> Ca {
        let key = KeyPair::generate().unwrap();
        let cert = ca_params(cn).self_signed(&key).unwrap();
        Ca { cert, key }
    }

    fn intermediate_ca(cn: &str, parent: &Ca) -> Ca {
        let key = KeyPair::generate().unwrap();
        let cert = ca_params(cn)
            .signed_by(&key, &parent.cert, &parent.key)
            .unwrap();
        Ca { cert, key }
    }

    /// A served chain (leaf first) and the leaf's key.
    struct Served {
        chain: Vec<Vec<u8>>,
        key: Vec<u8>,
    }

    impl Served {
        fn leaf_sha256(&self) -> String {
            format_fingerprint(&sha256(&self.chain[0]))
        }
    }

    fn leaf_params_for(host: &str) -> CertificateParams {
        let mut p = CertificateParams::new(vec![host.to_string()]).unwrap();
        p.distinguished_name = rcgen::DistinguishedName::new();
        p.distinguished_name.push(DnType::CommonName, host);
        p
    }

    /// A leaf for `host` issued by `ca`, served with `ca` behind it.
    fn leaf(host: &str, ca: &Ca) -> Served {
        leaf_with(leaf_params_for(host), ca)
    }

    fn leaf_with(params: CertificateParams, ca: &Ca) -> Served {
        let key = KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &ca.cert, &ca.key).unwrap();
        Served {
            chain: vec![cert.der().to_vec(), ca.cert.der().to_vec()],
            key: key.serialize_der(),
        }
    }

    fn self_signed_leaf(host: &str) -> Served {
        let key = KeyPair::generate().unwrap();
        let cert = leaf_params_for(host).self_signed(&key).unwrap();
        Served {
            chain: vec![cert.der().to_vec()],
            key: key.serialize_der(),
        }
    }

    /// What the app stores when the user trusts `served` for `origin`.
    fn trusted(origin: &str, served: &Served) -> TrustedCertificate {
        TrustedCertificate {
            origin: origin.to_string(),
            sha256: served.leaf_sha256(),
            issuer_der: served.chain.get(1).cloned(),
        }
    }

    /// A full TLS handshake in memory: `client` connecting to `host`, the
    /// server presenting `served`. The client's error if it rejects it.
    fn handshake(client: Arc<ClientConfig>, host: &str, served: &Served) -> Result<(), rustls::Error> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let chain = served
            .chain
            .iter()
            .map(|c| CertificateDer::from(c.clone()))
            .collect();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(served.key.clone()));
        let server = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .unwrap();
        let name = ServerName::try_from(host.to_string()).unwrap();
        let mut c = ClientConnection::new(client, name)?;
        let mut s = ServerConnection::new(Arc::new(server)).unwrap();
        for _ in 0..16 {
            if !c.is_handshaking() && !s.is_handshaking() {
                return Ok(());
            }
            let mut buf = Vec::new();
            while c.wants_write() {
                c.write_tls(&mut buf).unwrap();
            }
            let mut r = &buf[..];
            while !r.is_empty() {
                s.read_tls(&mut r).unwrap();
            }
            s.process_new_packets()?;
            let mut buf = Vec::new();
            while s.wants_write() {
                s.write_tls(&mut buf).unwrap();
            }
            let mut r = &buf[..];
            while !r.is_empty() {
                c.read_tls(&mut r).unwrap();
            }
            c.process_new_packets()?;
        }
        panic!("the handshake did not finish");
    }

    const UNKNOWN_ISSUER: rustls::Error =
        rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer);

    #[test]
    fn hub_trust_issuer_accepts_a_reminted_leaf() {
        let ca = root_ca("Example Proxy CA");
        let first = leaf("notes.example.test", &ca);
        let second = leaf("notes.example.test", &ca);
        assert_ne!(first.leaf_sha256(), second.leaf_sha256());
        let url = "https://notes.example.test";
        let trust = Trust::for_hub(url, &[trusted("https://notes.example.test:443", &first)]);
        let config = client_config(&trust);
        handshake(config.clone(), "notes.example.test", &first).unwrap();
        handshake(config.clone(), "NOTES.example.test", &second).unwrap();
        assert!(trust.chain().is_none());
    }

    #[test]
    fn hub_trust_issuer_under_a_root_accepts_a_reminted_leaf() {
        // The proxy's CA is an intermediate: the server sends it after the
        // leaf, and it alone is kept (its root is never seen).
        let root = root_ca("Proxy Root");
        let ca = intermediate_ca("Example Proxy CA", &root);
        let first = leaf("notes.example.test", &ca);
        let second = leaf("notes.example.test", &ca);
        let trust = Trust::for_hub(
            "wss://notes.example.test/agent/ws",
            &[trusted("https://notes.example.test:443", &first)],
        );
        handshake(client_config(&trust), "notes.example.test", &second).unwrap();
    }

    #[test]
    fn hub_trust_issuer_is_not_used_for_another_host() {
        let ca = root_ca("Example Proxy CA");
        let hub = leaf("notes.example.test", &ca);
        let other = leaf("other.example.test", &ca);
        let entry = trusted("https://notes.example.test:443", &hub);

        // The hub's client, reaching another host through the same CA.
        let trust = Trust::for_hub("https://notes.example.test", &[entry.clone()]);
        let config = client_config(&trust);
        assert_eq!(handshake(config.clone(), "other.example.test", &other), Err(UNKNOWN_ISSUER));
        // The hub's name, with a leaf issued for another host.
        assert_eq!(handshake(config, "notes.example.test", &other), Err(UNKNOWN_ISSUER));
        assert_eq!(
            trust.chain().unwrap()[0],
            other.chain[0],
            "the rejected chain is recorded"
        );

        // Another origin's client given the entry: the issuer is ignored.
        let trust = Trust::for_hub("https://other.example.test", &[entry.clone()]);
        assert_eq!(
            handshake(client_config(&trust), "other.example.test", &other),
            Err(UNKNOWN_ISSUER)
        );
        // The same host on another port is another origin too.
        let trust = Trust::for_hub("https://notes.example.test:8443", &[entry]);
        let reminted = leaf("notes.example.test", &ca);
        assert_eq!(
            handshake(client_config(&trust), "notes.example.test", &reminted),
            Err(UNKNOWN_ISSUER)
        );
    }

    #[test]
    fn hub_trust_issuer_checks_validity() {
        let ca = root_ca("Example Proxy CA");
        let first = leaf("notes.example.test", &ca);
        let mut p = leaf_params_for("notes.example.test");
        p.not_before = rcgen::date_time_ymd(2020, 1, 1);
        p.not_after = rcgen::date_time_ymd(2021, 1, 1);
        let expired = leaf_with(p, &ca);
        let trust = Trust::for_hub(
            "https://notes.example.test",
            &[trusted("https://notes.example.test:443", &first)],
        );
        assert!(handshake(client_config(&trust), "notes.example.test", &expired).is_err());
    }

    #[test]
    fn hub_trust_leaf_pin_without_issuer_still_works() {
        // An entry saved before issuers were kept: only the leaf pin.
        let ca = root_ca("Example Proxy CA");
        let served = leaf("notes.example.test", &ca);
        let entry = TrustedCertificate {
            origin: "https://notes.example.test:443".into(),
            sha256: served.leaf_sha256().to_lowercase().replace(':', ""),
            issuer_der: None,
        };
        let trust = Trust::for_hub("https://notes.example.test", &[entry]);
        let config = client_config(&trust);
        handshake(config.clone(), "notes.example.test", &served).unwrap();
        // A re-minted leaf is not covered by a leaf pin.
        let reminted = leaf("notes.example.test", &ca);
        assert_eq!(handshake(config, "notes.example.test", &reminted), Err(UNKNOWN_ISSUER));

        // A self-signed hub sends no issuer: the leaf pin is all there is.
        let own = self_signed_leaf("hub.example.test");
        let trust = Trust::for_hub("https://hub.example.test", &[trusted("https://hub.example.test:443", &own)]);
        handshake(client_config(&trust), "hub.example.test", &own).unwrap();
        // And the old fingerprint-only constructor.
        let trust = Trust::new(&[own.leaf_sha256()]);
        handshake(client_config(&trust), "hub.example.test", &own).unwrap();
    }

    #[test]
    fn hub_trust_untrusted_chain_is_recorded_with_its_issuer() {
        let ca = root_ca("Example Proxy CA");
        let served = leaf("notes.example.test", &ca);
        let trust = Trust::for_hub("https://notes.example.test", &[]);
        let generation = trust.generation();
        assert_eq!(
            handshake(client_config(&trust), "notes.example.test", &served),
            Err(UNKNOWN_ISSUER)
        );
        assert_eq!(trust.untrusted_since(generation), Some(served.leaf_sha256()));
        assert_eq!(trust.chain().unwrap(), served.chain);
        let info = CertificateInfo::chain(&served.chain);
        assert_eq!(info[0].issuer, "OU=Network Security,O=Example Corp,CN=Example Proxy CA");
    }

    #[test]
    fn hub_trust_ignores_an_unusable_issuer() {
        let ca = root_ca("Example Proxy CA");
        let served = leaf("notes.example.test", &ca);
        let mut entry = trusted("https://notes.example.test:443", &served);
        entry.issuer_der = Some(b"not a certificate".to_vec());
        entry.sha256 = format_fingerprint(&sha256(b"another leaf"));
        let trust = Trust::for_hub("https://notes.example.test", &[entry]);
        assert_eq!(
            handshake(client_config(&trust), "notes.example.test", &served),
            Err(UNKNOWN_ISSUER)
        );
    }

    #[test]
    fn hub_trust_origins_match_the_app() {
        let o = |u: &str| origin_of(u).map(|(o, _)| o);
        assert_eq!(o("https://Notes.Example.test"), Some("https://notes.example.test:443".into()));
        assert_eq!(o("https://notes.example.test/"), Some("https://notes.example.test:443".into()));
        assert_eq!(o("wss://notes.example.test/host/ws"), Some("https://notes.example.test:443".into()));
        assert_eq!(o("ws://10.0.0.2:8787/agent/ws"), Some("http://10.0.0.2:8787".into()));
        assert_eq!(o("192.0.2.10:8787"), Some("http://192.0.2.10:8787".into()));
        assert_eq!(o("https://[::1]:8443/x"), Some("https://[::1]:8443".into()));
        assert_eq!(o("https://[::1]"), Some("https://[::1]:443".into()));
        assert_eq!(origin_of("https://[::1]").unwrap().1, "::1");
        assert_eq!(o(""), None);
        assert_eq!(o("https://:443"), None);
    }
}
