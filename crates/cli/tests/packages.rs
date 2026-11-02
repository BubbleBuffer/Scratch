//! `scratch-cli packages`: list, install and remove against a local server.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use scratch_packages::{current_platform, Manifest, Package, Store};
use sha2::{Digest, Sha256};

fn zip_bytes() -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut out);
        zip.start_file("hello.txt", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"hello").unwrap();
        zip.finish().unwrap();
    }
    out.into_inner()
}

fn serve(body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/p.zip", listener.local_addr().unwrap());
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    url
}

fn run(words: &[&str], json: bool, manifest: &Manifest, store: &Store) -> Result<String, String> {
    let mut out = Vec::new();
    scratch_cli::run_packages_with(words, json, &mut out, manifest, store)
        .map_err(|e| format!("{e:#}"))?;
    Ok(String::from_utf8(out).unwrap())
}

fn fixture() -> (tempfile::TempDir, Store, Manifest) {
    let bytes = zip_bytes();
    let package = Package {
        name: "claude-code".into(),
        version: "1.0.0".into(),
        platform: current_platform().into(),
        url: serve(bytes.clone()),
        sha256: Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect(),
        size: bytes.len() as u64,
    };
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path()).allow_insecure_http();
    (dir, store, Manifest { packages: vec![package] })
}

#[test]
fn list_install_remove() {
    let (_dir, store, manifest) = fixture();

    let listed = run(&["list"], false, &manifest, &store).unwrap();
    assert!(listed.contains("claude-code 1.0.0") && listed.contains("not installed"), "{listed}");

    let installed = run(&["install", "claude-code"], false, &manifest, &store).unwrap();
    assert!(installed.contains("installed claude-code 1.0.0"), "{installed}");
    assert!(installed.contains("downloading claude-code: 100%"), "{installed}");

    let json = run(&["list"], true, &manifest, &store).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["packages"][0]["state"], "installed");
    assert_eq!(value["packages"][0]["installedVersion"], "1.0.0");

    assert!(run(&["remove", "claude-code"], false, &manifest, &store).unwrap().contains("removed"));
    assert!(run(&["remove", "claude-code"], false, &manifest, &store).unwrap().contains("not installed"));
    assert!(store.installed().is_empty());
}

#[test]
fn unknown_package_and_empty_manifest() {
    let (_dir, store, manifest) = fixture();
    let err = run(&["install", "browser"], false, &manifest, &store).unwrap_err();
    assert!(err.contains("not available on this platform"), "{err}");

    let empty = Manifest::default();
    let listed = run(&["list"], false, &empty, &store).unwrap();
    assert!(listed.contains("no packages are available on this platform"), "{listed}");
    assert!(run(&["bogus"], false, &empty, &store).is_err());
}

#[test]
fn the_real_entry_point_reports_the_checked_in_manifest_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let args: Vec<String> = ["packages", "list", "--data-dir", dir.path().to_str().unwrap()]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut out = Vec::new();
    scratch_cli::run(&args, &mut out).unwrap();
    assert!(String::from_utf8(out).unwrap().contains("no packages are available"));
}
