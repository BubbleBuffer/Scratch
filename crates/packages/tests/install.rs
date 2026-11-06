//! Install tests against local HTTP servers: good, bad hash, truncated,
//! interrupted, concurrent, update.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use scratch_packages::{current_platform, Error, Manifest, Package, Progress, State, Store};
use sha2::{Digest, Sha256};

fn zip_bytes(files: &[(&str, &str)]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut out);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, body) in files {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    out.into_inner()
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn package(version: &str, url: &str, bytes: &[u8]) -> Package {
    Package {
        name: "claude-code".into(),
        version: version.into(),
        platform: current_platform().into(),
        url: url.into(),
        sha256: sha(bytes),
        size: bytes.len() as u64,
    }
}

/// Serves `body` for every GET on a random port; counts the requests.
fn serve(body: Vec<u8>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/pkg.zip", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            counter.fetch_add(1, Ordering::SeqCst);
            let body = body.clone();
            thread::spawn(move || respond(stream, &body, body.len(), body.len()));
        }
    });
    (url, hits)
}

/// Answers one request claiming `declared` bytes, sending only `send`.
fn respond(mut stream: TcpStream, body: &[u8], declared: usize, send: usize) {
    let mut buf = [0u8; 2048];
    let _ = stream.read(&mut buf);
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body[..send.min(body.len())]);
}

/// A server that breaks the connection after `send` bytes of `declared`.
fn serve_broken(body: Vec<u8>, declared: usize, send: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/pkg.zip", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            respond(stream, &body, declared, send);
        }
    });
    url
}

fn store(dir: &tempfile::TempDir) -> Store {
    Store::new(dir.path()).allow_insecure_http()
}

fn no_progress(_: Progress) {}

fn leftovers(store: &Store) -> Vec<String> {
    std::fs::read_dir(store.root().join("claude-code"))
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with(".tmp"))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn good_install_unpacks_marks_and_reports_progress() {
    let bytes = zip_bytes(&[("bun.exe", "bun"), ("claude-code/index.js", "js")]);
    let (url, _) = serve(bytes.clone());
    let pkg = package("1.0.0", &url, &bytes);
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);

    assert_eq!(store.state(&pkg), State::NotInstalled);
    let mut seen = Vec::new();
    let installed = store.install(&pkg, &mut |p| seen.push(p)).unwrap();

    assert_eq!(installed.version, "1.0.0");
    assert_eq!(installed.path, dir.path().join("packages/claude-code/1.0.0"));
    assert_eq!(std::fs::read_to_string(installed.path.join("bun.exe")).unwrap(), "bun");
    assert_eq!(std::fs::read_to_string(installed.path.join("claude-code/index.js")).unwrap(), "js");
    assert!(installed.size_bytes >= 5);
    assert_eq!(store.state(&pkg), State::Current(installed.clone()));
    assert_eq!(store.installed(), vec![installed]);
    assert!(leftovers(&store).is_empty());
    assert_eq!(seen.last(), Some(&Progress::Installing));
    assert!(seen.contains(&Progress::Downloading { done: bytes.len() as u64, total: bytes.len() as u64 }));
    assert!(seen.contains(&Progress::Verifying) && seen.contains(&Progress::Extracting));
}

#[test]
fn bad_hash_installs_nothing() {
    let bytes = zip_bytes(&[("a.txt", "a")]);
    let (url, _) = serve(bytes.clone());
    let mut pkg = package("1.0.0", &url, &bytes);
    pkg.sha256 = sha(b"something else");
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);

    let err = store.install(&pkg, &mut no_progress).unwrap_err();
    assert!(matches!(err, Error::ChecksumMismatch { .. }), "{err}");
    assert!(store.installed().is_empty());
    assert!(!dir.path().join("packages/claude-code/1.0.0").exists());
    assert!(leftovers(&store).is_empty());
}

#[test]
fn short_body_with_honest_length_is_a_size_error() {
    let bytes = zip_bytes(&[("a.txt", "a")]);
    let half = bytes[..bytes.len() / 2].to_vec();
    let (url, _) = serve(half);
    let pkg = package("1.0.0", &url, &bytes);
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);

    let err = store.install(&pkg, &mut no_progress).unwrap_err();
    assert!(matches!(err, Error::SizeMismatch { .. }), "{err}");
    assert!(store.installed().is_empty());
    assert!(leftovers(&store).is_empty());
}

#[test]
fn longer_body_than_declared_is_a_size_error() {
    let bytes = zip_bytes(&[("a.txt", "a")]);
    let mut longer = bytes.clone();
    longer.extend_from_slice(&[0; 64]);
    let (url, _) = serve(longer);
    let pkg = package("1.0.0", &url, &bytes);
    let dir = tempfile::tempdir().unwrap();

    let err = store(&dir).install(&pkg, &mut no_progress).unwrap_err();
    assert!(matches!(err, Error::SizeMismatch { .. }), "{err}");
}

