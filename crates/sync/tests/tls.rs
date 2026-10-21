//! The sync client against a hub behind a TLS front with a self-signed or
//! private-CA certificate: the untrusted-certificate error and its chain,
//! pinned fingerprints, and a changed certificate failing again.

mod support;

use std::io::{BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use support::*;
use vault_core::*;
use vault_sync::{Hub, HubServer, SyncClient, SyncError};

/// SHA-256 of `der` as uppercase hex pairs separated by `:`, computed here
/// rather than by the client.
fn fingerprint(der: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// A certificate chain (leaf first) and the leaf's key.
struct Identity {
    chain: Vec<Vec<u8>>,
    key: Vec<u8>,
}

impl Identity {
    fn leaf_fingerprint(&self) -> String {
        fingerprint(&self.chain[0])
    }

    fn server_config(&self) -> Arc<ServerConfig> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let chain = self
            .chain
            .iter()
            .map(|c| CertificateDer::from(c.clone()))
            .collect();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key.clone()));
        Arc::new(
            ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(chain, key)
                .unwrap(),
        )
    }
}

fn leaf_params(cn: &str) -> CertificateParams {
    let mut p =
        CertificateParams::new(vec!["127.0.0.1".to_string(), "localhost".to_string()]).unwrap();
    p.distinguished_name = rcgen::DistinguishedName::new();
    p.distinguished_name.push(DnType::CommonName, cn);
    p
}

fn self_signed(cn: &str) -> Identity {
    let key = KeyPair::generate().unwrap();
    let cert = leaf_params(cn).self_signed(&key).unwrap();
    Identity {
        chain: vec![cert.der().to_vec()],
        key: key.serialize_der(),
    }
}

/// A leaf signed by a private CA; the hub presents leaf and CA.
fn private_ca(cn: &str) -> Identity {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.distinguished_name = rcgen::DistinguishedName::new();
    ca.distinguished_name
        .push(DnType::CommonName, "Scratch Test CA");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca.self_signed(&ca_key).unwrap();
    let key = KeyPair::generate().unwrap();
    let leaf = leaf_params(cn).signed_by(&key, &ca_cert, &ca_key).unwrap();
    Identity {
        chain: vec![leaf.der().to_vec(), ca_cert.der().to_vec()],
        key: key.serialize_der(),
    }
}

/// TLS in front of the plain-HTTP hub: one request per connection, relayed
/// with `Connection: close`. The certificate can be swapped while running.
struct TlsFront {
    url: String,
    addr: SocketAddr,
    config: Arc<Mutex<Arc<ServerConfig>>>,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl TlsFront {
    fn start(upstream: SocketAddr, id: &Identity) -> TlsFront {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let config = Arc::new(Mutex::new(id.server_config()));
        let stop = Arc::new(AtomicBool::new(false));
        let (cfg, st) = (config.clone(), stop.clone());
        let accept = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if st.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { break };
                let c = cfg.lock().unwrap().clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, c, upstream);
                });
            }
        });
        TlsFront {
            url: format!("https://{addr}"),
            addr,
            config,
            stop,
            accept: Some(accept),
        }
    }

    fn set_identity(&self, id: &Identity) {
        *self.config.lock().unwrap() = id.server_config();
    }
}

impl Drop for TlsFront {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
    }
}

fn serve(
    stream: TcpStream,
    config: Arc<ServerConfig>,
    upstream: SocketAddr,
) -> std::io::Result<()> {
    let conn = ServerConnection::new(config).map_err(std::io::Error::other)?;
    let mut tls = StreamOwned::new(conn, stream);
    let (start, headers, body) = read_message(&mut BufReader::new(&mut tls))?;
    let mut up = TcpStream::connect(upstream)?;
    let mut head = format!("{start}\r\n");
    for (k, v) in headers
        .iter()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("connection"))
    {
        head += &format!("{k}: {v}\r\n");
    }
    head += "Connection: close\r\n\r\n";
    up.write_all(head.as_bytes())?;
    up.write_all(&body)?;
    let (status, rh, rbody) = read_answer(&mut BufReader::new(up))?;
    let mut out = format!("{status}\r\n");
    for (k, v) in rh
        .iter()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("connection"))
    {
        out += &format!("{k}: {v}\r\n");
    }
    out += "Connection: close\r\n\r\n";
    tls.write_all(out.as_bytes())?;
    tls.write_all(&rbody)?;
    tls.conn.send_close_notify();
    tls.flush()
}

/// A hub over plain HTTP with a TLS front.
struct TlsHub {
    _dir: TempDir,
    hub: Arc<Hub>,
    _server: HubServer,
    front: TlsFront,
}

impl TlsHub {
    fn start(id: &Identity) -> TlsHub {
        let dir = TempDir::new("tls_hub");
        let hub = Hub::new(dir.open()).unwrap();
        let server = hub.serve("127.0.0.1:0").unwrap();
        let front = TlsFront::start(server.addr(), id);
        TlsHub {
            _dir: dir,
            hub,
            _server: server,
            front,
        }
    }

