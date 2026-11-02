//! `scratch-cli packages list|install <name>|remove <name>`: the packages
//! folder the app uses (`<data dir>/packages`), through `scratch_packages`.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use scratch_packages::{current_platform, Manifest, Package, Progress, State, Store};
use serde_json::json;

use crate::Flags;

/// Runs a `packages` subcommand against this build's embedded manifest.
pub(crate) fn run(words: &[&str], flags: &Flags, out: &mut dyn Write) -> Result<()> {
    let dir: PathBuf = crate::remote::data_dir(flags)?;
    run_with(words, flags.json, out, &Manifest::embedded(), &Store::new(&dir))
}

/// The same with an explicit manifest and store. Tests only.
#[doc(hidden)]
pub fn run_with(
    words: &[&str],
    json: bool,
    out: &mut dyn Write,
    manifest: &Manifest,
    store: &Store,
) -> Result<()> {
    let platform = current_platform();
    match words {
        ["list"] => list(json, out, manifest, store, platform),
        ["install", name] => install(name, out, manifest, store, platform),
        ["remove", name] => {
            if store.remove(name)? {
                writeln!(out, "removed {name}")?;
            } else {
                writeln!(out, "{name} is not installed")?;
            }
            Ok(())
        }
        _ => bail!("usage: scratch-cli packages list [--json] | install <name> | remove <name>"),
    }
}

fn state_word(state: &State) -> &'static str {
    match state {
        State::NotInstalled => "not installed",
        State::Current(_) => "installed",
        State::UpdateAvailable(_) => "update available",
    }
}

fn list(json: bool, out: &mut dyn Write, manifest: &Manifest, store: &Store, platform: &str) -> Result<()> {
    let available: Vec<&Package> = manifest.for_platform(platform).collect();
    let installed = store.installed();
    if json {
        let mut rows: Vec<_> = available
            .iter()
            .map(|p| {
                let state = store.state(p);
                let current = store.get(&p.name);
                json!({
                    "name": p.name,
                    "version": p.version,
                    "sizeBytes": p.size,
                    "state": state_word(&state),
                    "installedVersion": current.map(|i| i.version),
                })
            })
            .collect();
        for i in installed.iter().filter(|i| !available.iter().any(|p| p.name == i.name)) {
            rows.push(json!({
                "name": i.name, "version": null, "sizeBytes": null,
                "state": "installed", "installedVersion": i.version,
            }));
        }
        writeln!(out, "{}", json!({ "platform": platform, "packages": rows }))?;
        return Ok(());
    }
    if available.is_empty() && installed.is_empty() {
        writeln!(out, "no packages are available on this platform ({platform})")?;
        return Ok(());
    }
    for p in &available {
        let state = store.state(p);
        let installed_version = match &state {
            State::Current(i) | State::UpdateAvailable(i) => format!(" (installed {})", i.version),
            State::NotInstalled => String::new(),
        };
        writeln!(
            out,
            "{} {} {} MB: {}{}",
            p.name,
            p.version,
            p.size.div_ceil(1 << 20),
            state_word(&state),
            installed_version
        )?;
    }
    for i in installed.iter().filter(|i| !available.iter().any(|p| p.name == i.name)) {
        writeln!(out, "{} {}: installed", i.name, i.version)?;
    }
    Ok(())
}

fn install(name: &str, out: &mut dyn Write, manifest: &Manifest, store: &Store, platform: &str) -> Result<()> {
    let package = manifest
        .find(name, platform)
        .ok_or_else(|| anyhow!("{name} is not available on this platform ({platform})"))?;
    let mut last_decile = u64::MAX;
    let mut write_err = None;
    let installed = store.install(package, &mut |p| {
        let line = match p {
            Progress::Downloading { done, total } => {
                let decile = if total == 0 { 10 } else { done * 10 / total };
                if decile == last_decile {
                    return;
                }
                last_decile = decile;
                format!("downloading {name}: {}%", decile * 10)
            }
            Progress::Verifying => format!("verifying {name}"),
            Progress::Extracting => format!("extracting {name}"),
            Progress::Installing => format!("installing {name}"),
        };
        if let Err(e) = writeln!(out, "{line}") {
            write_err = Some(e);
        }
    })?;
    if let Some(e) = write_err {
        return Err(e.into());
    }
    writeln!(out, "installed {} {} at {}", installed.name, installed.version, installed.path.display())?;
    Ok(())
}
