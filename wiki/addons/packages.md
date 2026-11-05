---
type: topic
layer: implementation
summary: "On-demand runtime packages: manifest pinned in the build, verified download, atomic install into the data folder, shared by the app and scratch-cli."
parent: "[[addons/index]]"
status: current
---

# Runtime packages

An add-on that needs external dependencies (today the Claude Code bundle: Agent SDK, win32-x64 native package, the BubblePaw claude-code extension and Bun) loads them on demand instead of shipping them in the installer (owner decision 2026-11-02). The core is the `scratch_packages` crate ([crates/packages](../../crates/packages/src/lib.rs)), used by the Flutter bridge and by `scratch-cli`.

## Package and manifest

A package is one zip per platform: `scratch-pkg-<name>-<version>-<platform>.zip`, attached to the GitHub release. The manifest lists packages keyed by name and platform: `name`, `version`, `platform`, `url` (HTTPS), `sha256` (lowercase hex of the zip) and `size`. `version` is the content version and names the install folder. The build embeds the manifest from `crates/packages/manifest.json` (the checked-in file lists no packages; CI writes the real one before the app build), so the checksum is pinned in the binary. Platform keys: `windows-x64`, `linux-x64`, `android-arm64`; a package with no entry for the running platform is "not available on this platform".

## Install

`Store::install` works in `<data dir>/packages/<name>/`:

1. Takes an exclusive file lock (`.lock`) on the package folder. The app and `scratch-cli` share the folder; a second installer waits, then finds the package installed and returns without downloading.
2. Sweeps leftovers (`.tmp-download`, `.tmp-extract`, `.tmp-old`) of an interrupted install, so a retry starts clean.
3. Downloads over HTTPS (only; `http://` is accepted in tests) to `.tmp-download`, counting bytes: a body longer or shorter than `size` fails with a size error, a dropped connection with an offline error, a wrong sha256 with a checksum error.
4. Extracts to `.tmp-extract`, refusing paths outside it, symlinks and archives over 4 GiB unpacked, and writes a `.package.json` marker (name, version, sha256).
5. Renames the folder to `<version>/`, then deletes other versions (best effort: a locked folder on Windows stays until the next install). Until the rename, the old version keeps working.

`Store::state` compares the installed marker with the manifest entry: not installed, current, or update available (version or hash differs). `installed()` lists current versions, `remove(name)` deletes every version. Errors are an enum (`Offline`, `Http`, `SizeMismatch`, `ChecksumMismatch`, `BadArchive`, `DiskFull`, `Unavailable`, ...) with messages that name the state.

## Build and lookup

The `package` job in the release, nightly and windows-build workflows builds the zip and manifest ([Windows installer](../releases/windows-install.md#claude-code-package)); the Windows build embeds the manifest. `vault_agent::claude` finds Claude Code from overrides, then `PATH` and `~/.bun/bin` for Bun, then the installed package, then the legacy bundle.

## scratch-cli

`scratch-cli packages list [--json]`, `install <name>` (progress lines in 10% steps) and `remove <name>`, with `--data-dir` as for the `app` commands. See [scratch-cli](../integrations/scratch-cli.md).

Tests: `crates/packages/tests/install.rs` (local server: good, bad hash, short body, longer body, interrupted, offline, concurrent, update, same-version rebuild, HTTPS only, path traversal, remove) and `crates/cli/tests/packages.rs`.

## App UI

The Flutter side is the agent add-on's download card, the "Installed packages" settings section and `AgentStore.refreshRuntime()` (see [Agent](agent.md#claude-code-download-runtime-package)). The bridge exposes `packagesList`, `packageInfo`, `packagesPlatform`, `packageInstall` (a stream of `PackageProgress`: phase and bytes) and `packageRemove` ([packages.rs](../../app/rust/src/api/packages.rs)). Installing is always an explicit tap; a failed install shows the Rust error message and Retry, and does not touch the running agent.
