#!/usr/bin/env bash
# Run on the Docker host. Stream the stopped volume; never copy individual live DBs.
# Environment: BACKUP_DIR, COMPOSE_FILE, HUB_CONTAINER, HUB_SERVICE, HUB_VOLUME,
# HUB_IMAGE, DOCKER_CONTEXT. The compose-directory .deploy.lock also locks deploy.sh.
set -Eeuo pipefail
umask 077

BACKUP_DIR=${BACKUP_DIR:-/opt/notes-hub/backups}
COMPOSE_FILE=${COMPOSE_FILE:-/opt/notes-hub/compose.yaml}
HUB_CONTAINER=${HUB_CONTAINER:-notes-hub}
HUB_SERVICE=${HUB_SERVICE:-hub}
HUB_VOLUME=${HUB_VOLUME:-notes-hub-data}
HUB_IMAGE=${HUB_IMAGE:-notes-hub:latest}
DOCKER_CONTEXT=${DOCKER_CONTEXT:-default}
docker_cmd=(docker --context "$DOCKER_CONTEXT")
compose=("${docker_cmd[@]}" compose --file "$COMPOSE_FILE")
restart_needed=0
partial_archive=
partial_checksum=
helper="notes-hub-bk-test-create-$(date -u +%Y%m%dT%H%M%SZ)-$$-$RANDOM"
owner="$helper"
helper_active=0

