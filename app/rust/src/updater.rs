//! Windows updates: which GitHub release to offer, fetching its installer
//! and SHA256SUMS.txt, verifying the download, and starting the installer
//! the way a person would (ShellExecute, visible window). The flow and the
//! reasons behind it are in wiki/architecture/windows-install.md.
//!
//! The pure parts (release choice, checksum parsing, file verification,
//! pruning) build and are tested on every target. The network client and
//! the installer launch are Windows-only; other targets never call them.

#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The public repository whose releases are offered.
pub const RELEASES_URL: &str =
    "https://api.github.com/repos/BubbleBuffer/Scratch/releases?per_page=30";
/// Release assets are only fetched from this prefix.
pub const DOWNLOAD_PREFIX: &str = "https://github.com/BubbleBuffer/Scratch/releases/download/";
/// The checksum file every release publishes.
pub const SUMS_NAME: &str = "SHA256SUMS.txt";
/// Tags of nightly builds contain this (`v0.1.0-nightly.20261027`).
pub const NIGHTLY_MARK: &str = "-nightly.";
/// A download in progress, renamed to the installer's name once verified.
const PARTIAL_SUFFIX: &str = ".partial";

/// A release as listed by the GitHub API (only the fields used here).
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug)]
pub enum UpdateError {
    /// No connection, DNS failure, timeout: retried quietly later.
    Offline(String),
    /// GitHub's unauthenticated rate limit: retried quietly later.
    RateLimited,
    Http(u16),
    BadResponse(String),
    /// SHA256SUMS.txt has no line for the installer.
    MissingChecksum(String),
    /// The download's SHA-256 differs from SHA256SUMS.txt; the file was deleted.
    Mismatch { file: String },
    /// The installer's publisher was rejected ([verify_publisher]).
    Publisher(String),
    Install(String),
    Io(io::Error),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Offline(e) => write!(f, "Couldn't reach GitHub ({e})"),
            Self::RateLimited => write!(f, "GitHub's rate limit was reached; trying again later"),
            Self::Http(code) => write!(f, "GitHub answered HTTP {code}"),
            Self::BadResponse(e) => write!(f, "Unexpected answer from GitHub ({e})"),
            Self::MissingChecksum(name) => write!(f, "{SUMS_NAME} has no checksum for {name}"),
            Self::Mismatch { file } => {
                write!(f, "The download of {file} was corrupted (checksum mismatch) and was deleted")
            }
            Self::Publisher(e) => write!(f, "The installer's publisher was not accepted ({e})"),
            Self::Install(e) => write!(f, "Couldn't start the installer ({e})"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for UpdateError {}

impl From<io::Error> for UpdateError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

pub fn parse_releases(json: &str) -> Result<Vec<Release>, UpdateError> {
    serde_json::from_str(json).map_err(|e| UpdateError::BadResponse(e.to_string()))
}

pub fn is_nightly(tag: &str) -> bool {
    tag.contains(NIGHTLY_MARK)
}

/// A plain file name: no directories, no drive, not hidden.
fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':'])
        && !name.ends_with(PARTIAL_SUFFIX)
}

pub fn is_installer_name(name: &str) -> bool {
    name.starts_with("Scratch-Setup-") && name.ends_with(".exe") && is_plain_file_name(name)
}

/// The release's Windows installer (`Scratch-Setup-<version>.exe`).
pub fn installer_asset(release: &Release) -> Option<&Asset> {
    release.assets.iter().find(|a| is_installer_name(&a.name))
}

pub fn sums_asset(release: &Release) -> Option<&Asset> {
    release.assets.iter().find(|a| a.name == SUMS_NAME)
}

/// Assets are fetched only from this repository's release downloads.
pub fn is_release_download_url(url: &str) -> bool {
    url.strip_prefix(DOWNLOAD_PREFIX)
        .is_some_and(|rest| !rest.is_empty() && !rest.contains(['?', '#', '\\']) && !rest.contains(".."))
}

fn published(release: &Release) -> Option<jiff::Timestamp> {
    release.published_at.as_deref()?.parse().ok()
}

