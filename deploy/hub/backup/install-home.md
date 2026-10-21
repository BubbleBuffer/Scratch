# Home backup pull installation — prepared, not installed

This kit is for **linux01**, using `dev`'s systemd **user** service and timer.
The timer runs at **04:00 UTC** (linux01's timezone) and catches missed
runs with `Persistent=true`. Check `timedatectl` before enabling it. The pull
retains archives from the last 30 days based on their UTC filename timestamp,
so a repeated pull does not extend an archive's life. Files are `0600` in a
`0700` directory, default `/home/dev/backups/notes-hub`.

No timer has been installed or enabled by preparing this kit. VPS installation
and key authorization remain pending owner approval; see
[install-vps.md](install-vps.md).

## Dependencies and dedicated key

As root on linux01, install missing dependencies:

```sh
apt-get update
apt-get install --no-install-recommends bash coreutils util-linux gzip tar rsync openssh-client
install -d -o dev -g dev -m 700 /home/dev/.ssh
install -d -o dev -g dev -m 700 /home/dev/backups
install -d -o dev -g dev -m 700 /home/dev/backups/notes-hub
```

As **dev on linux01**, create the key if that exact pathname is not already in
use. Preserve an existing key; never overwrite it without investigating:

```sh
if [ ! -e /home/dev/.ssh/notes-hub-backup_ed25519 ] && [ ! -e /home/dev/.ssh/notes-hub-backup_ed25519.pub ]; then
    ssh-keygen -t ed25519 -N '' -C notes-hub-backup-pull -f /home/dev/.ssh/notes-hub-backup_ed25519
else
    printf 'Existing backup key found; inspect and preserve it.\n'
fi
chmod 600 /home/dev/.ssh/notes-hub-backup_ed25519
chmod 644 /home/dev/.ssh/notes-hub-backup_ed25519.pub
```

The empty passphrase permits this bounded unattended rsync pull. Its public
key is authorized on the VPS only with the forced read-only directory
restriction shown in `install-vps.md`. The private key stays on linux01. No
home-server credential or home-side authorized key is installed on the VPS.

## Pin the VPS host key out of band

Obtain the VPS's **public** Ed25519 host key or its SHA-256 fingerprint through
an existing trusted administrative channel (for example the VPS console or an
already verified administrative SSH session). On the VPS, the owner can read
the public key and fingerprint with:

```sh
cat /etc/ssh/ssh_host_ed25519_key.pub
ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub -E sha256
```

As dev on linux01, write a dedicated known_hosts entry using the verified
public key, with the same host token that `BACKUP_HOST` will use. For the
deployment IP:

```text
VPS_IP ssh-ed25519 VERIFIED_VPS_HOST_PUBLIC_KEY
```

Save it as `/home/dev/.ssh/notes-hub-backup_known_hosts`, owned by dev, mode
`0600`. Run `ssh-keygen -lf /home/dev/.ssh/notes-hub-backup_known_hosts -E
sha256` and compare the fingerprint to the trusted value. `ssh-keyscan` can
collect a candidate, but its output alone does not authenticate the VPS; do
not accept it without the out-of-band comparison. Never copy a host private
key. The pull enforces `StrictHostKeyChecking=yes`, only this dedicated
known_hosts file and only the dedicated client key. It ignores ordinary SSH
configuration and does not accept a new key automatically.

## Copy and install the exact files

From this checkout, copy these files using the owner's ordinary linux01
access into a private staging directory owned by dev:

```text
deploy/hub/backup/pull-notes-hub-backup.sh
deploy/hub/backup/pull-notes-hub-backup.service
deploy/hub/backup/pull-notes-hub-backup.timer
deploy/hub/backup/install-home.md
```

Example PowerShell commands (when installation is requested):

```powershell
& C:\Windows\System32\OpenSSH\ssh.exe -o BatchMode=yes linux01 'install -d -m 700 /home/dev/notes-hub-backup-kit'
& C:\Windows\System32\OpenSSH\scp.exe -o BatchMode=yes deploy/hub/backup/pull-notes-hub-backup.sh deploy/hub/backup/pull-notes-hub-backup.service deploy/hub/backup/pull-notes-hub-backup.timer deploy/hub/backup/install-home.md linux01:/home/dev/notes-hub-backup-kit/
```

As **dev** on linux01:

```sh
install -d -m 700 /home/dev/.local/libexec/notes-hub
install -d -m 700 /home/dev/.config/notes-hub
install -d -m 700 /home/dev/.config/systemd/user
install -m 755 /home/dev/notes-hub-backup-kit/pull-notes-hub-backup.sh /home/dev/.local/libexec/notes-hub/pull-notes-hub-backup.sh
install -m 644 /home/dev/notes-hub-backup-kit/pull-notes-hub-backup.service /home/dev/.config/systemd/user/pull-notes-hub-backup.service
install -m 644 /home/dev/notes-hub-backup-kit/pull-notes-hub-backup.timer /home/dev/.config/systemd/user/pull-notes-hub-backup.timer
if [ ! -e /home/dev/.config/notes-hub/pull-backup.env ]; then
    install -m 600 /dev/null /home/dev/.config/notes-hub/pull-backup.env
fi
chmod 600 /home/dev/.config/notes-hub/pull-backup.env
```

Edit `/home/dev/.config/notes-hub/pull-backup.env` to configure the verified VPS host:

```ini
BACKUP_HOST=root@VPS_IP
BACKUP_SSH_PORT=22
BACKUP_DIR=/home/dev/backups/notes-hub
BACKUP_SSH_KEY=/home/dev/.ssh/notes-hub-backup_ed25519
BACKUP_KNOWN_HOSTS=/home/dev/.ssh/notes-hub-backup_known_hosts
RETENTION_DAYS=30
```

The other defaults match these values; `BACKUP_HOST` has no default and must be set. It accepts a hostname or IP,
optionally prefixed with a user; `BACKUP_SSH_PORT` accepts ports 1–65535. Use a literal DNS
name or IP, since the pull ignores SSH aliases. Keep the known_hosts token in
sync with it. A nonstandard port uses a known_hosts token such as
`[VPS_IP]:2222`. If `BACKUP_DIR` changes, create it with dev ownership
and `0700`. The user manager expands `%h` in the service to dev's home and
reads the private environment file. The unit avoids mount namespace
restrictions that require privileges unavailable to a normal user manager.

For unattended operation after boot and logout, the user manager must linger.
Check `loginctl show-user dev -p Linger`. If it is not enabled, have the owner
enable it with `loginctl enable-linger dev` as root when installation is
authorized. This kit does not enable lingering or install a timer during
preparation.

## Verify before enabling

After the approved VPS key installation and a successful VPS backup, run as
**dev** on linux01:

```sh
bash -n /home/dev/.local/libexec/notes-hub/pull-notes-hub-backup.sh
systemd-analyze --user verify /home/dev/.config/systemd/user/pull-notes-hub-backup.service /home/dev/.config/systemd/user/pull-notes-hub-backup.timer
systemctl --user daemon-reload
systemctl --user start pull-notes-hub-backup.service
systemctl --user status pull-notes-hub-backup.service --no-pager
journalctl --user -u pull-notes-hub-backup.service -n 50 --no-pager
systemctl --user enable --now pull-notes-hub-backup.timer
systemctl --user list-timers pull-notes-hub-backup.timer --no-pager
```

The script takes a nonblocking lock, stages transfers privately, verifies the
strict single-line checksum manifest and gzip/tar structure for **every**
archive, then promotes verified files into the final directory. Rsync reuses
unchanged archives locally; new archives are transferred. A remote replacement
with the same timestamped name but different contents is rejected. Retained
local archives are verified before any promotion too. An interrupted promotion
that left an archive without its manifest is repaired only when the incoming
verified archive has identical contents. An incomplete transfer, missing/malformed
manifest, corrupt archive, empty remote backup set, SSH failure or contention
returns nonzero. Incoming failures do not promote corrupt files or prune
existing backups. Check the journal and archive ownership/modes after the
first run; timer enablement by itself is not proof of a successful pull.
