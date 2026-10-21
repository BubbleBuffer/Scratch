#!/usr/bin/env bash
# Verify an archive on the Docker host, using ONLY owned disposable resources.
# Usage: restore-test.sh [--allow-empty-vault] /private/path/notes-hub-data-<UTC>.tar.gz
# An archive whose vault has no documents fails unless --allow-empty-vault is given.
# Environment: HUB_IMAGE (notes-hub:latest), DOCKER_CONTEXT (default),
# RESTORE_TMP_DIR (TMPDIR or /tmp). Never uses production compose or its .env.
set -Eeuo pipefail
umask 077
HUB_IMAGE=${HUB_IMAGE:-notes-hub:latest}
DOCKER_CONTEXT=${DOCKER_CONTEXT:-default}
docker_cmd=(docker --context "$DOCKER_CONTEXT")
suffix="$(date -u +%Y%m%dT%H%M%SZ)-$$-$RANDOM"
name="notes-hub-bk-test-restore-$suffix"
volume="$name"
network="$name"
helper="$name-extract"
owner="$name"
temp_dir=
cleanup_allowed=0
volume_created=0
network_created=0
container_created=0

die() { printf 'restore-test: %s\n' "$*" >&2; exit 1; }
owned() {
    [[ $("${docker_cmd[@]}" "$1" inspect --format '{{index .Labels "notes.hub.backup-owner"}}' "$2" 2>/dev/null) == "$owner" ]]
}
cleanup() {
    local status=$?
    trap - EXIT HUP INT TERM
    set +e
    if (( cleanup_allowed )); then
    for container in "$name" "$helper"; do
        if [[ $("${docker_cmd[@]}" inspect --format '{{index .Config.Labels "notes.hub.backup-owner"}}' "$container" 2>/dev/null) == "$owner" ]]; then
            "${docker_cmd[@]}" rm -f "$container" >/dev/null || status=1
        elif [[ $container == "$name" ]] && (( container_created )); then
            printf 'restore-test: could not verify ownership of test container during cleanup\n' >&2
            status=1
        fi
    done
    if owned volume "$volume"; then
        "${docker_cmd[@]}" volume rm "$volume" >/dev/null || status=1
    elif (( volume_created )); then
        printf 'restore-test: could not verify ownership of test volume during cleanup\n' >&2
        status=1
    fi
    if owned network "$network"; then
        "${docker_cmd[@]}" network rm "$network" >/dev/null || status=1
    elif (( network_created )); then
        printf 'restore-test: could not verify ownership of test network during cleanup\n' >&2
        status=1
    fi
    fi
    if [[ -n $temp_dir && -d $temp_dir && ${temp_dir##*/} == "$name."* ]]; then
        rm -rf -- "$temp_dir" || status=1
    fi
    if (( cleanup_allowed && status == 0 )); then
        printf 'restore-test: removed owned test resources %s\n' "$name"
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

allow_empty_vault=0
args=()
for argument in "$@"; do
    if [[ $argument == --allow-empty-vault ]]; then allow_empty_vault=1; else args+=("$argument"); fi
done
[[ ${#args[@]} == 1 ]] || die 'usage: restore-test.sh [--allow-empty-vault] /private/path/archive.tar.gz'
for command in docker sha256sum gzip mktemp curl cmp cat grep; do
    command -v "$command" >/dev/null || die "required command is missing: $command"
done
archive=${args[0]}
[[ -f $archive && ! -L $archive ]] || die 'archive must be a regular file, not a symlink'
archive_dir=$(cd -- "$(dirname -- "$archive")" && pwd -P)
base=${archive##*/}
[[ $base =~ ^[A-Za-z0-9._-]+\.tar\.gz$ ]] || die 'archive basename must use letters, digits, dot, underscore or hyphen'
archive="$archive_dir/$base"
[[ -f $archive.sha256 && ! -L $archive.sha256 ]] || die 'regular sibling .sha256 checksum is required'
# Compare one expected checksum line; never let an untrusted checksum choose paths.
digest=$(sha256sum -- "$archive")
expected="${digest%% *}  $base"
printf '%s\n' "$expected" | cmp --silent -- "$archive.sha256" - || die 'SHA-256 checksum mismatch or invalid sidecar format'
gzip --test "$archive"
"${docker_cmd[@]}" image inspect "$HUB_IMAGE" >/dev/null
for resource in volume network container; do
    if "${docker_cmd[@]}" "$resource" inspect "$name" >/dev/null 2>&1; then
        die "test $resource name already exists; refusing to reuse it"
    fi
done
if "${docker_cmd[@]}" container inspect "$helper" >/dev/null 2>&1; then
    die 'test helper name already exists; refusing to reuse it'
fi
temp_dir=$(mktemp -d "${RESTORE_TMP_DIR:-${TMPDIR:-/tmp}}/$name.XXXXXX")
chmod 700 "$temp_dir"
cleanup_allowed=1
"${docker_cmd[@]}" volume create --label "notes.hub.backup-owner=$owner" "$volume" >/dev/null
volume_created=1
"${docker_cmd[@]}" network create --label "notes.hub.backup-owner=$owner" "$network" >/dev/null
network_created=1
"${docker_cmd[@]}" run --rm -i --name "$helper" --label "notes.hub.backup-owner=$owner" \
    --pull never --network none --user 0 --entrypoint tar \
    --mount "type=volume,src=$volume,dst=/data,volume-nocopy" \
    "$HUB_IMAGE" --numeric-owner --same-owner -C /data -xzf - <"$archive"
# GNU tar checks archive entries' bytes, numeric ownership, permissions and timestamps
# before application startup can legitimately update SQLite/WAL or token stores.
"${docker_cmd[@]}" run --rm -i --name "$helper" --label "notes.hub.backup-owner=$owner" \
    --pull never --network none --user 0 --entrypoint tar \
    --mount "type=volume,src=$volume,dst=/data,readonly,volume-nocopy" \
    "$HUB_IMAGE" --numeric-owner -C /data --compare --gzip --file - <"$archive"
printf 'restore-test: PASS archive content and ownership comparison\n'

# Hash the content of record (.vault/docs, .vault/logs), the markdown export, the
# token files and hub.toml, from the archive (extracted in a helper container's
# writable layer, removed with it) and from the restored volume. Both
# sides are taken before the hub starts, because a starting hub may rewrite index
# and lock files. Manifests stay in the private temporary directory and are never printed.
# shellcheck disable=SC2016 # $1 is the directory argument of the inner sh.
manifest_script='cd "$1" && { for p in vault/.vault/docs vault/.vault/logs vault/pages vault/.vault/hub; do if [ -d "$p" ]; then find "$p" -type f -exec sha256sum -- {} \; ; fi; done; if [ -f hub.toml ]; then sha256sum -- hub.toml; fi; } | LC_ALL=C sort'
"${docker_cmd[@]}" run --rm -i --name "$helper" --label "notes.hub.backup-owner=$owner" \
    --pull never --network none --user 0 --entrypoint sh \
    "$HUB_IMAGE" -ec 'mkdir /archive && tar -C /archive --numeric-owner -xzf - && '"$manifest_script" sh /archive \
    <"$archive" >"$temp_dir/archive.sha256"
"${docker_cmd[@]}" run --rm --name "$helper" --label "notes.hub.backup-owner=$owner" \
    --pull never --network none --user 0 --entrypoint sh \
    --mount "type=volume,src=$volume,dst=/data,readonly,volume-nocopy" \
    "$HUB_IMAGE" -ec "$manifest_script" sh /data >"$temp_dir/restored.sha256"
cmp --silent "$temp_dir/archive.sha256" "$temp_dir/restored.sha256" || die 'restored vault files differ from the archive'
count_matching() { grep -c -- "$1" "$temp_dir/archive.sha256" || true; }
docs=$(count_matching '  vault/\.vault/docs/')
logs=$(count_matching '  vault/\.vault/logs/')
pages=$(count_matching '  vault/pages/')
tokens=$(count_matching '  vault/\.vault/hub/')
config=$(count_matching '  hub\.toml$')
printf 'restore-test: PASS restored file SHA-256 equals archive (docs=%s logs=%s pages=%s hub-token-files=%s hub.toml=%s)\n' \
    "$docs" "$logs" "$pages" "$tokens" "$config"
if (( docs == 0 )); then
    (( allow_empty_vault )) || die 'vault has no documents in vault/.vault/docs; pass --allow-empty-vault only for an archive known to be empty'
    printf 'restore-test: WARN: vault has no documents; continuing because --allow-empty-vault was given\n'
fi

# A separate empty config in container /tmp prevents parsing the restored
# production TOML. The restored volume's config and credentials stay untouched.
"${docker_cmd[@]}" run -d --name "$name" --label "notes.hub.backup-owner=$owner" \
    --pull never --network "$network" --publish '127.0.0.1::8080' \
    --memory 1g --pids-limit 512 --security-opt no-new-privileges:true \
    --mount "type=volume,src=$volume,dst=/data,volume-nocopy" \
    --env NOTES_HUB_LISTEN=0.0.0.0:8080 --env NOTES_HUB_DATA_DIR=/data \
    --env NOTES_HUB_PUBLIC_URL=http://127.0.0.1 \
    --env NOTES_HUB_TRUST_PROXY=false --env NOTES_HUB_AGENT_ENABLED=false \
    --env NOTES_HUB_OIDC_ISSUER=https://restore-test.invalid/application/o/notes-hub/ \
    --env NOTES_HUB_OIDC_CLIENT_ID=restore-test --env NOTES_HUB_OIDC_CLIENT_SECRET=synthetic-secret \
    --env NOTES_HUB_OIDC_ALLOWED_USERS= --env NOTES_HUB_OIDC_ALLOWED_GROUPS=restore-test \
    --env 'NOTES_HUB_OIDC_SCOPES=openid email profile' \
    --entrypoint sh "$HUB_IMAGE" -c 'umask 077; touch /tmp/restore-test.toml; exec /usr/bin/tini -- notes-hub --config /tmp/restore-test.toml' >/dev/null
container_created=1
port=$("${docker_cmd[@]}" inspect --format '{{with index .NetworkSettings.Ports "8080/tcp"}}{{(index . 0).HostPort}}{{end}}' "$name")
[[ $port =~ ^[0-9]+$ ]] || die 'Docker did not allocate a local port'
url="http://127.0.0.1:$port/health"
ready=0
printf notes-hub >"$temp_dir/expected-health"
deadline=$((SECONDS + 120))
while (( SECONDS < deadline )); do
    if status=$(curl --noproxy '*' --silent --show-error --fail --max-time 2 \
        --output "$temp_dir/health" --write-out '%{http_code}' "$url" 2>/dev/null) \
        && [[ $status == 200 ]] && cmp --silent "$temp_dir/expected-health" "$temp_dir/health" \
        && [[ $("${docker_cmd[@]}" inspect --format '{{.State.Health.Status}}' "$name") == healthy ]]; then
        ready=1
        break
    fi
    [[ $("${docker_cmd[@]}" inspect --format '{{.State.Running}}' "$name") == true ]] || die 'restored hub exited before becoming healthy'
    sleep 2
done
(( ready )) || die 'restored hub did not return healthy HTTP 200 notes-hub within 2 minutes'
printf 'restore-test: PASS health HTTP 200 notes-hub at %s\n' "$url"
"${docker_cmd[@]}" exec "$name" notes-hub --config /tmp/restore-test.toml --devices >/dev/null
printf 'restore-test: PASS notes-hub --devices\n'
printf 'restore-test: PASS restored vault verified; synthetic OIDC configuration, agent disabled\n'