/// The release to offer a build tagged [current_tag], if any.
///
/// - Drafts are ignored; nightlies (`-nightly.` in the tag) only count when
///   [include_nightly] is set. Pre-releases always count.
/// - A candidate needs a Windows installer and SHA256SUMS.txt.
/// - Releases are ordered by `published_at`. When the running build's own
///   release is in the list (whatever its channel), only candidates
///   published after it count; otherwise any candidate does.
/// - The newest such candidate is offered.
pub fn pick_update<'a>(
    releases: &'a [Release],
    current_tag: &str,
    include_nightly: bool,
) -> Option<&'a Release> {
    let own = releases
        .iter()
        .filter(|r| !r.draft && r.tag_name == current_tag)
        .find_map(published);
    releases
        .iter()
        .filter(|r| !r.draft && r.tag_name != current_tag)
        .filter(|r| include_nightly || !is_nightly(&r.tag_name))
        .filter(|r| installer_asset(r).is_some() && sums_asset(r).is_some())
        .filter_map(|r| published(r).map(|at| (at, r)))
        .filter(|(at, _)| own.is_none_or(|own| *at > own))
        .max_by_key(|(at, _)| *at)
        .map(|(_, r)| r)
}

/// Parses `sha256sum` output: `<hex>  <name>` or `<hex> *<name>` per line.
/// Returns file name to lower-case hex digest; malformed lines are skipped.
pub fn parse_sha256sums(text: &str) -> HashMap<String, String> {
    let mut sums = HashMap::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        let Some((hash, rest)) = line.split_once(|c: char| c == ' ' || c == '\t') else {
            continue;
        };
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let name = rest.trim_start();
        let name = name.strip_prefix('*').unwrap_or(name);
        if name.is_empty() {
            continue;
        }
        sums.insert(name.to_owned(), hash.to_ascii_lowercase());
    }
    sums
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Ok when [path] hashes to [expected] (hex, any case).
pub fn verify_file(path: &Path, expected: &str) -> Result<(), UpdateError> {
    let actual = sha256_file(path)?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(UpdateError::Mismatch {
            file: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        })
    }
}

pub fn partial_path(dir: &Path, installer_name: &str) -> PathBuf {
    dir.join(format!("{installer_name}{PARTIAL_SUFFIX}"))
}

/// Accepts a finished download: when [actual] matches [expected], the
/// partial file becomes `dir/<installer_name>`; otherwise it is deleted and
/// the result is [UpdateError::Mismatch].
pub fn finish_download(
    dir: &Path,
    installer_name: &str,
    actual: &str,
    expected: &str,
) -> Result<PathBuf, UpdateError> {
    let part = partial_path(dir, installer_name);
    if !actual.eq_ignore_ascii_case(expected) {
        let _ = fs::remove_file(&part);
        return Err(UpdateError::Mismatch { file: installer_name.to_owned() });
    }
    let target = dir.join(installer_name);
    if target.exists() {
        fs::remove_file(&target)?;
    }
    fs::rename(&part, &target)?;
    Ok(target)
}

/// Deletes everything in [dir] except the file named [keep] (best effort):
/// only the latest download is kept.
pub fn prune_updates_dir(dir: &Path, keep: Option<&str>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if keep.is_some_and(|k| entry.file_name() == k) {
            continue;
        }
        let path = entry.path();
        let _ = if path.is_dir() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) };
    }
}

/// VERIFY PUBLISHER: the signature hook.
///
/// Scratch's installers are not code-signed yet, so there is nothing to
/// check and this accepts every installer. The download has already
/// matched the release's SHA256SUMS.txt at this point; that guards against
/// corruption and a swapped CDN file, not against a compromised release.
///
/// When releases are signed, check the Authenticode signature here
/// (WinVerifyTrust, then the signer's subject against the expected
/// publisher) and return [UpdateError::Publisher] when it doesn't match.
/// Every installer launch goes through this function.
pub fn verify_publisher(_installer: &Path) -> Result<(), UpdateError> {
    Ok(())
}

