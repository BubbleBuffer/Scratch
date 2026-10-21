# VPS backup installation — pending approval

These are installation instructions, not a record of an installation. **Do not
run them on the VPS until the owner approves the concrete kit.** No VPS change
or timer enablement is part of this preparation.

The root oneshot requires a running hub, stops it, archives the complete
`notes-hub-data` volume (including provider sign-ins), and restarts it.
Archives and checksums live in `/opt/notes-hub/backups`, owned by root,
with directory mode `0700` and file mode `0600`. The helper image is the already
installed `notes-hub:latest`; no backup helper image is downloaded. The newest
seven archive/checksum pairs are retained on the VPS. The timer
runs every day at **02:30 UTC**, with `Persistent=true` to catch a missed run.
The script shares `/opt/notes-hub/.deploy.lock` with CI deployments.
The service uses a private `0700` runtime Docker configuration directory at
`/run/notes-hub-backup`, avoiding the owner's credential-bearing Docker home.

## Copy and install the kit after approval

Copy these four repository files into a private temporary directory on the VPS
using the owner's existing administrative access:

```text
deploy/hub/backup/notes-hub-backup.sh
deploy/hub/backup/notes-hub-backup.service
deploy/hub/backup/notes-hub-backup.timer
deploy/hub/backup/install-server.md
```

For example, from PowerShell in this checkout, **after approval**:

```powershell
& C:\Windows\System32\OpenSSH\ssh.exe -o BatchMode=yes <server> 'install -d -m 700 /root/notes-hub-backup-kit'
& C:\Windows\System32\OpenSSH\scp.exe -o BatchMode=yes deploy/hub/backup/notes-hub-backup.sh deploy/hub/backup/notes-hub-backup.service deploy/hub/backup/notes-hub-backup.timer deploy/hub/backup/install-server.md <server>:/root/notes-hub-backup-kit/
```

As root on the VPS, install dependencies and the exact files:

```sh
apt-get update
apt-get install --no-install-recommends bash coreutils util-linux gzip tar rsync python3 openssh-server
test -x /usr/bin/rrsync
/usr/bin/rrsync -h
docker compose version
docker image inspect notes-hub:latest >/dev/null
install -d -o root -g root -m 755 /usr/local/libexec/notes-hub
install -d -o root -g root -m 700 /opt/notes-hub/backups
install -d -o root -g root -m 700 /etc/notes-hub
install -o root -g root -m 755 /root/notes-hub-backup-kit/notes-hub-backup.sh /usr/local/libexec/notes-hub/notes-hub-backup.sh
install -o root -g root -m 644 /root/notes-hub-backup-kit/notes-hub-backup.service /etc/systemd/system/notes-hub-backup.service
install -o root -g root -m 644 /root/notes-hub-backup-kit/notes-hub-backup.timer /etc/systemd/system/notes-hub-backup.timer
bash -n /usr/local/libexec/notes-hub/notes-hub-backup.sh
systemd-analyze verify /etc/systemd/system/notes-hub-backup.service /etc/systemd/system/notes-hub-backup.timer
```

`rrsync` must be installed as an executable at **`/usr/bin/rrsync`**. The rsync
package supplies it on current Debian; its Python interpreter is an explicit
dependency above because a minimal rsync installation can omit it. If the
executable or `rrsync -h` check fails, stop and resolve the
package/path before authorizing the key; do not replace the restriction with a
general shell key. Docker Engine and the Compose plugin are existing VPS
dependencies and are not installed or reconfigured by these commands.

Defaults are `/opt/notes-hub/compose.yaml`, volume `notes-hub-data`, container
`notes-hub`, service `hub`, local Docker context `default`, and image
`notes-hub:latest`. Optional non-secret overrides belong in a root-owned `0600`
`/etc/notes-hub/backup.env`. If paths change, adjust the unit's
`ReadWritePaths` and the forced command below to match; changing only the script
environment does not change systemd's filesystem boundary.

## Authorize only read-only backup pulls

Generate the dedicated key **on the Linux machine as dev**, following
[install-home.md](install-home.md). Bring only that key's public `.pub` line to
the VPS. The private key stays on the Linux machine and is never copied to the VPS or this
PC.

Preserve all existing root SSH keys. Add exactly one line to
`/root/.ssh/authorized_keys`, replacing the public-key placeholder with the
public line generated on the Linux machine:

```text
restrict,command="/usr/bin/rrsync -ro /opt/notes-hub/backups" ssh-ed25519 PUBLIC_KEY_FROM_LINUX01 notes-hub-backup-pull
```

Set `/root/.ssh` to root ownership and mode `0700`, and `authorized_keys` to
root ownership and mode `0600`. The `restrict` option disables forwarding,
PTYs and user rc files; `rrsync -ro` confines the key to read-only rsync beneath
the backup directory. It does not grant a shell, Docker access, or access to
`.env`, the live volume, or the rest of the VPS. Keep this key separate from
the existing CI deploy key and the owner's administrative keys.
The restriction follows [rsync's rrsync manual](https://download.samba.org/pub/rsync/rrsync.1).

The remote source used by the pull is **`root@HOST:./`**, relative to rrsync's
restricted directory. An absolute `/opt/notes-hub/backups` rsync source would
be interpreted inside that restriction and is incorrect.

## Enable and verify after approval

The initial service run deliberately stops and restarts the production hub.
Run it in the approved maintenance window, then inspect its result before
enabling the timer:

```sh
systemctl daemon-reload
systemctl start notes-hub-backup.service
systemctl status notes-hub-backup.service --no-pager
journalctl -u notes-hub-backup.service -n 50 --no-pager
systemctl enable --now notes-hub-backup.timer
systemctl list-timers notes-hub-backup.timer --no-pager
```

A failed service must be investigated; do not treat timer enablement as a
successful backup. Confirm the resulting timestamped archive and its sibling
`.tar.gz.sha256` are root-owned `0600`, and verify the home-side pull before
depending on this as an off-host backup. The backup archive contains
credentials. The deployment `.env` is not part of the volume archive; retain
it separately through the owner's private credential backup procedure.

The separate `restore-test.sh` is a disposable restore verifier, not a
production restore automation unit. Run it **on the Docker host**,
with a private test archive and a Docker context that points to its local
daemon (for example the named `<docker-host>` context). Its health check uses the
host's localhost port, so running it on another machine requires separate
port forwarding and is not covered by these instructions. It is not
installed on the VPS by these commands.
