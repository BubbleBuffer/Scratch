---
type: topic
layer: state
summary: "How Android builds are numbered, signed and published: a Scratch release is built on GitHub, then release-signed on the home Linux box and published to the owner's private F-Droid repo."
parent: "[[releases/index]]"
status: current
---

# Android release

**Status:** current, 2026-10-22. The release pipeline moved to the public Scratch repo on 2026-10-22; the older pipeline ran from the former private repo (see the last bullet).

- **Where builds go.** Scratch releases are published to the owner's private F-Droid repo through the scratch-ops `apk-publish` workflow and the home Linux box (see Build and publish). Droid-ify on the phone installs and auto-updates them. The first beta was published on 2026-10-07.
- **App id.** `de.bubblebuffer.notes` since 2026-10-08 (the owner wants all apps under `de.bubblebuffer`); it was `dev.notesapp.app`. The repo keys apps by it, and the phone treats a new id as a new app, so the old install has to be removed by hand. Do not change it again.
- **Signing.** The release key lives only on the home Linux box, outside the repo. `app/android/app/build.gradle.kts` uses the release key when a local keystore is present and falls back to the debug key otherwise. The GitHub build always uses that fallback. [release.yml](../../.github/workflows/release.yml) then re-signs the APK with a separate pre-alpha key when the `PREALPHA_*` secrets are set (in the `release` environment), or ships it as `...-unsigned.apk` otherwise. Either way it is not release-signed and does not update an installed release-signed app. The home Linux box re-signs it with the release key, verifies it, and checks the signer against the published build before upload. Losing the release key prevents updates to the existing phone install.
- **Version code.** It is the minute count at build time and goes up with every build: `flutter build apk --release --build-number=$(( $(date +%s) / 60 ))`. The release workflow builds the arm64 APK only (`--target-platform android-arm64`), with no `--split-per-abi`, so the code isn't offset per ABI.
- **Build and publish.**
  1. A `v*` tag is pushed to Scratch (or the `release` workflow is run by hand with a tag). The `release` workflow builds the arm64 APK on GitHub-hosted runners and attaches it to the GitHub release as `scratch-<version>-android-arm64.apk`.
  2. The scratch-ops `apk-publish` workflow polls Scratch releases every 30 minutes and downloads the arm64 APK.
  3. The home Linux box release-signs it with `apk-publish notes` and publishes it to the private F-Droid repo. The signing key never leaves that box.
  4. A marker tag `apk-published/<tag>` records a release that is done.
- **Repo.** The home Linux box owns the F-Droid repo: config, the index signing key, the repo and archive, and metadata. `apk-repo` refuses unsigned APKs, non-increasing versionCodes and changed app signers. The phone's index fingerprint remains `167B90C8779709943A382FAE4F67D17570375F0FA22749C2086CC9F4F9A074F9`.
- **Upload.** The home Linux box uploads to the VPS over a dedicated restricted key. The forced command validates the repo and archive input and swaps the served directories. The upload goes through the VPS's OpenSSH on a non-default port, not the mesh SSH port.
- **Manual publish.** The `apk-repo` skill covers publishing by hand from the home Linux box, and listing apps and versionCodes there.
- **Icon.** It is the book-cat [app icon](../product/design.md#app-icon): mipmaps plus an adaptive icon in `mipmap-anydpi-v26` with foreground, background and monochrome layers.
- **Former pipeline (until 2026-10-22).** The former private repo built the APK on its self-hosted CI runner for every master push that passed CI, then streamed it to the home Linux box through a restricted `apk-publish notes` key. That workflow, the private APK dispatch script and the runner path are retired with that repo.
