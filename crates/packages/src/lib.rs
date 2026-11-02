//! On-demand runtime packages for Scratch add-ons.
//!
//! An add-on that needs external dependencies (today: the Claude Code
//! bundle) loads them on demand instead of shipping them in the installer.
//! A package is one zip per platform, built in the release workflow and
//! attached to the same GitHub release. The app build embeds a
//! [`Manifest`] (name, version, platform, asset URL, sha256, size), so the
//! checksum is pinned in the binary.
//!
//! A [`Store`] installs packages under `<data dir>/packages/<name>/<version>/`:
//! download to a temp file, verify size and sha256, extract to a temp dir,
//! rename into place, then delete older versions. The app (the bridge) and
//! `scratch-cli` share one store; an advisory lock per package keeps their
//! installs apart.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The manifest embedded in this build. CI writes the real one to
/// `crates/packages/manifest.json` before the app build; the checked-in file
/// lists no packages.
pub const EMBEDDED_MANIFEST: &str = include_str!("../manifest.json");

/// Name of the per-version marker file written into every installed package.
const MARKER: &str = ".package.json";
/// Temp names inside a package folder; swept under the lock.
const TMP_DOWNLOAD: &str = ".tmp-download";
const TMP_EXTRACT: &str = ".tmp-extract";
const TMP_OLD: &str = ".tmp-old";
const LOCK: &str = ".lock";
/// Upper bound for the unpacked size of one package (zip bomb guard).
const MAX_UNPACKED: u64 = 4 << 30;
/// Progress is reported about every this many downloaded bytes.
const PROGRESS_STEP: u64 = 256 << 10;

/// The platform key of this build, as used in manifests and zip names.
pub fn current_platform() -> &'static str {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows-x64"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x64"
    } else if cfg!(target_os = "android") {
        "android-arm64"
    } else {
        "unsupported"
    }
}

/// One downloadable package for one platform.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    pub name: String,
    /// Content version; names the install folder.
    pub version: String,
    pub platform: String,
    /// HTTPS URL of the zip.
    pub url: String,
    /// Lowercase hex sha256 of the zip.
    pub sha256: String,
    /// Size of the zip in bytes.
    pub size: u64,
}

impl Package {
    /// Checks that names are safe as folder names and the hash is well formed.
    pub fn validate(&self) -> Result<(), Error> {
        let slug = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
        check_token("name", &self.name, slug)?;
        check_token("version", &self.version, |c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-')
        })?;
        check_token("platform", &self.platform, slug)?;
        let hex_ok = self.sha256.len() == 64
            && self.sha256.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
        if !hex_ok {
            return Err(Error::Manifest(format!("{}: sha256 is not 64 lowercase hex digits", self.name)));
        }
        Ok(())
    }
}

/// A list of packages keyed by name and platform.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub packages: Vec<Package>,
}

impl Manifest {
    /// Parses and validates manifest JSON.
    pub fn parse(json: &str) -> Result<Manifest, Error> {
        let manifest: Manifest =
            serde_json::from_str(json).map_err(|e| Error::Manifest(e.to_string()))?;
        for p in &manifest.packages {
            p.validate()?;
        }
        Ok(manifest)
    }

    /// The manifest embedded in this build.
    pub fn embedded() -> Manifest {
        Manifest::parse(EMBEDDED_MANIFEST).unwrap_or_default()
    }

    /// The package `name` for `platform`, if the manifest lists it.
    pub fn find(&self, name: &str, platform: &str) -> Option<&Package> {
        self.packages
            .iter()
            .find(|p| p.name == name && p.platform == platform)
    }

    /// Every package listed for `platform`.
    pub fn for_platform<'a>(&'a self, platform: &'a str) -> impl Iterator<Item = &'a Package> {
        self.packages.iter().filter(move |p| p.platform == platform)
    }
}

fn check_token(what: &str, value: &str, ok: impl Fn(char) -> bool) -> Result<(), Error> {
    if value.is_empty() || value.starts_with('.') || !value.chars().all(ok) {
        return Err(Error::Manifest(format!("invalid package {what} {value:?}")));
    }
    Ok(())
}

/// An installed package version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub name: String,
    pub version: String,
    /// sha256 of the zip this version was installed from.
    pub sha256: String,
    /// Unpacked size on disk, in bytes.
    pub size_bytes: u64,
    /// The version folder.
    pub path: PathBuf,
}

/// Where a package stands against the manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    NotInstalled,
    /// The installed version matches the manifest's version and hash.
    Current(Installed),
    /// Something else is installed; the manifest's package differs.
    UpdateAvailable(Installed),
}

/// What an install is doing, for the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Downloading { done: u64, total: u64 },
    Verifying,
    Extracting,
    Installing,
}

