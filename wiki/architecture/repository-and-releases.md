---
type: topic
layer: state
summary: "Where Scratch's code, CI, releases and private operations live since 2026-10-22: the public repo, the private scratch-ops repo, the private-strings check and how to cut a release."
parent: "[[architecture/index]]"
status: current
---

# Repository and releases

**Status:** current, 2026-10-22.

## Where things live

- **BubbleBuffer/Scratch (public)** is the only source repo. Development, CI and pre-alpha releases happen here. Its CI runs on GitHub-hosted runners only.
- **BubbleBuffer/notes-app (private)** is archived. Scratch is no longer an export of it: `scripts/publish-public.sh` is gone. Runs in the former private repo are cited as "run N in the former private repo".
- **BubbleBuffer/scratch-ops (private)** holds the private operations: the hub deploy, the APK publishing and the private patterns file (`private-strings.txt`). Its workflows run only on the home build server.
- **BubblePaw-Core (public)** supplies the agent crates. `crates/agent` and `crates/tools` depend on its `bubblepaw-sdk` crate by git (and `crates/agent` on `bubblepaw-mcp`), pinned to tag `v0.2.1` in `Cargo.toml`. Bump the tag to upgrade.
- **Licence:** `AGPL-3.0-or-later`, in [LICENSE](../../LICENSE). Bundled third-party code and assets are listed in [THIRD_PARTY.md](../../THIRD_PARTY.md).
- **Hosts stay out of this repo.** Deploy targets live in scratch-ops. Docs use placeholders such as `notes.example.com`.

## CI workflows

All workflows run on GitHub-hosted runners. Every workflow sets top-level `permissions: contents: read`; only the release publish job asks for `contents: write`.

| Workflow | Runs on | What it does |
|---|---|---|
| `ci` | pushes to `main`, pull requests (forks included), manual | Rust: fmt (report only), clippy, `cargo test --workspace`. Flutter: analyze, add-on import check, generated bindings check, fast test tier (goldens excluded). |
| `windows-build` | pushes to `main`, pull requests, manual | Windows app, installer and `notes` CLI as 7-day artifacts. Nothing is released. |
| `release` | tag push `v*`, or manual dispatch with an existing tag | Builds the release assets and creates the GitHub release. See below. |
| `private strings` | pushes, pull requests, manual | Runs `scripts/check-private-strings.sh`. See below. |

## Private strings check

`scripts/check-private-strings.sh` fails when tracked files contain:

- private host names and addresses, local user paths, personal accounts and machine names;
- common secret formats;
- personal tool folders such as `.claude/`, `.impeccable/`, `.codex/`, `.cursor/` and `.vscode/`;
- a workflow job on a self-hosted runner.

The private patterns are the `PRIVATE_STRINGS` Actions secret. Their source is `private-strings.txt` in scratch-ops. Fork pull requests do not get the secret, so they run only the public checks (secret formats, self-hosted runners, personal tool folders). Pushes and pull requests from the repo itself require the patterns.

Files kept on purpose: `.github/`, `.gitignore`, `.gitattributes`, `.dockerignore`, `.cargo/`, `app/.metadata`, `deploy/hub/.env.example`, and the Claude plugin's `.claude-plugin/` and `.mcp.json`.

To run it locally, from the repo root in Git Bash:

```
bash scripts/check-private-strings.sh --patterns <path to private-strings.txt> --require-patterns
```

## Cutting a release

- Tag and push: `git tag v0.1.0-prealpha.2 && git push origin v0.1.0-prealpha.2`
- Or, for a tag that already exists: `gh workflow run release.yml -f tag=v0.1.0-prealpha.2`

The tag must look like `v<major>.<minor>.<patch>` with an optional `-<pre-release>` part. A tag with a pre-release part becomes a GitHub prerelease. The release has five assets:

- `Scratch-Setup-<version>.exe`, the Windows installer
- `scratch-<version>-android-arm64.apk`, the Android app
- `notes-cli-<version>-windows-x64.exe` and `notes-cli-<version>-linux-x64.tar.gz`, the CLI
- `SHA256SUMS.txt`

The Android versionCode is the build time in epoch minutes, so each release is newer than the last. The GitHub APK is not release-signed; see below.

## How the APK reaches F-Droid

1. A Scratch release is published (above).
2. The scratch-ops `apk-publish` workflow polls Scratch releases every 30 minutes and downloads the arm64 APK.
3. The home Linux box release-signs it with `apk-publish notes` and publishes it to the private F-Droid repo. The signing key never leaves that box.
4. A marker tag `apk-published/<tag>` records that the release is done.

Details of the app id, signing and the F-Droid repo are in [Android release](android-release.md).

## Deploying the hub

Run the scratch-ops `hub-deploy` workflow by hand:

```
gh workflow run hub-deploy.yml -R BubbleBuffer/scratch-ops -f ref=<tag> -f dry_run=false
```

It builds `ghcr.io/bubblebuffer/scratch-hub:<sha>` from Scratch at `ref`, and deploys it to the VPS through a forced-command key. Details are in [Hub deployment](hub-deploy.md).

## Scratch repo settings

Actions is enabled with all actions allowed. The default workflow token is read-only and cannot approve pull requests. Fork pull requests from first-time contributors wait for approval.
