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
/// Upper bound for the number of entries in one package.
const MAX_ENTRIES: usize = 200_000;
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
    ///
    /// Every version's marker goes first, so the package counts as removed
    /// at once, even when a file in use (an antivirus scan, a runtime that is
    /// still shutting down) keeps its folder on Windows. Such leftovers carry
    /// no marker; [`Store::has_leftovers`] reports them and
    /// [`Store::clean_leftovers`] (the next app start) or the next install
    /// deletes them. A marker that cannot be deleted is an error: the package
    /// is still installed then.
    pub fn remove(&self, name: &str) -> Result<bool, Error> {
        check_token("name", name, |c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')?;
        let dir = self.root.join(name);
        if !dir.is_dir() {
            return Ok(false);
        }
        let _lock = PackageLock::acquire(&dir)?;
        let had = self.get(name).is_some();
        let paths: Vec<PathBuf> = fs::read_dir(&dir)?
            .flatten()
            .filter(|e| e.file_name() != LOCK)
            .map(|e| e.path())
            .collect();
        for path in &paths {
            let marker = path.join(MARKER);
            if marker.is_file() {
                fs::remove_file(&marker)?;
            }
        }
        for path in &paths {
            // A runtime that was just stopped may need a moment to let go.
            for attempt in 0..3 {
                sweep(path);
                if !path.exists() {
                    break;
                }
                if attempt < 2 {
                    std::thread::sleep(Duration::from_millis(300));
                }
            }
        }
        Ok(had)
    }

    /// Whether `name` has files on disk that no installed version owns: what
    /// a [`Store::remove`] could not delete.
    pub fn has_leftovers(&self, name: &str) -> bool {
        fs::read_dir(self.root.join(name))
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| e.file_name() != LOCK && !e.path().join(MARKER).is_file())
    }

    /// Deletes what no installed version of `name` owns: leftovers of a
    /// removal, and temp files of an interrupted install. Does nothing while
    /// another process installs or removes the package. Call it before
    /// anything loads from the package folder.
    pub fn clean_leftovers(&self, name: &str) {
        let slug = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
        if check_token("name", name, slug).is_err() {
            return;
        }
        let dir = self.root.join(name);
        if let Some(_lock) = PackageLock::try_acquire(&dir) {
            sweep_unowned(&dir);
        }
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
        // A folder a removal could not delete would be taken for the target.
        sweep_unowned(&dir);

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

/// Deletes what is not an installed version in `dir` (the caller holds the
/// lock): folders without a marker and `.tmp-*` files.
fn sweep_unowned(dir: &Path) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == LOCK {
            continue;
        }
        let owned = path.join(MARKER).is_file();
        if !owned && (path.is_dir() || name.to_string_lossy().starts_with(".tmp")) {
            sweep(&path);
        }
    }
}

/// Exclusive advisory lock on `<package dir>/.lock`, released on drop.
struct PackageLock(File);

impl PackageLock {
    fn open(dir: &Path) -> io::Result<File> {
        fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(LOCK))
    }

    fn acquire(dir: &Path) -> Result<PackageLock, Error> {
        let file = PackageLock::open(dir)?;
        file.lock()?;
        Ok(PackageLock(file))
    }
}

impl PackageLock {
    /// The lock, or None when another process holds it.
    fn try_acquire(dir: &Path) -> Option<PackageLock> {
        let file = PackageLock::open(dir).ok()?;
        file.try_lock().ok()?;
        Some(PackageLock(file))
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

/// What an archive may hold: total unpacked bytes (counted as written, not as
/// declared) and entries.
struct Limits {
    max_unpacked: u64,
    max_entries: usize,
}

const LIMITS: Limits = Limits { max_unpacked: MAX_UNPACKED, max_entries: MAX_ENTRIES };

/// The entry name as a relative path of plain components, or an error. This
/// does not rely on the zip crate: absolute paths, drive letters, backslashes,
/// `..`, empty names and NULs are refused.
fn safe_relative(name: &str) -> Result<PathBuf, Error> {
    let bad = || Error::BadArchive(format!("unsafe path {name:?}"));
    if name.is_empty() || name.starts_with('/') || name.contains(['\\', ':', '\0']) {
        return Err(bad());
    }
    let mut rel = PathBuf::new();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(bad()),
            part => rel.push(part),
        }
    }
    if rel.as_os_str().is_empty() {
        return Err(bad());
    }
    // Belt and braces: only plain components may remain.
    if rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return Err(bad());
    }
    Ok(rel)
}