#[derive(Debug)]
pub enum Error {
    /// The manifest is not valid.
    Manifest(String),
    /// The package has no build for this platform.
    Unavailable { name: String, platform: String },
    /// The URL is not HTTPS.
    InsecureUrl(String),
    /// The server could not be reached or the connection broke.
    Offline(String),
    /// The server answered with an error status.
    Http(u16),
    /// The download has another size than the manifest says.
    SizeMismatch { expected: u64, actual: u64 },
    /// The download's sha256 differs from the manifest's.
    ChecksumMismatch { expected: String, actual: String },
    /// The zip cannot be unpacked safely.
    BadArchive(String),
    DiskFull,
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Manifest(m) => write!(f, "package manifest invalid: {m}"),
            Error::Unavailable { name, platform } => {
                write!(f, "{name} is not available on this platform ({platform})")
            }
            Error::InsecureUrl(u) => write!(f, "package URL is not HTTPS: {u}"),
            Error::Offline(m) => write!(f, "download failed, check the connection: {m}"),
            Error::Http(code) => write!(f, "download failed, server answered {code}"),
            Error::SizeMismatch { expected, actual } => {
                write!(f, "download has the wrong size ({actual} bytes, expected {expected})")
            }
            Error::ChecksumMismatch { .. } => write!(f, "download failed the checksum check"),
            Error::BadArchive(m) => write!(f, "package archive is invalid: {m}"),
            Error::DiskFull => write!(f, "not enough disk space"),
            Error::Io(e) => write!(f, "file error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        if e.kind() == io::ErrorKind::StorageFull {
            Error::DiskFull
        } else {
            Error::Io(e)
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Marker {
    name: String,
    version: String,
    sha256: String,
}

/// The packages folder of one data dir.
#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
    allow_http: bool,
}

impl Store {
    /// The store under `<data_dir>/packages`.
    pub fn new(data_dir: &Path) -> Store {
        Store { root: data_dir.join("packages"), allow_http: false }
    }

    /// Accepts `http://` URLs. Tests against a local server only.
    #[doc(hidden)]
    pub fn allow_insecure_http(mut self) -> Store {
        self.allow_http = true;
        self
    }

    /// The packages folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The current installed version of `name`, if any. When several
    /// version folders exist (an update was interrupted before cleanup),
    /// the most recently installed wins.
    pub fn get(&self, name: &str) -> Option<Installed> {
        let dir = self.root.join(name);
        let mut best: Option<(std::time::SystemTime, Installed)> = None;
        for entry in fs::read_dir(&dir).ok()?.flatten() {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            let marker_path = path.join(MARKER);
            let Some(marker) = read_marker(&marker_path) else { continue };
            if marker.name != name {
                continue;
            }
            let stamp = fs::metadata(&marker_path)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if best.as_ref().is_none_or(|(t, _)| stamp > *t) {
                best = Some((
                    stamp,
                    Installed {
                        name: name.to_string(),
                        version: marker.version,
                        sha256: marker.sha256,
                        size_bytes: dir_size(&path),
                        path,
                    },
                ));
            }
        }
        best.map(|(_, i)| i)
    }

    /// Every installed package (its current version), sorted by name.
    pub fn installed(&self) -> Vec<Installed> {
        let mut out: Vec<Installed> = fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    return None;
                }
                self.get(&name)
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Where `package` stands against what is installed.
    pub fn state(&self, package: &Package) -> State {
        match self.get(&package.name) {
            None => State::NotInstalled,
            Some(i) if i.version == package.version && i.sha256 == package.sha256 => State::Current(i),
            Some(i) => State::UpdateAvailable(i),
        }
    }

    /// Removes every version of `name`. Returns whether anything was installed.
    pub fn remove(&self, name: &str) -> Result<bool, Error> {
        check_token("name", name, |c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')?;
        let dir = self.root.join(name);
        if !dir.is_dir() {
            return Ok(false);
        }
        let _lock = PackageLock::acquire(&dir)?;
        let had = self.get(name).is_some();
        for entry in fs::read_dir(&dir)?.flatten() {
            if entry.file_name() == LOCK {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
        }
        Ok(had)
    }

    /// Downloads, verifies and installs `package`. Safe to call from several
    /// threads or processes at once: the others wait, then find it installed.
    /// An existing install stays usable until the new one is in place.
    pub fn install(
        &self,
        package: &Package,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Installed, Error> {
        if !(package.url.starts_with("https://") || (self.allow_http && package.url.starts_with("http://"))) {
            return Err(Error::InsecureUrl(package.url.clone()));
        }
        package.validate()?;

        let dir = self.root.join(&package.name);
        fs::create_dir_all(&dir)?;
        let _lock = PackageLock::acquire(&dir)?;

        if let State::Current(installed) = self.state(package) {
            return Ok(installed);
        }

        // Leftovers of an interrupted install: nobody else holds the lock.
        let download = dir.join(TMP_DOWNLOAD);
        let extract = dir.join(TMP_EXTRACT);
        let old = dir.join(TMP_OLD);
        sweep(&download);
        sweep(&extract);
        sweep(&old);

        let result = self.install_locked(package, &dir, &download, &extract, &old, progress);
        sweep(&download);
        sweep(&extract);
        sweep(&old);
        result
    }

    fn install_locked(
        &self,
        package: &Package,
        dir: &Path,
        download: &Path,
        extract: &Path,
        old: &Path,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Installed, Error> {
        fetch(package, download, progress)?;

        progress(Progress::Extracting);
        fs::create_dir_all(extract)?;
        unpack(download, extract)?;
        let marker = Marker {
            name: package.name.clone(),
            version: package.version.clone(),
            sha256: package.sha256.clone(),
        };
        fs::write(extract.join(MARKER), serde_json::to_vec(&marker).unwrap_or_default())?;

        progress(Progress::Installing);
        let target = dir.join(&package.version);
        if target.exists() {
            // Same version, different build: swap, and put the old one back
            // if the new one cannot be moved in.
            fs::rename(&target, old)?;
            if let Err(e) = fs::rename(extract, &target) {
                let _ = fs::rename(old, &target);
                return Err(e.into());
            }
        } else {
            fs::rename(extract, &target)?;
        }

        // Only now do older versions go. Best effort: a file in use on
        // Windows keeps its folder, and the next install retries.
        for entry in fs::read_dir(dir)?.flatten() {
            let path = entry.path();
            let skip = entry.file_name().to_string_lossy().starts_with('.') || path == target;
            if !skip && path.is_dir() {
                let _ = fs::remove_dir_all(&path);
            }
        }
        self.get(&package.name)
            .ok_or_else(|| Error::Io(io::Error::other("installed package not found")))
    }
}

fn read_marker(path: &Path) -> Option<Marker> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).into_iter().flatten().flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(_) => total += entry.metadata().map(|m| m.len()).unwrap_or(0),
                Err(_) => {}
            }
        }
    }
    total
}

