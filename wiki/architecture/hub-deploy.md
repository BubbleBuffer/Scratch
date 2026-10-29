---
type: topic
layer: state
summary: "Remote amd64 image build and the lead's deployment procedure for notes-hub behind the existing VPS Caddy and Authentik."
parent: "[[architecture/index]]"
status: current
---

# Hub deployment

**Build step 4c, locally verified 2026-10-08.** The linux/amd64 image, archive and deploy kit are built and tested locally. These are instructions for the lead. No VPS connection or deployment was performed. See [Hub server](hub.md) for config and routes, and [the agent plan](../integrations/agent.md) for 4b and the later desktop relay.

The target is **linux/amd64** at **https://notes.example.com**. Build on the remote Docker host (`docker01`); the Windows PC sends the Scratch context and saves the image archive. The VPS is a deployment target, not the build host. The hub joins the external `notes-ingress` Docker network; Caddy terminates TLS and forwards HTTP to `notes-hub:8080`. The hub publishes no host port.

## Authentik

The existing installation is Authentik 2026.5 in `/home/networking/authentik`, container `authentik-server-1`. In its admin UI:

1. Create the group `notes-users` and add the intended users.
2. Create an OAuth2/OpenID provider with **Public** client type (PKCE, no client secret). Record its client ID. Set the redirect URI to **strict** `https://notes.example.com/auth/callback`.
3. Select an RSA **Signing Key** so ID tokens use **RS256**; leave token encryption unset. Use the provider's existing authorization flow and select the `openid`, `email` and `profile` scope mappings.
4. Keep issuer mode **Each provider has a different issuer, based on the application slug**. Create an application with slug **notes-hub** and associate this provider. Its issuer is `https://<authentik host>/application/o/notes-hub/`, including the trailing slash.

