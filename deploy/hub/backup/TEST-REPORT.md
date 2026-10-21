# Hub backup verification — 2026-10-14

> **Update, 2026-10-14:** the docker01 harness now seeds real documents through the hub's `/mcp` endpoint instead of the synthetic Markdown fixture described below. The restore check hashes `.vault/docs`, `.vault/logs`, `pages` and the hub state directory (including token files) plus `hub.toml`, before startup. See [hub deployment](../../../wiki/architecture/hub-deploy.md) for the current description. The sections below record the earlier run.

All five deliverables are complete. VPS installation remains pending approval.
No VPS connection, production request, timer installation or timer enablement
occurs in this run. Changes stay in `deploy/hub/backup/` and the backup section
of `wiki/architecture/hub-deploy.md`.

| Deliverable | Result |
| --- | --- |
| Stopped-volume backup script | Done; executed against the docker01 test hub. |
| VPS service, timer and installation guide | Done; staged systemd verification passes on linux01. |
| Home pull script, user service/timer and guide | Done; restricted SSH pull on linux01 passes. No timer installed. |
| Disposable restore verifier | Done; both original backup and linux01 copy restore on docker01. |
| Deployment wiki backup section | Done; schedule, copies, restoration and dated result documented; installation pending. |

## Image and fixtures

The image builds on docker01 with the repository's unchanged Dockerfile and
the named BubblePaw build context. No Rust, Flutter or Gradle build runs on
the PC. Source commits: notes-app `43b1a27`, BubblePaw `32986c5`.
The image is tagged `notes-hub:bk-test` and its inspected ID is
`sha256:2d922b091da4b2c61c909f6397de14734f3afa3a92c6ce5007cb5c65d2d0327c`.
Docker Engine is 29.9.0.

The test uses a fresh `notes-hub-bk-test-src` volume and compose container,
three renderer-format Markdown page files with valid IDs, and synthetic
markers under both provider homes. All configuration is synthetic; the agent
is disabled. Archive files and sidecars are `0600` in a `0700` directory.
No archive, private key, real token or provider credential enters the repository.

The build, backup, restore and cleanup commands use `--context docker01`.
On the Docker host, that context points to its local socket through a private
temporary `DOCKER_CONFIG`; initial context/image discovery uses that host's
local default context before the named test context is created.
The repeatable test invocation is:

```sh
DOCKER_CONFIG=/home/dev/notes-hub-bk-test-20261014/docker-config \
DOCKER_CONTEXT=docker01 HUB_IMAGE=notes-hub:bk-test \
bash backup/test-docker01.sh /home/dev/notes-hub-bk-test-20261014
```

The source container and volume are removed by the harness. The private
archives remain temporarily for the pull and off-host restore tests.

## Executed docker01 output

The final complete harness exits zero:

```text
test: PASS source HTTP 200 notes-hub, Docker healthy, three synthetic pages
test: PASS archive SHA-256, private permissions, three exact page hashes, numeric ownership, provider markers
restore-test: PASS archive content and ownership comparison
restore-test: PASS health HTTP 200 notes-hub at http://127.0.0.1:32775/health
restore-test: PASS notes-hub --devices
restore-test: PASS restored page hashes unchanged (3 Markdown pages)
restore-test: PASS restored vault has 3 Markdown pages; synthetic OIDC configuration, agent disabled
restore-test: removed owned test resources notes-hub-bk-test-restore-20261014T004537Z-354350-13205
test: PASS isolated restore
test: PASS injected tar failure returns nonzero, restarts source, publishes nothing, removes partials
test: PASS injected TERM returns 143, restarts source, publishes nothing, removes partials
test: PASS sparse oversized previous archive rejects backup before stopping source
test: PASS eight backups retain newest seven complete, verified pairs
test: PASS owned source resources removed; synthetic backups retained in /home/dev/notes-hub-bk-test-20261014/backups
```

The free-space fixture uses a sparse file and checks its physical allocation
is below 1 MiB; it does not fill the disk. The failure and signal fixtures
target only the backup process started by this harness.

Two earlier harness attempts fail safely and remove their owned resources.
The first exposes Docker 29's suppression of published ports on internal
bridges; dedicated normal bridges with localhost-only publishing resolve it.
The second exposes ZFS's nonzero metadata accounting for an empty sparse
file; the allocation assertion is corrected. The final run above covers both
corrections. Normal bridges do not block outbound traffic; the hub instead
uses an empty test config, a fake issuer and a disabled agent.

## Executed linux01 pull and restricted-key checks

A disposable SSH server on docker01, `notes-hub-bk-test-ssh`, exposes only the
private test backup directory through a read-only bind mount. Its dedicated
key entry is `restrict,command="/usr/bin/rrsync -ro /backups"`. The private
client key is generated and kept on linux01. The server host key is obtained
through the existing trusted docker01 administrative connection and pinned
on linux01. The pull uses this configuration:

