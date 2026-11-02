//! On-demand runtime packages for the Flutter app (crates/packages does the
//! work). Nothing here runs on its own: a download starts only from
//! [package_install], on the user's click. See wiki/addons/packages.md.

use std::path::Path;

use flutter_rust_bridge::frb;
use scratch_packages::{current_platform, Manifest, Progress, State, Store};

use crate::frb_generated::StreamSink;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageState {
    /// This build lists no such package for this platform.
    NotAvailable,
    NotInstalled,
    Installed,
    /// Something is installed and this build pins another version or build.
    UpdateAvailable,
}

#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub name: String,
    /// The version this build pins; empty when [PackageState::NotAvailable].
    pub version: String,
    /// Size of the download in bytes; 0 when not available.
    pub download_bytes: i64,
    pub state: PackageState,
    pub installed_version: Option<String>,
    /// Size on disk of the installed version.
    pub installed_bytes: Option<i64>,
}

/// What an install is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageProgress {
    Downloading { done: i64, total: i64 },
    Verifying,
    Extracting,
    Installing,
}

fn store(data_dir: &str) -> Store {
    Store::new(Path::new(data_dir))
}

fn size(bytes: u64) -> i64 {
    i64::try_from(bytes).unwrap_or(i64::MAX)
}

fn info_for(manifest: &Manifest, store: &Store, platform: &str, name: &str) -> PackageInfo {
    let installed = store.get(name);
    let mut info = PackageInfo {
        name: name.to_string(),
        version: String::new(),
        download_bytes: 0,
        state: PackageState::NotAvailable,
        installed_version: installed.as_ref().map(|i| i.version.clone()),
        installed_bytes: installed.as_ref().map(|i| size(i.size_bytes)),
    };
    match manifest.find(name, platform) {
        Some(package) => {
            info.version = package.version.clone();
            info.download_bytes = size(package.size);
            info.state = match store.state(package) {
                State::NotInstalled => PackageState::NotInstalled,
                State::Current(_) => PackageState::Installed,
                State::UpdateAvailable(_) => PackageState::UpdateAvailable,
            };
        }
        // Installed earlier, but this build no longer lists it: still
        // installed, and removable.
        None if installed.is_some() => info.state = PackageState::Installed,
        None => {}
    }
    info
}

fn list_for(manifest: &Manifest, store: &Store, platform: &str) -> Vec<PackageInfo> {
    let mut names: Vec<String> = manifest
        .for_platform(platform)
        .map(|p| p.name.clone())
        .collect();
    for installed in store.installed() {
        if !names.contains(&installed.name) {
            names.push(installed.name);
        }
    }
    names.sort();
    names
        .iter()
        .map(|name| info_for(manifest, store, platform, name))
        .collect()
}

/// Every package this build lists for this platform, plus any other installed
/// package, by name.
#[frb(sync)]
pub fn packages_list(data_dir: String) -> Vec<PackageInfo> {
    list_for(&Manifest::embedded(), &store(&data_dir), current_platform())
}

/// One package's state; [PackageState::NotAvailable] when this build lists no
/// such package for this platform and none is installed.
#[frb(sync)]
pub fn package_info(data_dir: String, name: String) -> PackageInfo {
    info_for(&Manifest::embedded(), &store(&data_dir), current_platform(), &name)
}

/// The platform key of this build (`windows-x64`, `linux-x64`, ...).
#[frb(sync)]
pub fn packages_platform() -> String {
    current_platform().to_string()
}

/// Downloads, verifies and installs the build's pinned version of `name`,
/// reporting progress on `sink`. Runs on a worker thread until done. The
/// error is a message that names the state (offline, checksum, disk full, ...).
pub fn package_install(
    data_dir: String,
    name: String,
    sink: StreamSink<PackageProgress>,
) -> Result<PackageInfo, String> {
    let manifest = Manifest::embedded();
    let platform = current_platform();
    let store = store(&data_dir);
    let package = manifest.find(&name, platform).ok_or_else(|| {
        scratch_packages::Error::Unavailable {
            name: name.clone(),
            platform: platform.to_string(),
        }
        .to_string()
    })?;
    store
        .install(package, &mut |progress| {
            let _ = sink.add(match progress {
                Progress::Downloading { done, total } => PackageProgress::Downloading {
                    done: size(done),
                    total: size(total),
                },
                Progress::Verifying => PackageProgress::Verifying,
                Progress::Extracting => PackageProgress::Extracting,
                Progress::Installing => PackageProgress::Installing,
            });
        })
        .map_err(|e| e.to_string())?;
    Ok(info_for(&manifest, &store, platform, &name))
}

/// Removes every installed version of `name`.
pub fn package_remove(data_dir: String, name: String) -> Result<(), String> {
    store(&data_dir)
        .remove(&name)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scratch_packages::Package;

    fn package(version: &str, sha: char) -> Package {
        Package {
            name: "claude-code".into(),
            version: version.into(),
            platform: "windows-x64".into(),
            url: "https://example.invalid/p.zip".into(),
            sha256: sha.to_string().repeat(64),
            size: 1000,
        }
    }

    fn install_marker(dir: &Path, version: &str, sha: char) {
        let folder = dir.join("packages/claude-code").join(version);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("data.bin"), [0u8; 10]).unwrap();
        let marker = format!(
            "{{\"name\":\"claude-code\",\"version\":\"{version}\",\"sha256\":\"{}\"}}",
            sha.to_string().repeat(64)
        );
        std::fs::write(folder.join(".package.json"), marker).unwrap();
    }

    #[test]
    fn states_follow_the_manifest_and_the_install() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let manifest = Manifest {
            packages: vec![package("2.0.0", 'b')],
        };

        // Nothing is listed for another platform.
        let other = info_for(&manifest, &store, "linux-x64", "claude-code");
        assert_eq!(other.state, PackageState::NotAvailable);
        assert!(list_for(&manifest, &store, "linux-x64").is_empty());

        let info = info_for(&manifest, &store, "windows-x64", "claude-code");
        assert_eq!(info.state, PackageState::NotInstalled);
        assert_eq!((info.version.as_str(), info.download_bytes), ("2.0.0", 1000));
        assert_eq!(info.installed_version, None);

        install_marker(dir.path(), "1.0.0", 'a');
        let info = info_for(&manifest, &store, "windows-x64", "claude-code");
        assert_eq!(info.state, PackageState::UpdateAvailable);
        assert_eq!(info.installed_version.as_deref(), Some("1.0.0"));
        assert!(info.installed_bytes.unwrap() >= 10);

        std::fs::remove_dir_all(dir.path().join("packages/claude-code/1.0.0")).unwrap();
        install_marker(dir.path(), "2.0.0", 'b');
        let info = info_for(&manifest, &store, "windows-x64", "claude-code");
        assert_eq!(info.state, PackageState::Installed);
        assert_eq!(list_for(&manifest, &store, "windows-x64").len(), 1);
    }

    #[test]
    fn an_installed_package_the_build_no_longer_lists_stays_listed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        install_marker(dir.path(), "1.0.0", 'a');
        let all = list_for(&Manifest::default(), &store, "windows-x64");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].state, PackageState::Installed);
        assert_eq!(all[0].version, "");
    }

    #[test]
    fn remove_deletes_the_install() {
        let dir = tempfile::tempdir().unwrap();
        install_marker(dir.path(), "1.0.0", 'a');
        let data = dir.path().to_string_lossy().into_owned();
        package_remove(data.clone(), "claude-code".into()).unwrap();
        assert!(Store::new(dir.path()).installed().is_empty());
        assert!(package_remove(data, "../x".into()).is_err());
    }
}