#[test]
fn interrupted_download_fails_then_retry_starts_clean() {
    let bytes = zip_bytes(&[("a.txt", "hello")]);
    let broken = serve_broken(bytes.clone(), bytes.len(), bytes.len() / 2);
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);

    let err = store.install(&package("1.0.0", &broken, &bytes), &mut no_progress).unwrap_err();
    assert!(matches!(err, Error::Offline(_) | Error::SizeMismatch { .. }), "{err}");
    assert!(store.installed().is_empty());
    assert!(leftovers(&store).is_empty());

    // A killed process leaves its temp files behind; the next install wipes them.
    let pkg_dir = dir.path().join("packages/claude-code");
    std::fs::write(pkg_dir.join(".tmp-download"), b"partial").unwrap();
    std::fs::create_dir_all(pkg_dir.join(".tmp-extract/half")).unwrap();

    let (url, _) = serve(bytes.clone());
    let installed = store.install(&package("1.0.0", &url, &bytes), &mut no_progress).unwrap();
    assert_eq!(std::fs::read_to_string(installed.path.join("a.txt")).unwrap(), "hello");
    assert!(leftovers(&store).is_empty());
}

#[test]
fn unreachable_server_is_offline() {
    let bytes = zip_bytes(&[("a.txt", "a")]);
    // A port nobody listens on.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let pkg = package("1.0.0", &format!("http://127.0.0.1:{port}/p.zip"), &bytes);
    let dir = tempfile::tempdir().unwrap();
    let err = store(&dir).install(&pkg, &mut no_progress).unwrap_err();
    assert!(matches!(err, Error::Offline(_)), "{err}");
}