/// The arguments the installer is started with: `/SILENT` shows Inno
/// Setup's progress window but no wizard pages, and the `[Run]` entry for
/// silent installs relaunches Scratch at the end (scratch.iss).
pub const INSTALLER_ARGS: &str = "/SILENT";

/// Re-checks [installer] against [sha256] and the publisher hook, then
/// starts it as a normal, visible process. The caller exits the app next.
#[cfg(windows)]
pub fn launch_installer(installer: &Path, sha256: &str) -> Result<(), UpdateError> {
    if !installer.file_name().and_then(|n| n.to_str()).is_some_and(is_installer_name) {
        return Err(UpdateError::Install("not a Scratch installer".into()));
    }
    verify_file(installer, sha256)?;
    verify_publisher(installer)?;
    shell_execute(installer, INSTALLER_ARGS)
}

#[cfg(windows)]
fn shell_execute(file: &Path, args: &str) -> Result<(), UpdateError> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }
    let verb = wide(OsStr::new("open"));
    let file_w = wide(file.as_os_str());
    let args_w = wide(OsStr::new(args));
    let dir_w = file.parent().map(|d| wide(d.as_os_str()));
    // SAFETY: every pointer is a NUL-terminated UTF-16 buffer that outlives the call.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file_w.as_ptr(),
            args_w.as_ptr(),
            dir_w.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute returns a value greater than 32 on success.
    let code = result as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(UpdateError::Install(format!("ShellExecute error {code}")))
    }
}

/// HTTPS to GitHub, trusting the Windows certificate store (so it works
/// behind TLS-inspecting proxies whose CA is installed there).
#[cfg(windows)]
pub mod net {
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use sha2::{Digest, Sha256};

    use super::*;

    const SUMS_MAX_BYTES: usize = 1 << 20;

