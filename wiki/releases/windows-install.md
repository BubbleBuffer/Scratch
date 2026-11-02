---
type: page
summary: "Per-user Inno Setup installer built in the windows-build workflow: where Scratch installs, shortcuts, upgrade, uninstall, how the app updates itself and how to build it."
parent: "[[releases/index]]"
status: current
---

# Windows installer

Built 2026-10-21 on `lane/win-installer`. The script is [app/windows/installer/scratch.iss](../../app/windows/installer/scratch.iss).

## What it does

- **Per-user, no admin.** `PrivilegesRequired=lowest`. It installs the `flutter build windows` Release folder to `%LOCALAPPDATA%\Programs\Scratch`. The exe stays `notes.exe`.
- **Shortcuts.** A Start menu shortcut named Scratch with the app icon, so Windows Search finds it. A desktop shortcut is an option on the tasks page, unchecked by default.
- **Apps & features.** An uninstall entry under HKCU: name Scratch, the displayed version (`VersionText`, see below), publisher BubbleBuffer, and the exe's icon.
- **Upgrade in place.** The AppId GUID is fixed (`1B472506-0E7C-46CD-B530-F256FB709BB5`), so a newer installer replaces the same install. Never change it.
- **Running app.** If Scratch is open, the installer closes it through the Restart Manager (`CloseApplications=yes`) before replacing files. The Restart Manager does not restart it (`RestartApplications=no`). An interactive install offers "Launch Scratch" on the finish page. A silent one (`/SILENT`, used by the updater) starts Scratch itself at the end, through the second `[Run]` entry (`skipifnotsilent`).
- **Uninstall.** Removes the program files (a legacy Claude Code bundle included), the shortcuts and `HKCU\Software\Classes\notes` (the `notes://` scheme). The installer writes that key at install time, pointing at the installed `notes.exe`, so links work before the first start; the app also rewrites it on every start ([app state, page links](../app/app-state.md#page-links)). User data in `%LOCALAPPDATA%\Notes` is never touched, downloaded packages (`%LOCALAPPDATA%Notespackages`) included.
- **Updates.** The app finds new releases on GitHub and installs them with this installer on the user's click. See [Updates](#updates).
- **Not done.** Unsigned (SmartScreen warns once on first run). No autostart or file associations.

## Claude Code package

Claude Code is not in the installer. It is a runtime package that the app and
`scratch-cli` download on demand ([Runtime packages](../addons/packages.md);
owner decision 2026-11-02). The package, built per release:

- `scratch-pkg-claude-code-<version>-windows-x64.zip`, attached to the same
  GitHub release. `<version>` is the content version, `<claude-agent-sdk
  version>-<first 7 of the BubblePaw-Core commit>`.
- `claude-code\`: the BubblePaw `claude-code` extension (`package.json`,
  `bun.lock`, `NOTICE`, `src\`, `node_modules\` with its production
  dependencies, Claude Code itself among them: `@anthropic-ai/claude-agent-sdk`
  and its `-win32-x64` native package).
- `bun\bun.exe`: the Bun that runs the extension (1.4.2, the hub's version).

[app/windows/installer/build-package-claude-code.ps1](../../app/windows/installer/build-package-claude-code.ps1)
builds it and writes `manifest.json` (name, version, platform, URL, sha256,
size) next to the zip. It copies the extension's files from BubblePaw-Core at
the commit `Cargo.lock` pins and runs `bun install --production
--frozen-lockfile` in it, the same files and command as the hub's image
([deploy/hub/Dockerfile](../../deploy/hub/Dockerfile)). Entries are sorted and
carry fixed timestamps; the script fails below its own checks (required files,
fewer than 65000 files, because the app refuses zip64).

CI: a `package` job runs before the Windows app build in
[windows-build.yml](../../.github/workflows/windows-build.yml),
[release.yml](../../.github/workflows/release.yml) and
[nightly.yml](../../.github/workflows/nightly.yml). It uploads the manifest
(artifact `package-manifest`) and the zip. The Windows build downloads the
manifest and copies it to `crates/packages/manifest.json` before building the
app and the CLIs, so the zip's sha256 is pinned in the binaries. The release
and nightly `publish` jobs attach the zip to the release; its URL in the
manifest is `<release download URL>/<tag>/<zip name>`. windows-build.yml runs
are not releases: their manifest URL names a release that does not exist, and
the zip is uploaded as an artifact only on a manual run.

`scripts/build-windows-installer.sh` no longer bundles anything, so the
Release folder, and the installer, hold only the app. Measured on the windows-build run of 2026-11-02: the installer is 26 MB (it was 143 MB with the bundle); the package zip is 168 MB (about 360 MB unpacked). Existing
installs keep a legacy `claude-code\` and `bun\` in the app folder until the
update replaces the folder; the lookup still finds them.

The app finds Claude Code without a setting: `crates/agent/src/claude.rs`
takes the extension and Bun from, in order, an explicit choice (an override,
`NOTES_CLAUDE_CODE_DIR`, the app's saved paths in Settings > Agent >
Advanced; for Bun also `PATH` and `~/.bun/bin`), then the installed package
(`<data dir>/packages/claude-code/<version>/`), then the legacy bundle in the
running executable's folder (and, for `scratch-cli`, the installed app folder,
`%LOCALAPPDATA%\Programs\Scratch`), then the extension location compiled into
the binary. The lookup order is also in
[scratch-cli](../integrations/scratch-cli.md#credentials).

## Updates

Added 2026-10-26. Windows only. The code is [app/rust/src/updater.rs](../../app/rust/src/updater.rs) (the work), [app/rust/src/api/update.rs](../../app/rust/src/api/update.rs) (the bridge) and [app/lib/features/updates/](../../app/lib/features/updates) (schedule, state and UI).

**Build identity.** Release and nightly builds are built with `--dart-define=SCRATCH_RELEASE_TAG=<tag>` ([scripts/build-windows-installer.sh](../../scripts/build-windows-installer.sh)). A build without it (`windows-build` artifacts, local builds) is a development build: Settings > About (Updates section) says so and nothing is checked. For testing, the environment variable `SCRATCH_UPDATE_TAG=<tag>` makes any build behave as that tag.

**Checking.**

- Rust calls `GET https://api.github.com/repos/BubbleBuffer/Scratch/releases?per_page=30`, unauthenticated, with `User-Agent: Scratch/<version>`.
- The client is reqwest on rustls with `rustls-platform-verifier`, so certificates are checked against the Windows certificate store. Behind a TLS-inspecting corporate proxy, whose CA is installed in Windows, it works like a browser does.
- The app checks 30 s after start, then every 6 hours, and on "Check now". Turning "Include nightly builds" on or off also checks.
- Offline and rate-limited checks are quiet: the status keeps what it knew, and Settings shows the reason under it. Nothing pops up.

**Which release.** `pick_update` decides:

- Drafts never count.
- "Include nightly builds" (off by default) decides whether tags containing `-nightly.` count. Pre-alphas always count.
- A candidate must have a `Scratch-Setup-*.exe` and `SHA256SUMS.txt`.
- Releases are ordered by `published_at`. If the running tag is in the list, only candidates published after it count. If it isn't (pruned nightly, or older than the 30 listed), any candidate does.
- The newest is offered.

**Downloading.**

- The installer goes to `%LOCALAPPDATA%\Notes\updates\` (the app's data folder, or `NOTES_DATA_DIR` if set), never `%TEMP%`.
- It downloads to `<name>.partial`. Its SHA-256 is compared with the release's `SHA256SUMS.txt`. A mismatch deletes the file and shows an error; only a matching file is renamed to `Scratch-Setup-<version>.exe`.
- Only the latest download is kept. A check that finds nothing to offer empties the folder.
- "Download updates automatically" (on by default) downloads in the background. Off, Settings shows a Download button. Nothing installs by itself.
- Assets are fetched only from `https://github.com/BubbleBuffer/Scratch/releases/download/`, and file names must be plain `Scratch-Setup-*.exe` names.

**Installing.** Only on a click on "Restart to update", in Settings > About, Updates section, or in the panel behind the sidebar's "Update ready" row:

1. Rust checks the file's SHA-256 again, then calls `verify_publisher`.
2. It starts the installer with `ShellExecuteW` ("open", `/SILENT`, normal window). Inno Setup shows its progress window without wizard pages.
3. The app exits the normal way (`exitApplication`, which flushes the vault), so `CloseApplications` finds it gone.
4. The installer replaces the files and starts Scratch again (the silent-only `[Run]` entry).

**Why it is built this way.** Builds are unsigned, so the update has to look to antivirus exactly like a person downloading and running an installer:

- one HTTPS download into the user's app data;
- a visible installer window started through the shell;
- the app closes itself.

There is no helper exe, PowerShell or cmd script, scheduled task, service, Run key, `/VERYSILENT`, hidden window or self-replacing file. The proxy-friendly TLS serves the same aim: corporate networks see an ordinary HTTPS client.

**Signature hook.** `verify_publisher` in `updater.rs` is the one place to require a signature. It accepts every installer today, because there is nothing to verify. When releases are signed, it should check the Authenticode signature (WinVerifyTrust) and the signer's subject, and refuse a mismatch with `UpdateError::Publisher`. Every launch goes through it. The SHA256SUMS check guards against corruption and a swapped download, not against a compromised release.

**Not verified yet.** The download and the install handoff have not been run against a real release. Unit and widget tests cover only the release choice, checksum handling and the settings states. The silent relaunch `[Run]` entry is untested.

## Building it

CI builds it in the `app` job of [.github/workflows/windows-build.yml](../../.github/workflows/windows-build.yml), right after `flutter build windows --release`. Inno Setup 6 ships on the windows-latest image; the step installs it with choco if it is missing. The version is the `version:` in `app/pubspec.yaml` without the build number. The artifact is `scratch-setup-<sha7>`, holding `Scratch-Setup-<version>-<sha7>.exe`, kept 7 days beside `notes-windows-<sha7>` (the plain folder).

`windows-build` runs on pushes to `main`, on pull requests and on demand: `gh workflow run windows-build.yml --ref <branch>`.

The release workflow ([.github/workflows/release.yml](../../.github/workflows/release.yml)) builds the installer again for each `v*` tag, or for a tag named by hand, and attaches it to the GitHub release as `Scratch-Setup-<version>.exe`. It and the nightly workflow call [scripts/build-windows-installer.sh](../../scripts/build-windows-installer.sh), which runs `flutter build windows` with the tag as `SCRATCH_RELEASE_TAG`, bundles Claude Code and runs ISCC. It passes both defines:

- `/DAppVersion` is the numeric version (`0.1.0`, the tag without the pre-release part). It sets the file version and the output file name.
- `/DVersionText` is the full version (`0.1.0-prealpha.1`, the tag without `v`). It is what the installer displays and what Apps & features shows.

`VersionText` is optional. Without it the installer shows `AppVersion`. `windows-build` passes only `AppVersion`, so its artifacts show the numeric version.

To build locally after a Release build, from `app/`:

```
"C:\Program Files (x86)\Inno Setup 6\ISCC.exe" /DAppVersion=0.2.0 /DSha7=local windows\installer\scratch.iss
```

Without `/DSourceDir` and `/DOutputDir` it reads `build\windows\x64\runner\Release` and writes to `build\windows\installer`.

A local build has no Claude Code package: its manifest is the checked-in empty one, so the app offers no download. To get Claude Code in a local build, set `NOTES_CLAUDE_CODE_DIR` and put Bun on `PATH` (or use Settings > Agent > Advanced), or build the package with Bun 1.4.2 on `PATH`, from the repository root:

```
pwsh -File app/windows/installer/build-package-claude-code.ps1 -OutDir packages -BaseUrl https://example.invalid
```

and unpack the zip into `<data dir>/packages/claude-code/<version>/` with a `.package.json` marker (see [Runtime packages](../addons/packages.md)).

## Not yet verified

- The installer has not been run on a PC yet: install, Start menu search, upgrade over an older install, closing a running Scratch, and uninstall are untested by hand.