die() { printf 'backup: %s\n' "$*" >&2; exit 1; }
cleanup() {
    local status=$?
    trap - EXIT HUP INT TERM
    set +e
    # A failed/interrupted docker CLI can leave its helper running. Only our label
    # permits deletion; a coincidentally matching pre-existing name is untouched.
    if (( helper_active )) && [[ $("${docker_cmd[@]}" inspect --format '{{index .Config.Labels "notes.hub.backup-owner"}}' "$helper" 2>/dev/null) == "$owner" ]]; then
        "${docker_cmd[@]}" rm -f "$helper" >/dev/null || status=1
    fi
    if (( restart_needed )); then
        printf 'backup: restarting %s\n' "$HUB_CONTAINER" >&2
        "${compose[@]}" start "$HUB_SERVICE" >/dev/null || status=1
        [[ $("${docker_cmd[@]}" inspect --format '{{.State.Running}}' "$HUB_CONTAINER" 2>/dev/null) == true ]] || status=1
    fi
    [[ -z $partial_archive ]] || rm -f -- "$partial_archive" || status=1
    [[ -z $partial_checksum ]] || rm -f -- "$partial_checksum" || status=1
    exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

[[ $# == 0 ]] || die 'usage: notes-hub-backup.sh (configuration is through environment variables)'
for command in docker flock df stat sha256sum gzip mktemp sort tail tr; do
    command -v "$command" >/dev/null || die "required command is missing: $command"
done
[[ -f $COMPOSE_FILE ]] || die "compose file is missing: $COMPOSE_FILE"
compose_dir=$(cd -- "$(dirname -- "$COMPOSE_FILE")" && pwd -P)
lock="$compose_dir/.deploy.lock"
[[ ! -L $lock ]] || die 'refusing a symlink deploy lock'
exec 9>"$lock"
chmod 600 "$lock"
flock -n 9 || die 'a deploy or backup is already running'

[[ ! -L $BACKUP_DIR ]] || die 'refusing a symlink backup directory'
mkdir -p -- "$BACKUP_DIR"
chmod 700 "$BACKUP_DIR"
BACKUP_DIR=$(cd -- "$BACKUP_DIR" && pwd -P)
"${docker_cmd[@]}" image inspect "$HUB_IMAGE" >/dev/null
"${docker_cmd[@]}" volume inspect "$HUB_VOLUME" >/dev/null
if "${docker_cmd[@]}" container inspect "$helper" >/dev/null 2>&1; then
    die 'backup helper name already exists; refusing to reuse it'
fi
hub_id=$("${docker_cmd[@]}" inspect --format '{{.Id}}' "$HUB_CONTAINER")
compose_id=$("${compose[@]}" ps --all --quiet "$HUB_SERVICE")
[[ -n $compose_id && $hub_id == "$compose_id"* ]] || die 'compose service does not identify the configured hub container'
[[ $("${docker_cmd[@]}" inspect --format '{{.State.Running}}' "$HUB_CONTAINER") == true ]] || die 'hub is not running; refusing to change its stopped state'
mount=$("${docker_cmd[@]}" inspect --format '{{range .Mounts}}{{if eq .Destination "/data"}}{{.Type}} {{.Name}}{{end}}{{end}}' "$HUB_CONTAINER")
[[ $mount == "volume $HUB_VOLUME" ]] || die 'configured hub volume does not match the container /data mount'

shopt -s nullglob
archives=()
for archive in "$BACKUP_DIR"/notes-hub-data-*.tar.gz; do
    [[ ${archive##*/} =~ ^notes-hub-data-[0-9]{8}T[0-9]{6}Z\.tar\.gz$ && -f $archive && ! -L $archive ]] && archives+=("$archive")
done
if ((${#archives[@]})); then
    mapfile -t archives < <(printf '%s\n' "${archives[@]}" | LC_ALL=C sort -r)
    last_size=$(stat --format='%s' -- "${archives[0]}")
    free_bytes=$(df --block-size=1 --output=avail -- "$BACKUP_DIR" | tail -n 1 | tr -d '[:space:]')
    [[ $free_bytes =~ ^[0-9]+$ && $last_size =~ ^[0-9]+$ ]] || die 'could not determine free space'
    (( free_bytes >= last_size * 2 )) || die "free space ($free_bytes bytes) is less than twice the last archive ($last_size bytes)"
fi

base="notes-hub-data-$(date -u +%Y%m%dT%H%M%SZ).tar.gz"
archive="$BACKUP_DIR/$base"
[[ ! -e $archive && ! -e $archive.sha256 ]] || die 'archive timestamp already exists; retry in a later second'
partial_archive=$(mktemp "$BACKUP_DIR/.notes-hub-data-archive.XXXXXX")
partial_checksum=$(mktemp "$BACKUP_DIR/.notes-hub-data-checksum.XXXXXX")
# Set this before stop: even a partially successful stop must trigger recovery.
restart_needed=1
printf 'backup: stopping %s\n' "$HUB_CONTAINER"
"${compose[@]}" stop --timeout 60 "$HUB_SERVICE" >/dev/null
[[ $("${docker_cmd[@]}" inspect --format '{{.State.Running}}' "$HUB_CONTAINER") == false ]] || die 'hub did not stop'
# Prevent an accidental archive of a volume still mounted by a second writer.
[[ -z $("${docker_cmd[@]}" ps --quiet --filter "volume=$HUB_VOLUME") ]] || die 'another running container mounts the hub volume'
helper_active=1
"${docker_cmd[@]}" run --rm --name "$helper" --label "notes.hub.backup-owner=$owner" \
    --pull never --network none --user 0 --entrypoint tar \
    --mount "type=volume,src=$HUB_VOLUME,dst=/data,readonly,volume-nocopy" \
    "$HUB_IMAGE" --numeric-owner -C /data -czf - . >"$partial_archive"
helper_active=0
gzip --test "$partial_archive"
digest=$(sha256sum -- "$partial_archive")
printf '%s  %s\n' "${digest%% *}" "$base" >"$partial_checksum"
chmod 600 "$partial_archive" "$partial_checksum"
# Recover serving before publishing or retention. The EXIT trap retries failures.
"${compose[@]}" start "$HUB_SERVICE" >/dev/null
[[ $("${docker_cmd[@]}" inspect --format '{{.State.Running}}' "$HUB_CONTAINER") == true ]] || die 'hub did not restart'
restart_needed=0
mv -- "$partial_archive" "$archive"
partial_archive=
mv -- "$partial_checksum" "$archive.sha256"
partial_checksum=

archives+=("$archive")
mapfile -t archives < <(printf '%s\n' "${archives[@]}" | LC_ALL=C sort -r)
for ((index=7; index<${#archives[@]}; index++)); do
    # Only timestamped archives in our chosen directory, plus their sidecars.
    rm -f -- "${archives[index]}" "${archives[index]}.sha256"
done
printf 'backup: saved %s (gzip verified, sha256 written; newest 7 retained)\n' "$archive"