    pub fn client(version: &str) -> Result<reqwest::Client, UpdateError> {
        use rustls_platform_verifier::BuilderVerifierExt;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .and_then(|b| b.with_platform_verifier())
            .map_err(|e| UpdateError::Offline(format!("TLS setup: {e}")))?
            .with_no_client_auth();
        reqwest::Client::builder()
            .use_preconfigured_tls(tls)
            .user_agent(format!("Scratch/{version}"))
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| UpdateError::Offline(e.to_string()))
    }

    fn net_err(e: reqwest::Error) -> UpdateError {
        UpdateError::Offline(e.to_string())
    }

    fn status_err(resp: &reqwest::Response) -> UpdateError {
        let status = resp.status().as_u16();
        let exhausted = resp
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.trim() == "0");
        if status == 429 || (status == 403 && exhausted) {
            UpdateError::RateLimited
        } else {
            UpdateError::Http(status)
        }
    }

    pub async fn fetch_releases(client: &reqwest::Client) -> Result<Vec<Release>, UpdateError> {
        let resp = client
            .get(RELEASES_URL)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(status_err(&resp));
        }
        parse_releases(&resp.text().await.map_err(net_err)?)
    }

    async fn fetch_sums(client: &reqwest::Client, url: &str) -> Result<HashMap<String, String>, UpdateError> {
        let resp = client.get(url).timeout(Duration::from_secs(30)).send().await.map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(status_err(&resp));
        }
        let bytes = resp.bytes().await.map_err(net_err)?;
        if bytes.len() > SUMS_MAX_BYTES {
            return Err(UpdateError::BadResponse(format!("{SUMS_NAME} is too large")));
        }
        Ok(parse_sha256sums(&String::from_utf8_lossy(&bytes)))
    }

    /// Downloads [installer_name] into [dir] and verifies it against the
    /// release's SHA256SUMS.txt. An already verified copy is reused. Returns
    /// the installer's path and SHA-256. [progress] gets (received, total).
    pub async fn download_installer(
        client: &reqwest::Client,
        dir: &Path,
        installer_name: &str,
        installer_url: &str,
        sums_url: &str,
        progress: impl Fn(u64, u64),
    ) -> Result<(PathBuf, String), UpdateError> {
        if !is_installer_name(installer_name)
            || !is_release_download_url(installer_url)
            || !is_release_download_url(sums_url)
        {
            return Err(UpdateError::BadResponse("unexpected asset".into()));
        }
        std::fs::create_dir_all(dir)?;
        let sums = fetch_sums(client, sums_url).await?;
        let expected = sums
            .get(installer_name)
            .cloned()
            .ok_or_else(|| UpdateError::MissingChecksum(installer_name.to_owned()))?;

        let target = dir.join(installer_name);
        if target.is_file() && verify_file(&target, &expected).is_ok() {
            prune_updates_dir(dir, Some(installer_name));
            return Ok((target, expected));
        }
        prune_updates_dir(dir, None);

        let mut resp = client.get(installer_url).send().await.map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(status_err(&resp));
        }
        let total = resp.content_length().unwrap_or(0);
        let part = partial_path(dir, installer_name);
        // Blocking writes are fine: this runs on the bridge's worker thread.
        let mut file = std::fs::File::create(&part)?;
        let mut hasher = Sha256::new();
        let mut received = 0u64;
        progress(0, total);
        let copied: Result<(), UpdateError> = async {
            while let Some(chunk) = resp.chunk().await.map_err(net_err)? {
                hasher.update(&chunk);
                file.write_all(&chunk)?;
                received += chunk.len() as u64;
                progress(received, total);
            }
            file.flush()?;
            file.sync_all()?;
            Ok(())
        }
        .await;
        drop(file);
        if let Err(e) = copied {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
        let actual = hex(&hasher.finalize());
        let path = finish_download(dir, installer_name, &actual, &expected)?;
        Ok((path, expected))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"[
      {"tag_name":"v0.1.0-nightly.20261027","name":"Nightly 20261027","draft":false,"prerelease":true,
       "published_at":"2026-10-27T02:41:00Z","html_url":"https://github.com/BubbleBuffer/Scratch/releases/tag/v0.1.0-nightly.20261027",
       "assets":[{"name":"Scratch-Setup-0.1.0-nightly.20261027.exe","browser_download_url":"https://github.com/BubbleBuffer/Scratch/releases/download/v0.1.0-nightly.20261027/Scratch-Setup-0.1.0-nightly.20261027.exe","size":10},
                 {"name":"SHA256SUMS.txt","browser_download_url":"https://github.com/BubbleBuffer/Scratch/releases/download/v0.1.0-nightly.20261027/SHA256SUMS.txt","size":1}]},
      {"tag_name":"v0.1.0-prealpha.12","name":"draft","draft":true,"prerelease":true,"published_at":null,"html_url":"",
       "assets":[{"name":"Scratch-Setup-0.1.0-prealpha.12.exe","browser_download_url":"x","size":1},{"name":"SHA256SUMS.txt","browser_download_url":"x","size":1}]},
      {"tag_name":"v0.1.0-prealpha.11","name":"v0.1.0-prealpha.11","draft":false,"prerelease":true,
       "published_at":"2026-10-25T10:00:00Z","html_url":"https://github.com/BubbleBuffer/Scratch/releases/tag/v0.1.0-prealpha.11",
       "assets":[{"name":"Scratch-Setup-0.1.0-prealpha.11.exe","browser_download_url":"https://github.com/BubbleBuffer/Scratch/releases/download/v0.1.0-prealpha.11/Scratch-Setup-0.1.0-prealpha.11.exe","size":10},
                 {"name":"scratch-0.1.0-prealpha.11-android-arm64.apk","browser_download_url":"https://github.com/BubbleBuffer/Scratch/releases/download/v0.1.0-prealpha.11/scratch-0.1.0-prealpha.11-android-arm64.apk","size":10},
                 {"name":"SHA256SUMS.txt","browser_download_url":"https://github.com/BubbleBuffer/Scratch/releases/download/v0.1.0-prealpha.11/SHA256SUMS.txt","size":1}]},
      {"tag_name":"v0.1.0-nightly.20261024","name":"Nightly 20261024","draft":false,"prerelease":true,
       "published_at":"2026-10-24T02:40:00Z","html_url":"",
       "assets":[{"name":"Scratch-Setup-0.1.0-nightly.20261024.exe","browser_download_url":"https://github.com/BubbleBuffer/Scratch/releases/download/v0.1.0-nightly.20261024/Scratch-Setup-0.1.0-nightly.20261024.exe","size":10},
                 {"name":"SHA256SUMS.txt","browser_download_url":"https://github.com/BubbleBuffer/Scratch/releases/download/v0.1.0-nightly.20261024/SHA256SUMS.txt","size":1}]},
      {"tag_name":"v0.1.0-prealpha.10","name":"v0.1.0-prealpha.10","draft":false,"prerelease":true,
       "published_at":"2026-10-23T22:04:05Z","html_url":"",
       "assets":[{"name":"Scratch-Setup-0.1.0-prealpha.10.exe","browser_download_url":"https://github.com/BubbleBuffer/Scratch/releases/download/v0.1.0-prealpha.10/Scratch-Setup-0.1.0-prealpha.10.exe","size":10},
                 {"name":"SHA256SUMS.txt","browser_download_url":"https://github.com/BubbleBuffer/Scratch/releases/download/v0.1.0-prealpha.10/SHA256SUMS.txt","size":1}]},
      {"tag_name":"v0.1.0-prealpha.9","name":"no installer","draft":false,"prerelease":true,
       "published_at":"2026-10-23T12:00:00Z","html_url":"",
       "assets":[{"name":"SHA256SUMS.txt","browser_download_url":"x","size":1}]}
    ]"#;

    fn releases() -> Vec<Release> {
        parse_releases(FIXTURE).expect("fixture parses")
    }

    fn pick(current: &str, nightly: bool) -> Option<String> {
        let all = releases();
        pick_update(&all, current, nightly).map(|r| r.tag_name.clone())
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("scratch-update-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn update_parses_release_fixture() {
        let all = releases();
        assert_eq!(all.len(), 6);
        assert!(all[1].draft);
        assert_eq!(all[1].published_at, None);
        assert_eq!(
            installer_asset(&all[2]).map(|a| a.name.as_str()),
            Some("Scratch-Setup-0.1.0-prealpha.11.exe")
        );
        assert!(installer_asset(&all[5]).is_none());
        assert!(parse_releases("{not json").is_err());
    }

    #[test]
    fn update_nightly_tags_are_recognised() {
        assert!(is_nightly("v0.1.0-nightly.20261027"));
        assert!(!is_nightly("v0.1.0-prealpha.11"));
        assert!(!is_nightly("v0.2.0"));
    }

    #[test]
    fn update_stable_channel_skips_nightlies_and_drafts() {
        // prealpha.12 is a draft, the nightlies are filtered out.
        assert_eq!(pick("v0.1.0-prealpha.10", false).as_deref(), Some("v0.1.0-prealpha.11"));
    }

    #[test]
    fn update_nightly_channel_takes_newest_by_published_at() {
        assert_eq!(pick("v0.1.0-prealpha.10", true).as_deref(), Some("v0.1.0-nightly.20261027"));
    }

    #[test]
    fn update_newest_build_is_up_to_date() {
        assert_eq!(pick("v0.1.0-prealpha.11", false), None);
        assert_eq!(pick("v0.1.0-nightly.20261027", true), None);
    }

    #[test]
    fn update_only_releases_published_after_the_running_one() {
        // On a nightly that is newer than every stable release, with the
        // nightly channel turned off: nothing newer to offer.
        assert_eq!(pick("v0.1.0-nightly.20261027", false), None);
        // On an older nightly, the newer stable pre-release is offered.
        assert_eq!(pick("v0.1.0-nightly.20261024", false).as_deref(), Some("v0.1.0-prealpha.11"));
        assert_eq!(pick("v0.1.0-nightly.20261024", true).as_deref(), Some("v0.1.0-nightly.20261027"));
    }

    #[test]
    fn update_unlisted_tag_gets_newest_candidate() {
        assert_eq!(pick("v0.0.1-prealpha.1", false).as_deref(), Some("v0.1.0-prealpha.11"));
        assert_eq!(pick("v0.0.1-prealpha.1", true).as_deref(), Some("v0.1.0-nightly.20261027"));
    }

    #[test]
    fn update_draft_own_release_does_not_anchor() {
        // The running tag only exists as a draft: treated as unlisted.
        assert_eq!(pick("v0.1.0-prealpha.12", false).as_deref(), Some("v0.1.0-prealpha.11"));
    }

    #[test]
    fn update_candidates_need_installer_and_sums() {
        let mut all = releases();
        all.retain(|r| r.tag_name == "v0.1.0-prealpha.9");
        assert!(pick_update(&all, "v0.0.1", true).is_none());
    }

    #[test]
    fn update_installer_names_and_urls_are_checked() {
        assert!(is_installer_name("Scratch-Setup-0.1.0-prealpha.11.exe"));
        assert!(!is_installer_name("notes-cli-0.1.0-windows-x64.exe"));
        assert!(!is_installer_name("Scratch-Setup-..\\evil.exe"));
        assert!(!is_installer_name("Scratch-Setup-a/b.exe"));
        assert!(is_release_download_url(
            "https://github.com/BubbleBuffer/Scratch/releases/download/v1/Scratch-Setup-1.exe"
        ));
        assert!(!is_release_download_url("https://example.com/Scratch-Setup-1.exe"));
        assert!(!is_release_download_url(
            "https://github.com/BubbleBuffer/Scratch/releases/download/../../other/x.exe"
        ));
    }

    #[test]
    fn update_parses_sha256sums() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let text = format!(
            "{a}  Scratch-Setup-0.1.0-prealpha.11.exe\r\n{b} *scratch-0.1.0-prealpha.11-android-arm64.apk\nnot a line\n{}  short.txt\n",
            "c".repeat(10)
        );
        let sums = parse_sha256sums(&text);
        assert_eq!(sums.len(), 2);
        assert_eq!(sums["Scratch-Setup-0.1.0-prealpha.11.exe"], a);
        assert_eq!(sums["scratch-0.1.0-prealpha.11-android-arm64.apk"], "b".repeat(64));
    }

    #[test]
    fn update_verifies_file_hash() {
        let dir = temp_dir("verify");
        let file = dir.join("Scratch-Setup-1.exe");
        fs::write(&file, b"abc").unwrap();
        // SHA-256 of "abc".
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_file(&file).unwrap(), abc);
        assert!(verify_file(&file, &abc.to_uppercase()).is_ok());
        assert!(matches!(verify_file(&file, &"0".repeat(64)), Err(UpdateError::Mismatch { .. })));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_mismatch_deletes_the_download() {
        let dir = temp_dir("mismatch");
        let name = "Scratch-Setup-0.1.0-prealpha.11.exe";
        let part = partial_path(&dir, name);
        fs::write(&part, b"tampered").unwrap();
        let actual = sha256_file(&part).unwrap();
        let err = finish_download(&dir, name, &actual, &"0".repeat(64)).unwrap_err();
        assert!(matches!(err, UpdateError::Mismatch { .. }));
        assert!(!part.exists());
        assert!(!dir.join(name).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_verified_download_is_kept_alone() {
        let dir = temp_dir("keep");
        fs::write(dir.join("Scratch-Setup-0.1.0-prealpha.10.exe"), b"old").unwrap();
        let name = "Scratch-Setup-0.1.0-prealpha.11.exe";
        let part = partial_path(&dir, name);
        fs::write(&part, b"new").unwrap();
        let actual = sha256_file(&part).unwrap();
        let path = finish_download(&dir, name, &actual, &actual).unwrap();
        assert_eq!(path, dir.join(name));
        prune_updates_dir(&dir, Some(name));
        let left: Vec<_> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from(name)]);
        assert!(verify_file(&path, &actual).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_publisher_hook_accepts_unsigned_today() {
        assert!(verify_publisher(Path::new("Scratch-Setup-1.exe")).is_ok());
    }
}
