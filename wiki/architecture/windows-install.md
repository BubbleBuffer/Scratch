---
type: page
summary: "Per-user Inno Setup installer built in the windows-build workflow: where Scratch installs, shortcuts, upgrade, uninstall and how to build it."
parent: "[[architecture/index]]"
status: current
---

# Windows installer

Built 2026-10-21 on `lane/win-installer`. The script is [app/windows/installer/scratch.iss](../../app/windows/installer/scratch.iss).

## What it does

- **Per-user, no admin.** `PrivilegesRequired=lowest`. It installs the `flutter build windows` Release folder to `%LOCALAPPDATA%\Programs\Scratch`. The exe stays `notes.exe`.
- **Shortcuts.** A Start menu shortcut named Scratch with the app icon, so Windows Search finds it. A desktop shortcut is an option on the tasks page, unchecked by default.
- **Apps & features.** An uninstall entry under HKCU: name Scratch, the pubspec version, publisher BubbleBuffer, and the exe's icon.
- **Upgrade in place.** The AppId GUID is fixed (`1B472506-0E7C-46CD-B530-F256FB709BB5`), so a newer installer replaces the same install. Never change it.
- **Running app.** If Scratch is open, the installer closes it through the Restart Manager (`CloseApplications=yes`) before replacing files. It does not restart it; the finish page has a "Launch Scratch" box.
- **Uninstall.** Removes the program files, the shortcuts and `HKCU\Software\Classes\notes` (the `notes://` scheme). The installer writes that key at install time, pointing at the installed `notes.exe`, so links work before the first start; the app also rewrites it on every start ([app state, page links](app-state.md#page-links)). User data in `%LOCALAPPDATA%\Notes` is never touched.
- **Not done.** Unsigned (SmartScreen warns once on first run). No auto-update, autostart or file associations.

## Building it

CI builds it in the `app` job of [.github/workflows/windows-build.yml](../../.github/workflows/windows-build.yml), right after `flutter build windows --release`. Inno Setup 6 ships on the windows-latest image; the step installs it with choco if it is missing. The version is the `version:` in `app/pubspec.yaml` without the build number. The artifact is `scratch-setup-<sha7>`, holding `Scratch-Setup-<version>-<sha7>.exe`, kept 7 days beside `notes-windows-<sha7>` (the plain folder).

The workflow is manual: `gh workflow run windows-build.yml --ref <branch>`.

To build locally after a Release build, from `app/`:

```
"C:\Program Files (x86)\Inno Setup 6\ISCC.exe" /DAppVersion=0.2.0 /DSha7=local windows\installer\scratch.iss
```

Without `/DSourceDir` and `/DOutputDir` it reads `build\windows\x64\runner\Release` and writes to `build\windows\installer`.

## Not yet verified

- The installer has not been run on a PC yet: install, Start menu search, upgrade over an older install, closing a running Scratch, and uninstall are untested by hand.