/// Unpacks `zip_path` into `dest`, refusing symlinks, paths that are absolute
/// or leave `dest`, duplicate names (case-insensitive, as on Windows) and
/// archives over the entry or size limits.
fn unpack(zip_path: &Path, dest: &Path) -> Result<(), Error> {
    unpack_with(zip_path, dest, &LIMITS)
}

/// The entry names as the archive's own central directory lists them, read
/// without the zip crate, so a parser differential (a duplicate or hidden
/// entry the crate reads differently) is caught. The end-of-central-directory
/// record is the last one whose comment length reaches exactly to the end of
/// the file, so a signature inside a comment is not taken for it. Zip64
/// archives are refused: a package is far below their limits.
fn central_directory_names(path: &Path) -> Result<Vec<Vec<u8>>, Error> {
    use std::io::{Seek, SeekFrom};
    let bad = |m: &str| Error::BadArchive(m.to_string());
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let tail_len = len.min(22 + 65_535);
    file.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = vec![0u8; tail_len as usize];
    file.read_exact(&mut tail)?;
    let eocd = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| {
            tail[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]
                && i + 22 + u16::from_le_bytes([tail[i + 20], tail[i + 21]]) as usize == tail.len()
        })
        .ok_or_else(|| bad("no end of central directory"))?;
    let rec = &tail[eocd..];
    let u16_at = |b: &[u8], o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as usize;
    let u32_at = |b: &[u8], o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as u64;
    let (count, size, offset) = (u16_at(rec, 10), u32_at(rec, 12), u32_at(rec, 16));
    if count == 0xFFFF || size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF {
        return Err(bad("zip64 archives are not supported"));
    }
    if offset.checked_add(size).is_none_or(|end| end > len) || size > 256 << 20 {
        return Err(bad("central directory out of range"));
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut cd = vec![0u8; size as usize];
    file.read_exact(&mut cd)?;
    let mut names = Vec::with_capacity(count);
    let mut at = 0usize;
    for _ in 0..count {
        let head = cd.get(at..at + 46).ok_or_else(|| bad("truncated central directory"))?;
        if head[..4] != [0x50, 0x4b, 0x01, 0x02] {
            return Err(bad("bad central directory entry"));
        }
        let (n, x, c) = (u16_at(head, 28), u16_at(head, 30), u16_at(head, 32));
        let name = cd.get(at + 46..at + 46 + n).ok_or_else(|| bad("truncated central directory"))?;
        names.push(name.to_vec());
        at += 46 + n + x + c;
    }
    if at != cd.len() {
        return Err(bad("central directory has trailing data"));
    }
    Ok(names)
}

