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
- **Guard:** before committing, the script refuses an export that contains the private host addresses, local user paths, personal account names or common secret formats it checks for. Two example addresses are allowed: the Tailscale range in `pairing_codec.dart` and the LAN address in the sync golden test.
- **Hosts stay out of the tree:** workflows read deploy targets from repository variables (`VPS_HOST`, `VPS_USER`, `APK_PUBLISH_SSH`); scripts take them from the environment or the ssh config. Docs use placeholders such as `VPS_IP` and `DOCKER01_IP`.
- **Actions are disabled on Scratch,** so its copies of the workflows never reach the self-hosted runners. The workflows also check `github.repository == 'BubbleBuffer/notes-app'`.
