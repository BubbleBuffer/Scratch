//! Windows updates for the Flutter app (crate::updater does the work).
//! The app decides when to check; nothing here runs on its own, and the
//! installer only starts from [update_install], on the user's click.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use flutter_rust_bridge::frb;

/// A release the running build can update to.
#[derive(Debug, Clone)]
pub struct UpdateRelease {
    pub tag: String,
    pub name: String,
    pub published_at: String,
    /// The release page on GitHub (release notes).
    pub notes_url: String,
    pub nightly: bool,
    pub installer_name: String,
    pub installer_url: String,
    pub installer_size: i64,
    pub sums_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateCheckStatus {
    UpToDate,
    Available,
    /// No connection: the app retries at the next scheduled check.
    Offline,
    /// GitHub's unauthenticated rate limit: also retried later.
    RateLimited,
    Failed,
    /// Not Windows, or no release tag to compare with.
    Unsupported,
}

#[derive(Debug, Clone)]
pub struct UpdateCheck {
    pub status: UpdateCheckStatus,
    pub release: Option<UpdateRelease>,
    pub message: Option<String>,
}

/// A verified installer on disk.
#[derive(Debug, Clone)]
pub struct DownloadedUpdate {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy)]
pub struct UpdateProgress {
    pub received: i64,
    /// 0 when unknown.
    pub total: i64,
    pub active: bool,
}

static RECEIVED: AtomicU64 = AtomicU64::new(0);
static TOTAL: AtomicU64 = AtomicU64::new(0);
static DOWNLOADING: AtomicBool = AtomicBool::new(false);

#[cfg_attr(not(windows), allow(dead_code))]
/// The version part of a release tag (`v0.1.0-prealpha.11` → `0.1.0-prealpha.11`).
fn version_of(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

#[cfg(windows)]
fn block_on<T>(f: impl std::future::Future<Output = T>) -> Result<T, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    Ok(rt.block_on(f))
}

/// Lists the releases on GitHub and picks the one to offer a build tagged
/// [current_tag] (rules: crate::updater::pick_update). When nothing is
/// offered, earlier downloads in [updates_dir] are deleted.
pub fn update_check(
    current_tag: String,
    include_nightly: bool,
    updates_dir: String,
) -> UpdateCheck {
    let failed = |status, message: String| UpdateCheck {
        status,
        release: None,
        message: Some(message),
    };
    if current_tag.is_empty() {
        return failed(UpdateCheckStatus::Unsupported, "Development build".into());
    }
    #[cfg(not(windows))]
    {
        let _ = (include_nightly, updates_dir);
        failed(
            UpdateCheckStatus::Unsupported,
            "Updates are only available on Windows".into(),
        )
    }
    #[cfg(windows)]
    {
        use crate::updater::{self, net, UpdateError};
        let result = block_on(async {
            let client = net::client(version_of(&current_tag))?;
            net::fetch_releases(&client).await
        });
        let releases = match result {
            Ok(Ok(r)) => r,
            Ok(Err(UpdateError::Offline(e))) => return failed(UpdateCheckStatus::Offline, e),
            Ok(Err(UpdateError::RateLimited)) => {
                return failed(
                    UpdateCheckStatus::RateLimited,
                    UpdateError::RateLimited.to_string(),
                )
            }
            Ok(Err(e)) => return failed(UpdateCheckStatus::Failed, e.to_string()),
            Err(e) => return failed(UpdateCheckStatus::Failed, e),
        };
        let dir = std::path::Path::new(&updates_dir);
        let Some(r) = updater::pick_update(&releases, &current_tag, include_nightly) else {
            updater::prune_updates_dir(dir, None);
            return UpdateCheck {
                status: UpdateCheckStatus::UpToDate,
                release: None,
                message: None,
            };
        };
        // pick_update only returns releases with both assets.
        let installer = updater::installer_asset(r).expect("candidate has an installer");
        let sums = updater::sums_asset(r).expect("candidate has SHA256SUMS.txt");
        UpdateCheck {
            status: UpdateCheckStatus::Available,
            release: Some(UpdateRelease {
                tag: r.tag_name.clone(),
                name: r
                    .name
                    .clone()
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| r.tag_name.clone()),
                published_at: r.published_at.clone().unwrap_or_default(),
                notes_url: r.html_url.clone(),
                nightly: updater::is_nightly(&r.tag_name),
                installer_name: installer.name.clone(),
                installer_url: installer.browser_download_url.clone(),
                installer_size: installer.size as i64,
                sums_url: sums.browser_download_url.clone(),
            }),
            message: None,
        }
    }
}

/// Downloads [release]'s installer into [updates_dir] and verifies it
/// against the release's SHA256SUMS.txt (a mismatching file is deleted).
/// Only the latest download is kept. Progress: [update_download_progress].
pub fn update_download(
    release: UpdateRelease,
    current_tag: String,
    updates_dir: String,
) -> Result<DownloadedUpdate, String> {
    #[cfg(not(windows))]
    {
        let _ = (release, current_tag, updates_dir);
        Err("Updates are only available on Windows".into())
    }
    #[cfg(windows)]
    {
        use crate::updater::net;
        if DOWNLOADING.swap(true, Ordering::SeqCst) {
            return Err("A download is already running".into());
        }
        RECEIVED.store(0, Ordering::SeqCst);
        TOTAL.store(release.installer_size.max(0) as u64, Ordering::SeqCst);
        let result = block_on(async {
            let client = net::client(version_of(&current_tag))?;
            net::download_installer(
                &client,
                std::path::Path::new(&updates_dir),
                &release.installer_name,
                &release.installer_url,
                &release.sums_url,
                |received, total| {
                    RECEIVED.store(received, Ordering::SeqCst);
                    if total > 0 {
                        TOTAL.store(total, Ordering::SeqCst);
                    }
                },
            )
            .await
        });
        DOWNLOADING.store(false, Ordering::SeqCst);
        match result {
            Ok(Ok((path, sha256))) => Ok(DownloadedUpdate {
                path: path.to_string_lossy().into_owned(),
                sha256,
            }),
            Ok(Err(e)) => Err(e.to_string()),
            Err(e) => Err(e),
        }
    }
}

#[frb(sync)]
pub fn update_download_progress() -> UpdateProgress {
    UpdateProgress {
        received: RECEIVED.load(Ordering::SeqCst) as i64,
        total: TOTAL.load(Ordering::SeqCst) as i64,
        active: DOWNLOADING.load(Ordering::SeqCst),
    }
}

/// Starts the downloaded installer (after re-checking its SHA-256 and the
/// publisher hook) as a normal, visible process. The app exits right after,
/// so the installer finds it closed. Only called on the user's click.
pub fn update_install(path: String, sha256: String) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        let _ = (path, sha256);
        Err("Updates are only available on Windows".into())
    }
    #[cfg(windows)]
    {
        crate::updater::launch_installer(std::path::Path::new(&path), &sha256)
            .map_err(|e| e.to_string())
    }
}
