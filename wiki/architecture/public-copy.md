---
type: topic
layer: state
summary: "The public Scratch repo: a pre-alpha, AGPL-3.0-or-later snapshot of this repo's master, refreshed by scripts/publish-public.sh."
parent: "[[architecture/index]]"
status: current
---

# Public copy (BubbleBuffer/Scratch)

This repo stays the development home. `BubbleBuffer/Scratch` is a public snapshot of its `master` tree, with its own history: one commit per refresh, titled "Sync from notes-app <sha7>". None of this repo's history is copied.

- **Licence:** `AGPL-3.0-or-later` (set on every crate; full text in [LICENSE](../../LICENSE)), with bundled third-party code and assets in [THIRD_PARTY.md](../../THIRD_PARTY.md). The README opens with a pre-alpha banner.
- **Refresh:** `scripts/publish-public.sh [--ref REF] [--push] [CHECKOUT]` exports `origin/master` (default) into the Scratch checkout (default `F:/build/public/Scratch`), drops the paths in `scripts/publish-public.exclude`, and commits. Without `--push` it only commits locally. The commit uses the configured git identity.
- **Exclusions:** app screenshots except the two this wiki embeds, design capture shots, a design report with local tool paths, one benchmark provenance file with local temp paths, and the publish tooling itself.
- **BubblePaw dependency:** here `crates/agent` and `crates/tools` use path dependencies into a sibling BubblePaw-base checkout (CI checks out its `release` branch with a deploy key). The export rewrites them to git dependencies on the public `BubbleBuffer/BubblePaw-Core` repo, pinned to `CORE_REV` in the script, and adds the matching `source` lines to `Cargo.lock`, so the public copy builds with `--locked` against BubblePaw-Core alone. BubblePaw-Core is a one-commit-per-sync export of BubblePaw-base (its `lane/public-core` branch and its own `scripts/publish-public.sh`); bump `CORE_REV` whenever it is re-synced. This repo itself keeps the path dependencies because notes-app CI has no credentials for BubblePaw-Core while it is private.
- **Guard:** before committing, the script refuses an export that contains the private host addresses, local user paths, personal account names or common secret formats it checks for. Two example addresses are allowed: the Tailscale range in `pairing_codec.dart` and the LAN address in the sync golden test.
- **Hosts stay out of the tree:** workflows read deploy targets from repository variables (`VPS_HOST`, `VPS_USER`, `APK_PUBLISH_SSH`); scripts take them from the environment or the ssh config. Docs use placeholders such as `VPS_IP` and `DOCKER01_IP`.
- **Actions are disabled on Scratch,** so its copies of the workflows never reach the self-hosted runners. The workflows also check `github.repository == 'BubbleBuffer/notes-app'`.
