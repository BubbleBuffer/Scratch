#!/usr/bin/env bash
# Pull via a dedicated root key forced to rrsync -ro on the VPS. No push or
# arbitrary remote command is needed. Run as dev on linux01, never on the VPS.
set -Eeuo pipefail
umask 077
export LC_ALL=C

BACKUP_DIR=${BACKUP_DIR:-/home/dev/backups/notes-hub}
BACKUP_HOST=${BACKUP_HOST:?set BACKUP_HOST (user@VPS address) in pull-backup.env}
BACKUP_SSH_PORT=${BACKUP_SSH_PORT:-22}
BACKUP_SSH_KEY=${BACKUP_SSH_KEY:-/home/dev/.ssh/notes-hub-backup_ed25519}
BACKUP_KNOWN_HOSTS=${BACKUP_KNOWN_HOSTS:-/home/dev/.ssh/notes-hub-backup_known_hosts}
RETENTION_DAYS=${RETENTION_DAYS:-30}

die() { printf 'notes-hub backup pull: %s\n' "$*" >&2; exit 1; }
for tool in rsync ssh sha256sum flock gzip tar date mktemp cmp realpath; do
    command -v "$tool" >/dev/null || die "missing dependency: $tool"
done
[[ $BACKUP_DIR == /* && ! -L $BACKUP_DIR ]] || die 'BACKUP_DIR must be an absolute, non-symlink directory'
[[ $BACKUP_HOST =~ ^[A-Za-z0-9_.@:-]+$ && $BACKUP_HOST != -* ]] || die 'invalid BACKUP_HOST'
[[ $BACKUP_SSH_PORT =~ ^[0-9]{1,5}$ ]] || die 'BACKUP_SSH_PORT must be an integer from 1 to 65535'
(( 10#$BACKUP_SSH_PORT >= 1 && 10#$BACKUP_SSH_PORT <= 65535 )) || die 'BACKUP_SSH_PORT must be an integer from 1 to 65535'
[[ $RETENTION_DAYS =~ ^[1-9][0-9]*$ ]] || die 'RETENTION_DAYS must be a positive integer'
[[ -f $BACKUP_SSH_KEY && ! -L $BACKUP_SSH_KEY ]] || die 'dedicated SSH private key is missing or is a symlink'
[[ -s $BACKUP_KNOWN_HOSTS && ! -L $BACKUP_KNOWN_HOSTS ]] || die 'pinned known_hosts file is missing or is a symlink'
mkdir -p -- "$BACKUP_DIR"
BACKUP_DIR=$(realpath -e -- "$BACKUP_DIR")
[[ -O $BACKUP_DIR ]] || die 'backup directory must belong to the current user'
chmod 700 -- "$BACKUP_DIR"
[[ ! -L $BACKUP_DIR/.pull.lock ]] || die 'lock file is a symlink'
exec 9>"$BACKUP_DIR/.pull.lock"
chmod 600 -- "$BACKUP_DIR/.pull.lock"
flock -n 9 || die 'another backup pull is running'

stage=$(mktemp -d "$BACKUP_DIR/.pull-stage.XXXXXXXX")
cleanup() { rm -rf -- "$stage"; }
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# -F /dev/null deliberately ignores ordinary SSH identities/configuration.
# The dedicated known_hosts file must be pinned through a trusted channel.
printf -v ssh_command '%q ' ssh -F /dev/null -p "$BACKUP_SSH_PORT" -i "$BACKUP_SSH_KEY" \
    -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=yes \
    -o "UserKnownHostsFile=$BACKUP_KNOWN_HOSTS" -o GlobalKnownHostsFile=/dev/null \
    -o ConnectTimeout=20 -o ServerAliveInterval=30 -o ServerAliveCountMax=3
# rrsync sees ./ relative to /opt/notes-hub/backups, never an absolute VPS path.
# Hard links reuse unchanged archives; rsync transfers new or changed files
# into the private stage. It never writes through a final archive's pathname.
rsync -rt --timeout=300 --chmod=D0700,F0600 --link-dest="$BACKUP_DIR" \
    --include='/notes-hub-data-*.tar.gz' --include='/notes-hub-data-*.tar.gz.sha256' \
    --exclude='*' -e "$ssh_command" -- "$BACKUP_HOST:./" "$stage/"

shopt -s nullglob
archive_epoch() {
    local name=$1 stamp canonical epoch
    [[ $name =~ ^notes-hub-data-([0-9]{8}T[0-9]{6}Z)\.tar\.gz$ ]] || die "invalid archive name: $name"
    stamp=${BASH_REMATCH[1]}
    canonical=$(date -u -d "${stamp:0:4}-${stamp:4:2}-${stamp:6:2} ${stamp:9:2}:${stamp:11:2}:${stamp:13:2} UTC" +%Y%m%dT%H%M%SZ) \
        || die "invalid archive date: $name"
    [[ $canonical == "$stamp" ]] || die "invalid archive date: $name"
    epoch=$(date -u -d "${stamp:0:4}-${stamp:4:2}-${stamp:6:2} ${stamp:9:2}:${stamp:11:2}:${stamp:13:2} UTC" +%s)
    printf '%s\n' "$epoch"
}
verify_pair() {
    local archive=$1 name manifest checksum_line expected actual
    name=${archive##*/}
    archive_epoch "$name" >/dev/null
    manifest=$archive.sha256
    [[ -f $archive && ! -L $archive && -f $manifest && ! -L $manifest ]] || die "incomplete or non-regular backup: $name"
    IFS= read -r checksum_line < "$manifest" || die "checksum lacks final newline: $name"
    [[ $checksum_line =~ ^([0-9a-f]{64})\ \ (.*)$ && ${BASH_REMATCH[2]} == "$name" ]] || die "invalid checksum manifest: $name"
    expected=${BASH_REMATCH[1]}
    printf '%s\n' "$checksum_line" | cmp -s - "$manifest" || die "checksum must contain exactly one LF-terminated line: $name"
    actual=$(sha256sum -- "$archive")
    [[ ${actual%% *} == "$expected" ]] || die "checksum mismatch: $name"
    gzip --test -- "$archive" || die "invalid gzip archive: $name"
    tar -tzf "$archive" >/dev/null || die "invalid tar archive: $name"
}