#[test]
fn concurrent_installs_download_once() {
    let bytes = zip_bytes(&[("a.txt", "a")]);
    let (url, hits) = serve(bytes.clone());
    let pkg = package("1.0.0", &url, &bytes);
    let dir = tempfile::tempdir().unwrap();

    let handles: Vec<_> = (0..6)
        .map(|_| {
            let (pkg, root) = (pkg.clone(), dir.path().to_path_buf());
            thread::spawn(move || {
                Store::new(&root).allow_insecure_http().install(&pkg, &mut no_progress)
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap().unwrap();
    }
    assert_eq!(hits.load(Ordering::SeqCst), 1, "later installs find it installed");
    assert_eq!(store(&dir).installed().len(), 1);
    assert!(leftovers(&store(&dir)).is_empty());
}

#[test]
fn update_replaces_the_old_version_only_after_the_new_one_is_in() {
    let v1 = zip_bytes(&[("v.txt", "one")]);
    let v2 = zip_bytes(&[("v.txt", "two")]);
    let (url1, _) = serve(v1.clone());
    let (url2, _) = serve(v2.clone());
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let p1 = package("1.0.0", &url1, &v1);
    store.install(&p1, &mut no_progress).unwrap();

    // A failed update (bad hash) leaves the old version working.
    let mut bad = package("2.0.0", &url2, &v2);
    bad.sha256 = sha(b"nope");
    assert!(store.install(&bad, &mut no_progress).is_err());
    let kept = store.get("claude-code").unwrap();
    assert_eq!(kept.version, "1.0.0");
    assert_eq!(std::fs::read_to_string(kept.path.join("v.txt")).unwrap(), "one");

    let p2 = package("2.0.0", &url2, &v2);
    assert!(matches!(store.state(&p2), State::UpdateAvailable(_)));
    store.install(&p2, &mut no_progress).unwrap();
    assert!(!dir.path().join("packages/claude-code/1.0.0").exists());
    assert_eq!(store.get("claude-code").unwrap().version, "2.0.0");
    assert!(matches!(store.state(&p2), State::Current(_)));
}

#[test]
fn same_version_with_a_new_hash_is_an_update() {
    let a = zip_bytes(&[("v.txt", "a")]);
    let b = zip_bytes(&[("v.txt", "b")]);
    let (ua, _) = serve(a.clone());
    let (ub, _) = serve(b.clone());
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    store.install(&package("1.0.0", &ua, &a), &mut no_progress).unwrap();
    let pb = package("1.0.0", &ub, &b);
    assert!(matches!(store.state(&pb), State::UpdateAvailable(_)));
    let installed = store.install(&pb, &mut no_progress).unwrap();
    assert_eq!(std::fs::read_to_string(installed.path.join("v.txt")).unwrap(), "b");
    assert!(leftovers(&store).is_empty());
}

#[test]
fn https_only() {
    let bytes = zip_bytes(&[("a.txt", "a")]);
    let pkg = package("1.0.0", "http://example.invalid/p.zip", &bytes);
    let dir = tempfile::tempdir().unwrap();
    let err = Store::new(dir.path()).install(&pkg, &mut no_progress).unwrap_err();
    assert!(matches!(err, Error::InsecureUrl(_)), "{err}");
}

#[test]
fn archive_paths_cannot_escape() {
    let bytes = zip_bytes(&[("../evil.txt", "x")]);
    let (url, _) = serve(bytes.clone());
    let dir = tempfile::tempdir().unwrap();
    let err = store(&dir).install(&package("1.0.0", &url, &bytes), &mut no_progress).unwrap_err();
    assert!(matches!(err, Error::BadArchive(_)), "{err}");
    assert!(!dir.path().join("packages/claude-code/evil.txt").exists());
    assert!(!dir.path().join("packages/evil.txt").exists());
}

#[test]
fn remove_deletes_every_version() {
    let bytes = zip_bytes(&[("a.txt", "a")]);
    let (url, _) = serve(bytes.clone());
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    store.install(&package("1.0.0", &url, &bytes), &mut no_progress).unwrap();
    assert!(store.remove("claude-code").unwrap());
    assert!(store.installed().is_empty());
    assert!(!store.remove("claude-code").unwrap());
    assert!(store.remove("../x").is_err());
}

#[test]
fn manifest_parse_and_lookup() {
    let sum = sha(b"x");
    let json = format!(
        r#"{{"packages":[{{"name":"claude-code","version":"1.2.3","platform":"windows-x64","url":"https://h/p.zip","sha256":"{sum}","size":10}}]}}"#
    );
    let m = Manifest::parse(&json).unwrap();
    assert!(m.find("claude-code", "windows-x64").is_some());
    assert!(m.find("claude-code", "linux-x64").is_none());
    assert_eq!(m.for_platform("windows-x64").count(), 1);
    assert!(Manifest::parse(&json.replace("claude-code", "../x")).is_err());
    assert!(Manifest::parse(&json.replace(&sum, "ABC")).is_err());
    assert!(Manifest::parse("nope").is_err());
    assert!(Manifest::embedded().packages.is_empty() || Manifest::embedded().packages[0].validate().is_ok());
}

#[test]
fn files_a_removal_left_behind_are_not_an_install_and_go_at_the_next_clean() {
    let bytes = zip_bytes(&[("a.txt", "a")]);
    let (url, _) = serve(bytes.clone());
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let installed = store.install(&package("1.0.0", &url, &bytes), &mut no_progress).unwrap();
    // What a removal leaves when a file is in use: the version folder with
    // its files, but no marker.
    std::fs::remove_file(installed.path.join(".package.json")).unwrap();
    assert!(store.installed().is_empty(), "no marker: not installed");
    assert!(store.has_leftovers("claude-code"));
    std::fs::create_dir_all(store.root().join("claude-code/.tmp-extract")).unwrap();

    store.clean_leftovers();
    assert!(!installed.path.exists());
    assert!(!store.has_leftovers("claude-code"));
    assert!(leftovers(&store).is_empty());
    assert!(store.root().join("claude-code/.lock").exists());

    // An installed version is never touched by the cleaning.
    let again = store.install(&package("1.0.0", &url, &bytes), &mut no_progress).unwrap();
    store.clean_leftovers();
    assert!(again.path.join("a.txt").exists());
}

#[test]
fn install_replaces_an_unmarked_folder_left_by_a_removal() {
    let bytes = zip_bytes(&[("a.txt", "a")]);
    let (url, _) = serve(bytes.clone());
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let stale = store.root().join("claude-code/1.0.0");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("old.bin"), b"x").unwrap();
    let installed = store.install(&package("1.0.0", &url, &bytes), &mut no_progress).unwrap();
    assert!(installed.path.join("a.txt").exists());
    assert!(!installed.path.join("old.bin").exists());
}

#[cfg(unix)]
#[test]
fn remove_with_a_folder_it_cannot_empty_still_removes_the_package() {
    use std::os::unix::fs::PermissionsExt;
    let bytes = zip_bytes(&[("sub/a.txt", "a")]);
    let (url, _) = serve(bytes.clone());
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let installed = store.install(&package("1.0.0", &url, &bytes), &mut no_progress).unwrap();
    let sub = installed.path.join("sub");
    // A read-only folder: its file cannot be unlinked (unless run as root).
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();
    let stuck = std::fs::remove_file(sub.join("a.txt")).is_err();
    let had = store.remove("claude-code").unwrap();
    assert!(had);
    assert!(store.installed().is_empty(), "treated as removed");
    if stuck {
        assert!(store.has_leftovers("claude-code"), "the leftovers are reported");
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
        store.clean_leftovers();
        assert!(!store.has_leftovers("claude-code"));
    }
}

#[test]
fn a_failed_install_leaves_no_lock_that_blocks_the_retry() {
    let bytes = zip_bytes(&[("a.txt", "a")]);
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    // A port nobody listens on: the connection is refused at once.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let dead = package("1.0.0", &format!("http://127.0.0.1:{port}/pkg.zip"), &bytes);
    assert!(matches!(
        store.install(&dead, &mut no_progress),
        Err(Error::Offline(_))
    ));
    assert!(store.installed().is_empty());
    assert_eq!(leftovers(&store), Vec::<String>::new());
    // The retry takes the lock at once and installs.
    let (url, _) = serve(bytes.clone());
    store.install(&package("1.0.0", &url, &bytes), &mut no_progress).unwrap();
    assert_eq!(store.installed().len(), 1);
}