fn unpack_with(zip_path: &Path, dest: &Path, limits: &Limits) -> Result<(), Error> {
    let bad = |e: zip::result::ZipError| Error::BadArchive(e.to_string());
    let mut archive = zip::ZipArchive::new(File::open(zip_path)?).map_err(bad)?;
    if archive.len() > limits.max_entries {
        return Err(Error::BadArchive(format!("more than {} entries", limits.max_entries)));
    }
    // The archive's own list of names must be the one the crate reads, and
    // free of duplicates (case-insensitive, as on Windows).
    let listed = central_directory_names(zip_path)?;
    let mut seen = std::collections::HashSet::new();
    for raw in &listed {
        if !seen.insert(raw.to_ascii_lowercase()) {
            return Err(Error::BadArchive(format!(
                "duplicate entry {:?}",
                String::from_utf8_lossy(raw)
            )));
        }
    }
    if listed.len() != archive.len() {
        return Err(Error::BadArchive("entry count differs from the central directory".to_string()));
    }
    let mut seen = std::collections::HashSet::new();
    let mut unpacked: u64 = 0;
    #[allow(clippy::needless_range_loop)]
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(bad)?;
        let name = entry.name().map_err(bad)?.into_owned();
        let rel = safe_relative(&name)?;
        if listed[i].as_slice() != name.as_bytes() {
            return Err(Error::BadArchive(format!("entry {i} differs from the central directory")));
        }
        if !seen.insert(rel.to_string_lossy().to_lowercase()) {
            return Err(Error::BadArchive(format!("duplicate entry {name:?}")));
        }
        // 0o120000 is S_IFLNK; checked in addition to the crate's own test.
        let linked = entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000);
        if entry.is_symlink() || linked {
            return Err(Error::BadArchive(format!("symlink {name:?}")));
        }
        let out = dest.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&out)?;
        let budget = limits.max_unpacked - unpacked;
        let written = io::copy(&mut entry.by_ref().take(budget.saturating_add(1)), &mut file)?;
        if written > budget {
            return Err(Error::BadArchive("unpacked size too large".to_string()));
        }
        unpacked += written;
        file.flush()?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o777));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(f: impl FnOnce(&mut zip::ZipWriter<io::Cursor<Vec<u8>>>)) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        f(&mut zip);
        zip.finish().unwrap().into_inner()
    }

    fn opts() -> zip::write::SimpleFileOptions {
        zip::write::SimpleFileOptions::default()
    }

    /// Unpacks `bytes` into a fresh dir; returns the dir and the result.
    fn run(bytes: &[u8], limits: &Limits) -> (tempfile::TempDir, Result<(), Error>) {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("p.zip");
        fs::write(&zip_path, bytes).unwrap();
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        let result = unpack_with(&zip_path, &dest, limits);
        (dir, result)
    }

    fn roomy() -> Limits {
        Limits { max_unpacked: 1 << 20, max_entries: 100 }
    }

    fn rejects(bytes: &[u8], limits: &Limits, what: &str) -> tempfile::TempDir {
        let (dir, result) = run(bytes, limits);
        match result {
            Err(Error::BadArchive(m)) => assert!(m.contains(what), "{m}"),
            other => panic!("expected BadArchive({what}), got {other:?}"),
        }
        dir
    }

    #[test]
    fn plain_archive_unpacks() {
        let bytes = build(|z| {
            z.add_directory("d/", opts()).unwrap();
            z.start_file("d/a.txt", opts()).unwrap();
            z.write_all(b"a").unwrap();
        });
        let (dir, result) = run(&bytes, &roomy());
        result.unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("out/d/a.txt")).unwrap(), "a");
    }

    #[test]
    fn rejects_parent_traversal() {
        for name in ["../evil.txt", "a/../../evil.txt", "a/../b.txt"] {
            let bytes = build(|z| {
                z.start_file(name, opts()).unwrap();
                z.write_all(b"x").unwrap();
            });
            let dir = rejects(&bytes, &roomy(), "unsafe path");
            assert!(!dir.path().join("evil.txt").exists());
        }
    }

    #[test]
    fn rejects_absolute_and_drive_paths() {
        for name in ["/etc/evil", "C:/evil.txt", "C:evil.txt", "\\\\server\\share\\x", "a\\..\\b.txt"] {
            let bytes = build(|z| {
                z.start_file(name, opts()).unwrap();
                z.write_all(b"x").unwrap();
            });
            rejects(&bytes, &roomy(), "unsafe path");
        }
    }

    #[test]
    fn rejects_symlinks() {
        let bytes = build(|z| {
            z.add_symlink("link", "/etc/passwd", opts()).unwrap();
        });
        rejects(&bytes, &roomy(), "symlink");
    }

    #[test]
    fn rejects_duplicate_names() {
        let mut bytes = build(|z| {
            for name in ["a.txt", "b.txt"] {
                z.start_file(name, opts()).unwrap();
                z.write_all(b"x").unwrap();
            }
        });
        // Same-length rename of the second entry in its local and central headers.
        for i in 0..bytes.len() - 5 {
            if &bytes[i..i + 5] == b"b.txt" {
                bytes[i..i + 5].copy_from_slice(b"a.txt");
            }
        }
        rejects(&bytes, &roomy(), "duplicate");
    }

    #[test]
    fn rejects_names_that_differ_only_by_case() {
        let bytes = build(|z| {
            for name in ["a.txt", "A.TXT"] {
                z.start_file(name, opts()).unwrap();
                z.write_all(b"x").unwrap();
            }
        });
        rejects(&bytes, &roomy(), "duplicate");
    }

    #[test]
    fn rejects_too_many_entries() {
        let bytes = build(|z| {
            for i in 0..5 {
                z.start_file(format!("f{i}"), opts()).unwrap();
            }
        });
        rejects(&bytes, &Limits { max_unpacked: 1 << 20, max_entries: 4 }, "entries");
    }

    #[test]
    fn rejects_too_much_unpacked_data() {
        let bytes = build(|z| {
            for i in 0..3 {
                z.start_file(format!("f{i}"), opts()).unwrap();
                z.write_all(&[7u8; 600]).unwrap();
            }
        });
        // 1800 bytes against a 1000 byte budget, counted across entries.
        rejects(&bytes, &Limits { max_unpacked: 1000, max_entries: 100 }, "too large");
    }
}