archives=("$stage"/notes-hub-data-*.tar.gz)
manifests=("$stage"/notes-hub-data-*.tar.gz.sha256)
(( ${#archives[@]} > 0 )) || die 'remote directory contains no complete backups'
(( ${#archives[@]} == ${#manifests[@]} )) || die 'remote archive/checksum count mismatch'
for manifest in "${manifests[@]}"; do
    [[ -f ${manifest%.sha256} && ! -L $manifest ]] || die "orphan or non-regular checksum: ${manifest##*/}"
done
for archive in "${archives[@]}"; do verify_pair "$archive"; done

# Validate existing history before changing final paths. Timestamped backups
# are immutable: a different, valid remote archive must not replace that date.
local_archives=("$BACKUP_DIR"/notes-hub-data-*.tar.gz)
local_manifests=("$BACKUP_DIR"/notes-hub-data-*.tar.gz.sha256)
for manifest in "${local_manifests[@]}"; do
    [[ -f ${manifest%.sha256} && ! -L $manifest ]] || die "orphan or non-regular local checksum: ${manifest##*/}"
done
for archive in "${local_archives[@]}"; do
    name=${archive##*/}
    [[ -f $archive && ! -L $archive ]] || die "non-regular local backup: $name"
    if [[ -e $archive.sha256 || -L $archive.sha256 ]]; then
        verify_pair "$archive"
    else
        # Recover an interrupted previous promotion only when the incoming
        # verified archive is byte-identical to the final archive left behind.
        if [[ ! -f $stage/$name ]] || ! cmp -s -- "$archive" "$stage/$name"; then
            die "local backup is missing its checksum: $name"
        fi
    fi
done
for archive in "${archives[@]}"; do
    name=${archive##*/}
    if [[ -e $BACKUP_DIR/$name || -L $BACKUP_DIR/$name ]]; then
        if [[ ! -f $BACKUP_DIR/$name || -L $BACKUP_DIR/$name ]] \
            || ! cmp -s -- "$archive" "$BACKUP_DIR/$name"; then
            die "remote backup differs from immutable local backup: $name"
        fi
    fi
done

# All incoming pairs have passed. A crash between these two renames leaves a
# valid archive and can be repaired by the next staged pull of the same pair.
for archive in "${archives[@]}"; do
    name=${archive##*/}
    [[ ! -L $BACKUP_DIR/$name && ! -L $BACKUP_DIR/$name.sha256 ]] || die "local backup path is a symlink: $name"
    chmod 600 -- "$archive" "$archive.sha256"
    if [[ $archive -ef $BACKUP_DIR/$name ]]; then
        rm -- "$archive"
    else
        mv -f -- "$archive" "$BACKUP_DIR/$name"
    fi
    if [[ $archive.sha256 -ef $BACKUP_DIR/$name.sha256 ]]; then
        rm -- "$archive.sha256"
    else
        mv -f -- "$archive.sha256" "$BACKUP_DIR/$name.sha256"
    fi
done

# Incoming and retained local pairs have passed. Retention starts from the
# archive's UTC creation name, never filesystem mtime.
cutoff=$(date -u -d "$RETENTION_DAYS days ago" +%s)
local_archives=("$BACKUP_DIR"/notes-hub-data-*.tar.gz)
local_manifests=("$BACKUP_DIR"/notes-hub-data-*.tar.gz.sha256)
(( ${#local_archives[@]} == ${#local_manifests[@]} )) || die 'local archive/checksum count mismatch'
for archive in "${local_archives[@]}"; do
    chmod 600 -- "$archive" "$archive.sha256"
    if (( $(archive_epoch "${archive##*/}") < cutoff )); then
        rm -- "$archive" "$archive.sha256"
    fi
done
printf 'notes-hub backup pull: verified %s remote backups; retained the last %s days in %s\n' "${#archives[@]}" "$RETENTION_DAYS" "$BACKUP_DIR"