fn sweep(path: &Path) {
    if path.is_dir() {
        let _ = fs::remove_dir_all(path);
    } else {
        let _ = fs::remove_file(path);
    }
}

/// Exclusive advisory lock on `<package dir>/.lock`, released on drop.
struct PackageLock(File);

impl PackageLock {
    fn acquire(dir: &Path) -> Result<PackageLock, Error> {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(LOCK))?;
        file.lock()?;
        Ok(PackageLock(file))
    }
}

impl Drop for PackageLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_recv_body(Some(Duration::from_secs(120)))
        .build()
        .into()
}

fn map_ureq(e: ureq::Error) -> Error {
    match e {
        ureq::Error::StatusCode(code) => Error::Http(code),
        ureq::Error::Io(e) if e.kind() == io::ErrorKind::StorageFull => Error::DiskFull,
        other => Error::Offline(other.to_string()),
    }
}

/// Downloads `package.url` to `dest`, checking the size and sha256.
fn fetch(package: &Package, dest: &Path, progress: &mut dyn FnMut(Progress)) -> Result<(), Error> {
    let response = agent().get(&package.url).call().map_err(map_ureq)?;
    let mut body = response.into_body();
    // One byte more than expected: a longer body is a size error, not a hang.
    let mut reader = body.as_reader().take(package.size.saturating_add(1));
    let mut file = File::create(dest)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 << 10];
    let mut done: u64 = 0;
    let mut reported: u64 = 0;
    progress(Progress::Downloading { done: 0, total: package.size });
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == io::ErrorKind::StorageFull => return Err(Error::DiskFull),
            Err(e) => return Err(Error::Offline(e.to_string())),
        };
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        done += n as u64;
        if done - reported >= PROGRESS_STEP {
            reported = done;
            progress(Progress::Downloading { done: done.min(package.size), total: package.size });
        }
    }
    file.flush()?;
    file.sync_all()?;
    drop(file);
    progress(Progress::Downloading { done: done.min(package.size), total: package.size });
    if done != package.size {
        return Err(Error::SizeMismatch { expected: package.size, actual: done });
    }
    progress(Progress::Verifying);
    let actual = hex(&hasher.finalize());
    if actual != package.sha256 {
        return Err(Error::ChecksumMismatch { expected: package.sha256.clone(), actual });
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Unpacks `zip_path` into `dest`, refusing paths that leave it, symlinks and
/// oversized archives.
fn unpack(zip_path: &Path, dest: &Path) -> Result<(), Error> {
    let bad = |e: zip::result::ZipError| Error::BadArchive(e.to_string());
    let mut archive = zip::ZipArchive::new(File::open(zip_path)?).map_err(bad)?;
    let mut unpacked: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(bad)?;
        let rel = entry
            .enclosed_name()
            .ok_or_else(|| Error::BadArchive(format!("unsafe path {:?}", entry.name())))?;
        if entry.is_symlink() {
            return Err(Error::BadArchive(format!("symlink {:?}", entry.name())));
        }
        let out = dest.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        unpacked = unpacked.saturating_add(entry.size());
        if unpacked > MAX_UNPACKED {
            return Err(Error::BadArchive("unpacked size too large".to_string()));
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&out)?;
        io::copy(&mut entry.by_ref().take(MAX_UNPACKED), &mut file)?;
        file.flush()?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o777));
        }
    }
    Ok(())
}