    fn client(&self, label: &str, trusted: Vec<String>) -> (TempDir, SyncClient) {
        let dir = TempDir::new(label);
        let token = self.hub.pair(label).unwrap();
        let client = SyncClient::with_trusted(dir.open(), &self.front.url, &token, trusted);
        (dir, client)
    }
}

fn untrusted(c: &SyncClient) -> String {
    match c.sync_once() {
        Err(SyncError::UntrustedCertificate { sha256 }) => sha256,
        other => panic!("expected an untrusted certificate, got {other:?}"),
    }
}

#[test]
fn self_signed_hub_without_pins_is_untrusted() {
    let id = self_signed("hub.test");
    let hub = TlsHub::start(&id);
    let (_d, c) = hub.client("a", vec![]);
    assert!(c.untrusted_certificate().is_none());
    let err = c.sync_once().unwrap_err();
    let msg = err.to_string();
    assert!(msg.starts_with("untrusted certificate: "), "{msg}");
    assert!(!msg.starts_with("hub connection"), "{msg}");
    assert_eq!(
        err,
        SyncError::UntrustedCertificate {
            sha256: id.leaf_fingerprint()
        }
    );
    let chain = c.untrusted_certificate().expect("chain recorded");
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].sha256, id.leaf_fingerprint());
    assert_eq!(chain[0].subject, "CN=hub.test");
    assert!(chain[0].self_signed);
    // A client made with `new` (no pins) fails the same way.
    let plain = SyncClient::new(
        TempDir::new("plain").open(),
        &hub.front.url,
        &hub.hub.pair("plain").unwrap(),
    );
    assert_eq!(untrusted(&plain), id.leaf_fingerprint());
}

#[test]
fn pinned_self_signed_hub_syncs() {
    let id = self_signed("hub.test");
    let hub = TlsHub::start(&id);
    let (_da, a) = hub.client("a", vec![id.leaf_fingerprint()]);
    let (_db, b) = hub.client("b", vec![id.leaf_fingerprint()]);
    let page = a.vault().create_page(None, "Over TLS".into()).unwrap();
    a.vault()
        .apply(
            page.clone(),
            vec![BlockOp::Insert {
                parent: None,
                after: None,
                block: NewBlock {
                    id: "p1".into(),
                    kind: BlockKind::Paragraph,
                    text: "pinned hello".into(),
                    marks: vec![],
                    checked: false,
                },
            }],
        )
        .unwrap();
    a.sync_once().unwrap();
    b.sync_once().unwrap();
    let got = b.vault().load_page(page).unwrap();
    assert_eq!(got.blocks.len(), 1);
    assert_eq!(got.blocks[0].text, "pinned hello");
    assert!(a.untrusted_certificate().is_none());
    assert!(b.untrusted_certificate().is_none());
}

#[test]
fn changed_certificate_is_untrusted_again() {
    let old = self_signed("hub.test");
    let hub = TlsHub::start(&old);
    let (_d, c) = hub.client("a", vec![old.leaf_fingerprint()]);
    c.sync_once().unwrap();
    let new = self_signed("hub.test");
    hub.front.set_identity(&new);
    assert_eq!(untrusted(&c), new.leaf_fingerprint());
    assert_ne!(new.leaf_fingerprint(), old.leaf_fingerprint());
    let chain = c.untrusted_certificate().unwrap();
    assert_eq!(chain[0].sha256, new.leaf_fingerprint());
    // Back to the pinned certificate: syncs, and the chain is cleared.
    hub.front.set_identity(&old);
    c.sync_once().unwrap();
    assert!(c.untrusted_certificate().is_none());
}

#[test]
fn pin_in_lowercase_without_colons_is_accepted() {
    let id = self_signed("hub.test");
    let hub = TlsHub::start(&id);
    let pin = id.leaf_fingerprint().replace(':', "").to_lowercase();
    let (_d, c) = hub.client("a", vec!["not a fingerprint".into(), pin]);
    c.sync_once().unwrap();
}

#[test]
fn private_ca_chain_is_recorded_leaf_first() {
    let id = private_ca("hub.test");
    let hub = TlsHub::start(&id);
    let (_d, c) = hub.client("a", vec![]);
    assert_eq!(untrusted(&c), id.leaf_fingerprint());
    let chain = c.untrusted_certificate().unwrap();
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0].sha256, id.leaf_fingerprint());
    assert_eq!(chain[0].subject, "CN=hub.test");
    assert_eq!(chain[0].issuer, "CN=Scratch Test CA");
    assert!(!chain[0].self_signed);
    assert_eq!(chain[1].sha256, fingerprint(&id.chain[1]));
    assert_eq!(chain[1].subject, "CN=Scratch Test CA");
    assert!(chain[1].self_signed);
    // Pinning the leaf accepts it.
    let (_d2, pinned) = hub.client("b", vec![id.leaf_fingerprint()]);
    pinned.sync_once().unwrap();
}