```sh
BACKUP_HOST=root@DOCKER01_IP
BACKUP_SSH_PORT=32768
BACKUP_SSH_KEY=/home/dev/notes-hub-bk-test-20261014/pull-key
BACKUP_KNOWN_HOSTS=/home/dev/notes-hub-bk-test-20261014/known_hosts
BACKUP_DIR=/home/dev/notes-hub-bk-test-20261014/pulled
```

The completed linux01 test log records:

```text
PASS pulled seven real test-hub backups; checksums and 0600/0700 permissions
PASS repeated pull reuses unchanged local archives
PASS corrupt retained local checksum rejects pull before publishing
PASS interrupted promotion repairs missing checksum with identical archive
PASS thirty-day retention removes 31-day pair and keeps eight-day off-remote pair
PASS pull staging directories removed
```

The retention test adds explicitly dated copies of a synthetic archive; it
does not wait 30 days or alter the system clock. Existing final-file inodes
stay unchanged after the repeat pull. Further executed checks record:

```text
PASS restricted key rejects upload
PASS restricted key rejects parent directory traversal
PASS restricted key rejects arbitrary shell command
notes-hub backup pull: checksum mismatch: notes-hub-data-20261014T004616Z.tar.gz
PASS corrupt remote checksum rejected; retained pairs unchanged
PASS test-only remote checksum restored
```

An empty remote set also returns nonzero. Initial test-server key ownership
causes authentication failure; correcting it to root ownership and `0600`
allows authentication. Only subsequent forced-command responses and successful
rsync transfers establish the restriction checks.

During the pull tests, linux01 reaches its existing 8 GiB memory limit. The
Proxmox kernel records cgroup OOM kills of its user systemd and D-Bus processes;
ordinary SSH paths then time out. No unrelated process, service, memory limit
or host setting is changed. The completed test log is read through Proxmox,
and a fresh pull is independently rerun inside linux01 as UID/GID 1000 using
`pct exec 104 -- setpriv --reuid=1000 --regid=1000 --init-groups --reset-env`.
That pull exits zero and verifies seven remote backups into `pulled-confirmed`.
The corrupt-remote check also exits zero through this route. All SSH from the
PC uses Windows OpenSSH with `BatchMode=yes`.

## Restore of the off-host copy

The verified linux01 archive and its checksum are streamed through Windows
OpenSSH back to a private docker01 directory. No archive is written on the PC.
The copy is byte-identical to the original docker01 archive. Running
`restore-test.sh` against that returned copy exits zero with this output:

```text
restore-test: PASS archive content and ownership comparison
restore-test: PASS health HTTP 200 notes-hub at http://127.0.0.1:32785/health
restore-test: PASS notes-hub --devices
restore-test: PASS restored page hashes unchanged (3 Markdown pages)
restore-test: PASS restored vault has 3 Markdown pages; synthetic OIDC configuration, agent disabled
restore-test: removed owned test resources notes-hub-bk-test-restore-20261014T005852Z-359953-637
```

## Static and scheduling checks

- `bash -n` and ShellCheck 0.10.0 on linux01 pass for all four repository
  scripts, including the harness, after the final script corrections.
- `systemd-analyze` 257.13 verifies the unmodified VPS units in an isolated
  filesystem root. Executable paths are staged there with an inert
  `docker.service` and required target stubs because linux01 has no Docker.
- Native `systemd-analyze --user verify` passes for the unmodified user unit
  and timer with a test-only drop-in redirecting `ExecStart` to the private
  staged script. Final checks use a private `XDG_RUNTIME_DIR` in the test
  directory, since the console route has no login session. No user unit is
  installed in the real unit directory.
- Both calendar expressions parse: daily `02:30:00 UTC` and `04:00:00` local.
  linux01's current local timezone is UTC. Both timers set `Persistent=true`.
- `git diff --check` passes. Final scope and commit checks are recorded in
  the accompanying completion report.

## Cleanup

The source and both restore runs remove their own containers, volumes and
networks. Final docker01 checks confirm no `notes-hub-bk-test-*` containers,
volumes or networks remain after removing the disposable SSH server. The
two image tags created by this run are removed; Docker build cache is left
alone. Private test directories, synthetic archives, test Docker context and
the dedicated linux01 test key are removed. PC temporary source archives and
test scratch are removed. No detached test process remains.

## Limits and installation decisions

The tests prove stopped-volume archive preservation, root extraction,
numeric ownership, hub startup, the devices CLI, unchanged Markdown page
files, restricted transport, checksum verification and retention. The seeded
Markdown files are not imported into Loro documents or the page index; these
checks do not prove authenticated page sync, OIDC login or real provider
sign-in. The systemd units are statically verified, not started or scheduled
on production. Production data, image compatibility with future migrations
and an actual production restore remain unverified.

The VPS installation and home timer enablement remain pending owner approval.
The owner should confirm linux01's intended local timezone before enabling
its timer. linux01's memory pressure is a separate operational issue and is
not repaired by this backup work.
