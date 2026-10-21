//! TLS for the sync client: a certificate verifier that also accepts pinned
//! hub certificates, and the details of a certificate a hub presented.
//!
//! The verifier first runs rustls' webpki verifier over the bundled webpki
//! roots (the roots ureq uses by default), so a publicly trusted hub behaves
//! as before. Only when that fails with an unknown issuer (a self-signed
//! certificate or a private CA) is the end-entity certificate's SHA-256
//! compared with the pinned fingerprints: a match is accepted, otherwise the
//! presented chain is recorded in [`Trust`] and the error stands. Every other
//! verification error (expired, wrong name under a trusted issuer, ...) is
//! returned unchanged. Handshake signatures are always checked by the webpki
//! verifier.
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

// ---- Trust: pins and the last untrusted chain ----

/// The pinned fingerprints of one client and the chain its last rejected
/// handshake presented. Shared by the client's agents and their verifiers.
pub(crate) struct Trust {
    pins: Vec<[u8; 32]>,
    /// End entity first, then intermediates, as DER.
    untrusted: Mutex<Option<Vec<Vec<u8>>>>,
    /// Bumped each time a chain is recorded.
    recorded: AtomicU64,
}

impl Trust {
    /// Pins from `trusted`; entries that are not SHA-256 fingerprints are
    /// skipped with a warning.
    pub(crate) fn new(trusted: &[String]) -> Arc<Trust> {
        let pins = trusted
            .iter()
            .filter_map(|t| {
                let p = parse_fingerprint(t);
                if p.is_none() {
                    tracing::warn!(
                        "ignoring a trusted certificate fingerprint that is not SHA-256 hex"
                    );
                }
                p
            })
            .collect();
        Arc::new(Trust {
            pins,
            untrusted: Mutex::new(None),
            recorded: AtomicU64::new(0),
        })
    }

    /// A marker to pass to [`Trust::untrusted_since`].
    pub(crate) fn generation(&self) -> u64 {
        self.recorded.load(Ordering::SeqCst)
    }

    /// The leaf fingerprint of a chain recorded after `generation`.
    pub(crate) fn untrusted_since(&self, generation: u64) -> Option<String> {
        if self.generation() == generation {
            return None;
        }
        let g = self.untrusted.lock().unwrap_or_else(|p| p.into_inner());
        g.as_ref()
            .and_then(|c| c.first())
            .map(|leaf| format_fingerprint(&sha256(leaf)))
    }

    /// Forgets the recorded chain (after a successful request).
    pub(crate) fn clear(&self) {
        let mut g = self.untrusted.lock().unwrap_or_else(|p| p.into_inner());
        *g = None;
    }

    /// The recorded chain, leaf first.
    pub(crate) fn chain(&self) -> Option<Vec<Vec<u8>>> {
        self.untrusted
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn record(&self, chain: Vec<Vec<u8>>) {
        *self.untrusted.lock().unwrap_or_else(|p| p.into_inner()) = Some(chain);
        self.recorded.fetch_add(1, Ordering::SeqCst);
    }
}

/// The rustls config with the pinning verifier over `trust`.
fn client_config(trust: &Arc<Trust>) -> Arc<ClientConfig> {
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
                if self.trust.pins.contains(&sha256(end_entity)) {
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
}
