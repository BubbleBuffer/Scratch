---
type: topic
layer: state
summary: "How Android release builds are signed, numbered and published to the owner's private F-Droid repo."
parent: "[[architecture/index]]"
status: current
---

# Android release

**Status:** current, 2026-10-10.

- **Where builds go.** Beta builds are published to the owner's private F-Droid repo (`apps.example.com/repo`, the `apk-repo` skill). Droid-ify on the phone installs and auto-updates them. The first beta was published on 2026-10-07.
- **App id.** `de.bubblebuffer.notes` since 2026-10-08 (the owner wants all apps under `de.bubblebuffer`); it was `dev.notesapp.app`. The repo keys apps by it, and the phone treats a new id as a new app, so the old install has to be removed by hand. Do not change it again.
- **Signing.** The Linux machine holds the release key outside the repo at `/home/dev/keys/notes/` (`release.jks` plus `keystore.properties`, mode 0600). The PC copy at `%LOCALAPPDATA%\Notes\android-keystore\` remains a backup. `app/android/app/build.gradle.kts` falls back to a debug key on the self-hosted CI runner; the Linux machine zipaligns, re-signs with the stable release key, verifies, and checks the signer against the published build before upload. Losing the release key prevents updates to the existing phone install.
- **Version code.** It is the minute count at build time and goes up with every build: `flutter build apk --release --build-number=$(( $(date +%s) / 60 ))`. Build one universal APK, without `--split-per-abi`, so the code isn't offset per ABI.
- **Build and publish.** Since 2026-10-12 every master push that passes `ci` builds and publishes automatically (a newer run cancels an older one). For a manual build, the private APK dispatch script dispatches `.github/workflows/android-apk.yml` on master, watches it, and prints the publish result. The self-hosted CI runner builds arm64 and streams the APK directly over vmbr1 to the Linux machine, using `/home/builder/.ssh/<linux-host>-apk`. Its restricted key only permits `apk-publish notes`, with the APK on stdin. No GitHub artifact, PC signing or WSL step is involved.
- **Repo.** `/srv/fdroid` on the Linux machine owns config, the index signing key, repo/archive and metadata. `apk-repo` refuses unsigned APKs, non-increasing versionCodes and changed app signers. The phone's index fingerprint remains `167B90C8779709943A382FAE4F67D17570375F0FA22749C2086CC9F4F9A074F9`. WSL `/root/fdroid` stays as a backup.
- **Upload.** The Linux machine's dedicated restricted key uploads to VPS OpenSSH at its Tailscale address on port 2222; the forced command validates repo/archive tar input and swaps the served directories. Mesh port 22 belongs to NetBird's built-in SSH server and is not the upload endpoint.
- **Manual publish.** From Git Bash, use Windows OpenSSH: `ssh <linux-host> apk-repo publish < signed-release.apk`, or scp the APK first and pass its remote path. For an unsigned/debug-signed Scratch APK, `ssh <linux-host> 'apk-publish notes' < notes.apk` performs release signing there. `ssh <linux-host> apk-repo list` shows apps and versionCodes.
- **Icon.** It is the book-cat [app icon](../product/design.md#app-icon): mipmaps plus an adaptive icon in `mipmap-anydpi-v26` with foreground, background and monochrome layers.