These provider settings follow [Authentik's OAuth2 documentation](https://docs.goauthentik.io/add-secure-apps/providers/oauth2/). Use the real Authentik hostname when filling `.env`.

Because the kit uses `allowed_groups`, create an OAuth2 scope mapping under **Customization → Property Mappings** with scope name `groups` and this expression:

```python
return {
    "groups": ["notes-users"] if ak_is_group_member(request.user, name="notes-users") else []
}
```

Assign it to the provider's scopes and enable **Include claims in id_token**. The hub checks the ID token's `groups` claim; it requests the `groups` scope automatically when `allowed_groups` is set. The mapping and claim setting follow [Authentik's property mapping documentation](https://docs.goauthentik.io/add-secure-apps/providers/property-mappings/); the membership helper is documented in [OAuth2 scope authorization](https://docs.goauthentik.io/add-secure-apps/providers/oauth2/#scope-authorization). If using a different group, change both this expression and `.env`. Both hub allow lists empty means every login is refused.

## Build and send the kit

Run from the Scratch repository root in **PowerShell 7.4 or newer**, with `gzip.exe` on PATH and the `docker01` Docker context available. The build script defaults to `-DockerContext docker01`; it does not require Docker Desktop or run Cargo on the PC. PowerShell 7.4 preserves the binary pipe from `docker save` into gzip:

```powershell
pwsh -File .\deploy\hub\build.ps1
```

The existing smoke harness requires PowerShell and curl on the Docker host with a local daemon, because it connects to its published localhost port. Run it there after the build, not from this PC against the remote daemon:

```powershell
pwsh -File .\deploy\hub\smoke.ps1
```

The smoke script uses a fake issuer, an ephemeral localhost port and its own throwaway container and volume. It checks health, runtime identity, pinned CLI versions, `/data` persistence, emergency pairing and a full empty binary sync round through host curl, then removes its test resources.

The build script builds `notes-hub:latest` for `linux/amd64` and saves `target/notes-hub.tar.gz`. Its equivalent build command is:

```powershell
docker --context docker01 buildx build --platform linux/amd64 --file deploy/hub/Dockerfile --tag notes-hub:latest --load .
```

Scratch alone is the build context. Cargo fetches `bubblepaw-sdk` and `bubblepaw-mcp` from the public BubblePaw-Core repository at tag `v0.3.1`, pinned to `f3603685a040196617f1fb894978493fbe992349` in `Cargo.lock`. The Dockerfile reads the unique Core revision from that lockfile and fetches the same commit for the Claude Code extension, including its frozen Bun lockfile. No named build context or sibling checkout is needed. Cargo builds `notes_hub` in release mode with `--locked` and BuildKit caches. These source changes do not establish a new successful image build; historical verification below predates them.

Pins: Rust **1.97.1** (`rust:1.97.1-bookworm`), Node **22.18.0** (`node:22.18.0-bookworm-slim`), Bun **1.4.2**, Claude Code **2.1.293**. The Codex CLI was removed from the image on 2026-10-25, with the hub's move to the four [hub models](hub.md#agent-models-and-credentials). Runtime base: `debian:bookworm-slim`. Docker reports **2,387,570,531 bytes** (2.39 GB / 2.22 GiB); `target/notes-hub.tar.gz` is **672,359,162 bytes** (672 MB / 641 MiB). Node, Bun and both CLI version commands passed as `hub`.

The following transfer commands are for the lead to run; the build script does not deploy. For example, first create `/home/networking/notes-hub` on the VPS using the lead's normal access, then send the archive and text kit using **Windows OpenSSH**:

```powershell
$hubDestination = '<server>:/home/networking/notes-hub/'
& C:\Windows\System32\OpenSSH\scp.exe .\target\notes-hub.tar.gz $hubDestination
& C:\Windows\System32\OpenSSH\scp.exe .\deploy\hub\compose.yaml .\deploy\hub\.env.example .\deploy\hub\Caddyfile.snippet $hubDestination
```

## Configure and start on the VPS

The lead runs these commands on the VPS, after transferring the files:

```sh
cd /home/networking/notes-hub
gzip -dc notes-hub.tar.gz | docker load
cp .env.example .env
chmod 600 .env
```

Edit `.env`: set the real issuer and client ID, and `NOTES_HUB_OIDC_ALLOWED_GROUPS=notes-users`. Leave `NOTES_HUB_OIDC_CLIENT_SECRET` empty for the public provider; set it only for a confidential one (then the hub sends it with HTTP Basic auth). Keep `NOTES_HUB_PUBLIC_URL=https://notes.example.com`. Comma-separated or space-separated lists work. Environment variables override `/data/hub.toml`; keep secrets in `.env` rather than in the wiki or shell command history.

The model settings are optional: `NOTES_HUB_AGENT_MODELS` (the offered list) and `NOTES_HUB_AGENT_DEFAULT_MODEL` (must be on that list, or startup fails). The defaults are the four [hub models](hub.md#agent-models-and-credentials) with `claude-code/claude-sonnet-5-5` as the default.

### Claude connector

The Claude connector is off by default. Set both variables, or neither:

| Variable | TOML key | Value |
|---|---|---|
| `NOTES_HUB_MCP_OAUTH_ISSUER` | `[mcp_oauth] issuer` | `https://auth.example.com/application/o/<provider>/`, the same as `[oidc] issuer` |
| `NOTES_HUB_MCP_OAUTH_CLIENT_ID` | `[mcp_oauth] client_id` | `<client-id>`, the same as `[oidc] client_id` |

- MCP OAuth uses the `[oidc]` provider: one public client serves device sign-in and every connector. Connectors need only the client ID, with no client secret; the hub itself only validates tokens.
- Setting only one of them is a startup error. Both need `NOTES_HUB_PUBLIC_URL`.
- The allow-list comes from the `oidc` section. If both allow lists are empty, the hub warns at start and refuses every OAuth request.
- What exists on the Authentik side is in [Claude connector](../integrations/ai-assistants.md#claude-connector). What the endpoint accepts is in [Hub server](hub.md#agent-tokens-and-mcp-mcp).

Create the external network once, before using compose:

```sh
docker network create notes-ingress
```

If it already exists, use it. The compose kit uses `restart: unless-stopped`, a 1 GB memory limit, a 512 PID limit and `no-new-privileges`.

The image runs as `hub`, UID **10001**, with `HOME=/data/home` and data dir `/data`. The named volume `notes-hub-data` contains the vault replica, hub config and provider sign-ins; 4b also places the agent store under the data directory. A fresh Docker volume is seeded from the image with an empty `/data/hub.toml` and a writable home directory, so env-only configuration works.

The entrypoint is `tini -- notes-hub --config /data/hub.toml`. **An explicitly named config file must exist**: the binary errors if it is absent. For an existing volume without the file, create it before starting:

```sh
docker compose run --rm --no-deps --user 0 --entrypoint sh hub -c 'mkdir -p /data/home; touch /data/hub.toml; chown 10001:10001 /data /data/home /data/hub.toml'
```

This initializes only those paths; it does not recursively change ownership of an existing vault. A supplied TOML config must contain supported keys ([reference](hub.md#config)).

## Caddy

Add [Caddyfile.snippet](../../deploy/hub/Caddyfile.snippet) to `/home/networking/caddy/templates/Caddyfile.template`, not to the generated `configs/Caddyfile`:

```caddyfile
notes.example.com {
    import cloudflare
    header {
        Strict-Transport-Security "max-age=31536000; includeSubDomains; preload"
        X-Content-Type-Options "nosniff"
        X-Frame-Options "DENY"
    }
    reverse_proxy notes-hub:8080
}
```

In Caddy's compose file, append the network to the `caddy` service's existing network list and add the top-level definition, preserving all other entries:

```yaml
services:
  caddy:
    networks:
      # Keep the existing networks here.
      - notes-ingress

networks:
  # Keep the existing network definitions here.
  notes-ingress:
    external: true
```

Use the same DNS setup as the existing sites for `notes.example.com`. WebSockets pass through `reverse_proxy` by default; `flush_interval -1` is unnecessary.

Start the hub, render the Caddy template, and recreate Caddy with the added service network:

```sh
cd /home/networking/notes-hub
docker compose up -d
cd /home/networking/caddy
docker compose run --rm config-generator
docker compose up -d caddy
```

The template path, service names and generator command above come from the lead's supplied layout, not a live VPS inspection. `up -d caddy` attaches the newly added network and recreates the service when its compose config changes, loading the rendered Caddyfile. For later template-only updates, rerun the generator and reload Caddy using the config path mounted by that compose file (or restart the `caddy` service); `up -d` alone does not reload an otherwise unchanged running container.

## Provider sign-in and checks

From `/home/networking/notes-hub`, sign in as the default non-root container user:

```sh
docker compose exec -it hub claude
# At Claude Code's prompt: /login
```

Follow the CLI's browser instructions from the lead's browser. The interactive sign-in persists under `/data/home/.claude`, which is the default for the hub's `HOME`. The hub's models need credentials from its environment, set in `.env` (`chmod 600`) like the OIDC secret; a run without one fails naming the variable:

- Claude models: `CLAUDE_CONFIG_DIR`, a directory mounted in the container with Claude Code's `.credentials.json` and login files, or `CLAUDE_CODE_OAUTH_TOKEN`, a token for the account. The default container login directory under `HOME` also works. Prefer one credential source; an inherited token can override the directory login.
- DeepSeek (`opencode/deepseek-v4.1-flash`): `OPENCODE_API_KEY`.

After an interactive sign-in or replacing login files, restart the hub (`docker compose restart hub`), since the agent checks credential availability at start. After changing `.env`, recreate it with `docker compose up -d` to load the new environment. Keep the values out of the wiki and shell history. The image sets `NOTES_HUB_AGENT_BUN=/usr/local/bin/bun` and `NOTES_HUB_AGENT_CLAUDE_CODE_EXTENSION=/opt/bubblepaw/claude-code`; `vault_agent` checks that extension directory and appends `src/main.ts`.

```sh
docker compose ps
docker compose logs --tail 100 hub
docker compose exec hub curl --fail --silent http://127.0.0.1:8080/health
curl --fail --silent https://notes.example.com/health
docker compose exec hub notes-hub --config /data/hub.toml --devices
```

Both health requests should return HTTP 200 with body `notes-hub`. Then point the app's hub setting at `https://notes.example.com`, sign in as a `notes-users` member and check one sync round; check a user outside the allowed group is refused. `/health` alone does not prove Authentik login, app callbacks or agent provider access. Once 4b is available, check the remote Agent section and streamed responses too.

For emergency pairing only, `docker compose exec hub notes-hub --config /data/hub.toml --pair <label>` prints a bearer token. The running hub reloads token file changes before authentication checks and reloads under a lock before every save, so `--pair` and `--unpair` take effect without a restart. Revoke the token after recovery with the matching `--unpair <label>` command. A restart cancels logins and agent connections in progress.

## Updates, backups and rollback

The [backup kit](../../deploy/hub/backup/install-server.md) runs the same stopped-volume backup at **02:30 UTC daily** through `notes-hub-backup.service` and its timer on the VPS, installed 2026-10-14. The first run, `notes-hub-data-20261014T024635Z.tar.gz`, succeeded. The root oneshot stops the compose `hub` service, streams a root-owned tar of the complete volume through the installed hub image, and restarts the hub. An exit trap retries the restart on failure. A shared `.deploy.lock` excludes deployments during the backup.

Archives live in `/opt/notes-hub/backups/notes-hub-data-<UTC timestamp>.tar.gz`, with a sibling `.tar.gz.sha256`. The directory is `0700`; archives and checksums are `0600`. The script keeps the newest seven archives and refuses to stop the hub when free disk is below twice the last archive size. A first backup has no previous size to compare.

The [Linux machine pull kit](../../deploy/hub/backup/install-home.md) runs as `dev` through a systemd user timer at **04:00 UTC daily** (since 2026-10-14) and pulls into `/home/dev/backups/notes-hub/`. Its key is restricted to `rrsync -ro /opt/notes-hub/backups`. It checks every SHA-256 and gzip/tar structure before publishing, and keeps 30 days by archive timestamp. Its private key stays on the Linux machine. The VPS public-key entry uses `restrict,command="/usr/bin/rrsync -ro /opt/notes-hub/backups"`; it grants read-only backup-directory access. Both timers use `Persistent=true`.
Before loading a replacement image, retain the current image and take a backup. After the approved kit installation, run from `/opt/notes-hub`:

```sh
docker tag notes-hub:latest notes-hub:previous
/usr/local/libexec/notes-hub/notes-hub-backup.sh
```

The archive includes the vault, tokens, TOML config, agent database and provider sign-ins. Protect it like credentials; never put it in the repository. Save `.env` separately in a private credential backup. Keep previous image tags and avoid deleting the data volume (`docker compose down -v`). Check both backup and pull journals after failures.

Load the new image archive with the same `gzip -dc ... | docker load` command, then:

```sh
docker compose up -d --force-recreate hub
```

Repeat the health, login and sync checks. To roll back the binary:

```sh
docker tag notes-hub:previous notes-hub:latest
docker compose up -d --force-recreate hub
```

An image rollback keeps current data; it does not undo a data-format migration. For a restore, obtain a verified archive from either private backup directory, choose an image compatible with that archive, stop the hub, create a fresh named volume, and extract as root so numeric ownership is preserved. Change the compose volume's `name:` to the restored volume. Keep the original volume until health, device, login and sync checks pass. Run as root from `/opt/notes-hub`; replace the timestamp with the selected archive:

```sh
umask 077
archive=notes-hub-data-YYYYMMDDTHHMMSSZ.tar.gz
(cd backups && sha256sum --check "$archive.sha256")
gzip --test "backups/$archive"
docker compose stop hub
docker volume create notes-hub-data-restore
docker run --rm --user 0 --entrypoint tar -v notes-hub-data-restore:/data -v "$PWD/backups:/backup:ro" notes-hub:previous --numeric-owner --same-owner -C /data -xzf "/backup/$archive"
# Edit compose.yaml: volumes.notes-hub-data.name: notes-hub-data-restore
docker compose up -d --force-recreate hub
```

The [restore verifier](../../deploy/hub/backup/restore-test.sh) runs on the Docker host against a private archive and checksum. It creates a fresh `notes-hub-bk-test-restore-*` volume, extracts as root, and compares archive contents and ownership. Before the hub starts, it compares the SHA-256 of every file under `vault/.vault/docs`, `vault/.vault/logs`, `vault/pages` and `vault/.vault/hub`, and of `hub.toml`, between the archive and the restored volume. The documents in `.vault/docs` are the content of record, so an archive with none fails unless `--allow-empty-vault` is given. The 2026-10-14 production archive has an empty vault and needs that flag. The verifier then starts an isolated hub on a free localhost port with an empty test config, a fake issuer and the agent disabled, checks `/health` and `notes-hub --devices`, and removes its own resources. The private backup test script seeds two synthetic pages through the hub's own agent MCP, so the archive holds real Loro documents. It then exercises backup failure recovery and retention.

**Restore test: 2026-10-14.** The Docker host harness seeds two pages through the hub's agent MCP, so the archive holds two Loro documents, two update logs and two Markdown pages. The restore check compares every hashed file before startup, then passes health and `--devices`. Tar failure, SIGTERM, disk-space refusal and retention checks pass. The production archive of 2026-10-14 has an empty vault: it fails without `--allow-empty-vault` and passes with it. These are filesystem restore checks, not authenticated sync or real provider sign-in tests. Earlier recorded output and verification limits were in the former private repo's `deploy/hub/backup/TEST-REPORT.md`; that file is not in Scratch.

## Verification record and unverified points

- **Build passed:** `pwsh -File deploy/hub/build.ps1` on Docker Engine 29.8.2, linux/amd64, including the locked release build and `gzip --test`. Built the working tree at HEAD `7327d6d` with the committed on master 4b changes; no clean-checkout fallback was needed. The earlier Docker startup block is resolved.
- **Image identity:** `sha256:158818ddd074feaf82747c9d49367a7cc38522cabe01ab532087145de3428d43`. Archive SHA-256: `c9eed039149e40d8ab4d9b12b440e6891332a9a9a5ee2f2689cfba20332a00db`.
- **Smoke passed:** fake issuer and env configuration with the seeded empty TOML; host curl `/health` returns HTTP 200 `notes-hub`; Docker HEALTHCHECK is healthy; `notes-hub --devices`; Claude, Codex, Bun and Node match their pins as UID 10001; Codex help supports device auth. Config, vault, agent SQLite and writable CLI homes live under `/data`, owned by `hub`.
- **Pair/sync passed:** emergency `--pair test`, restart to reload the token, host curl `POST /sync` with a valid v1 full empty request, decoded converged response and persisted device binding. CLI-home write probes survive the restart. This verifies an empty protocol round, not page-content replication through a host `SyncClient`.
- **Smoke harness fix:** Docker chose a new ephemeral host port on restart; the script now refreshes the mapping. The corrected complete run passed. Build and smoke logs are in `target/hub-build-verification.log` and `target/hub-smoke-verification.log`.
- **Static checks passed:** PowerShell parsing, Compose config validation and diff whitespace checks.
- **Test cleanup passed; cache cleanup blocked-env:** both throwaway containers and volumes were removed. `docker builder prune --force` reported deletions, then stalled without a completion total; a final 60-second retry also timed out. Only the verified prune CLI processes were canceled. Logs: `target/hub-build-prune.log` and `target/hub-build-prune-retry.log`. No Docker/WSL restart or changes to other containers were made; the image and archive were retained.
- The real VPS, DNS/Cloudflare TLS, Caddy compose mount and reload, Authentik 2026.5 UI and complete real OIDC/app login: not verified here.
- Actual provider sign-in and a model run need the lead's credentials and remain unverified.
- Backup/restore and rollback commands are documented, not executed against production data.

## Live deployment

- Running at `https://notes.example.com` on the VPS (`<server>`), from `/opt/notes-hub` (compose file plus a `0600` `.env`). The data volume is `notes-hub-data`.
- Running image: commit `d45af66`, deployed 2026-10-14 by CI (run 38544539596 in the former private repo) after the first push to that repo's `release` branch. It serves `/mcp` with agent tokens. `notes-hub:previous` is the 2026-10-08 hand-loaded image (`0caec07`). Both providers are signed in inside the container.
- Authentik (`auth.example.com`):
  - The provider and the application are both called `notes-hub`, created through `ak shell`.
  - It is a public client, with no client secret since 2026-10-22 (it was confidential before): strict redirect `https://notes.example.com/auth/callback`, signed with the self-signed key, scopes openid, email and profile.
  - Allowed grant types: authorization code and refresh token. Authentik 2026.5 checks this list per provider; a provider created through `ak shell` had it empty, and every login failed with "Invalid grant_type for provider" until it was set by hand (2026-10-15). The first real login, from the PC (`RobinPC`), succeeded right after.
  - The group is `notes-users`, with `akadmin` in it.
  - The hub asks for `openid email profile` (`NOTES_HUB_OIDC_SCOPES`); Authentik's profile scope carries `groups`.
- Caddy:
  - Backups `*.before-notes-hub-20261008T*` were taken first.
  - The template gained the `notes.example.com` block; the compose file gained `notes-ingress`.
  - Caddy was attached live with `docker network connect` and reloaded, with no recreate.
- Checked live:
  - `/health` returns 200 (2026-10-08 and 2026-10-14).
  - `/mcp` without a token returns 401, and with a foreign Origin returns 403 (2026-10-14).
  - `/auth/start` returns a 302 to Authentik with PKCE S256.
  - `/agent/ws` without a token returns 401.
  - The agent opened on start (2026-10-14 default model `codex/gpt-6-luna`; from 2026-10-25 the default is `claude-code/claude-sonnet-5-5`, see [hub models](hub.md#agent-models-and-credentials)).
- Still to do:
  - Provider sign-ins in the container (`docker compose exec -it hub claude`, or the credential variables in [Provider sign-in](#provider-sign-in-and-checks)).
  - A full browser login.
  - A real phone login, vault sync and agent turn. The app-side browser login UI is committed (`a806fbb`); Android agent wiring is on `mobile-agent` ([state and phone steps](mobile-agent.md)).

## CI deploys (scratch-ops)

The hub image is built and deployed from the private scratch-ops repo. The former private repo's `hub-image.yml` is retired; see the history bullet.

- **Repos:** `BubbleBuffer/Scratch` (public) is the source. `BubbleBuffer/scratch-ops` (private) holds the `hub-deploy` workflow, the VPS deploy key and the private patterns. The hub image takes the BubblePaw-Core commit that `Cargo.lock` pins for tag `v0.3.1`.
- **Trigger:** by hand, from scratch-ops:
  `gh workflow run hub-deploy.yml -R BubbleBuffer/scratch-ops -f ref=<tag> -f dry_run=false`
  `dry_run=true` runs the build without the deploy.
- **Build:** the workflow builds the image from Scratch at `ref`, with the BubblePaw-Core commit that `Cargo.lock` pins for tag `v0.3.1`, and pushes `ghcr.io/bubblebuffer/scratch-hub:<sha>`. The package is private. The workflow runs on the home build server, not on GitHub-hosted runners.
- **Deploy:** the workflow connects to the VPS with a key whose `authorized_keys` entry is `restrict,command="/opt/notes-hub/deploy.sh"`, so it can run nothing else. The 40-character commit sha goes as the command and a GHCR token on stdin. [deploy.sh](../../deploy/hub/deploy.sh):
  - logs in to GHCR in a temporary `DOCKER_CONFIG`, so no lasting token sits on the VPS
  - pulls `ghcr.io/bubblebuffer/scratch-hub:<sha>`, keeps the current `notes-hub:latest` as `notes-hub:previous` and retags the new image as `notes-hub:latest`
  - runs `docker compose up -d` and waits up to 2 min for the container to report healthy, rolling back to `:previous` otherwise
  - prunes old `scratch-hub` and `notes-hub` images, since the disk is tight
- **Manual rollback on the VPS:** `docker tag notes-hub:previous notes-hub:latest && docker compose up -d`.
- **Cost:** no GitHub-hosted minutes; the build runs on the home build server.
- **History (former private repo, until 2026-10-22):**
  - Deploys ran from the former private repo's `hub-image.yml` on pushes to its `release` branch, with image `notes-hub:<sha>` on GHCR. Its `setup-ci.sh` created the keys; that script is not in Scratch.
  - On 2026-10-14 a new VPS deploy key was generated, `VPS_DEPLOY_KEY` replaced, `/opt/notes-hub/deploy.sh` installed, and the key added to root's `authorized_keys` as `restrict,command="/opt/notes-hub/deploy.sh"` (backup `authorized_keys.bak-notes-app-ci-deploy-20261014`). The first deploy attempt failed with "Permission denied (publickey)" before this; the rerun deployed.
  - The image name became `scratch-hub` when the deploy moved to scratch-ops. The VPS container and local tags keep the `notes-hub` name.
