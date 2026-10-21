#!/usr/bin/env bash
# On docker01: test-docker01.sh /absolute/private/scratch
# The caller supplies its temporary DOCKER_CONFIG and a built HUB_IMAGE. The image
# must include the agent MCP transport (notes-app commit cc50c81 or later): the
# harness seeds its pages through /mcp, so the vault holds real Loro documents.
# Keeps synthetic archives in BACKUP_DIR for the separate linux01 pull check.
# TEST_RETENTION=0 skips the eight-backup retention exercise.
set -Eeuo pipefail
umask 077
DOCKER_CONTEXT=${DOCKER_CONTEXT:-docker01}
HUB_IMAGE=${HUB_IMAGE:-notes-hub:bk-test2}
[[ $DOCKER_CONTEXT == docker01 ]] || { echo 'test: DOCKER_CONTEXT must be docker01' >&2; exit 2; }
docker_cmd=(docker --context docker01)
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source_name=notes-hub-bk-test-src
work=
owner=
cleanup_allowed=0
volume_created=0
network_created=0
container_created=0
seed_name=
fake_archive=

die() { printf 'test: %s\n' "$*" >&2; exit 1; }
label() { "${docker_cmd[@]}" "$1" inspect --format '{{index .Labels "notes.hub.backup-test-owner"}}' "$2" 2>/dev/null; }
cleanup() {
    local status=$?
    trap - EXIT HUP INT TERM
    set +e
    if (( cleanup_allowed )); then
        for container in "$source_name" "$seed_name"; do
            if [[ $("${docker_cmd[@]}" container inspect --format '{{index .Config.Labels "notes.hub.backup-test-owner"}}' "$container" 2>/dev/null) == "$owner" ]]; then
                "${docker_cmd[@]}" rm -f "$container" >/dev/null || status=1
            elif [[ $container == "$source_name" ]] && (( container_created )); then
                printf 'test: unable to verify source container ownership during cleanup\n' >&2
                status=1
            fi
        done
        if [[ $(label volume "$source_name") == "$owner" ]]; then
            "${docker_cmd[@]}" volume rm "$source_name" >/dev/null || status=1
        elif (( volume_created )); then status=1; fi
        if [[ $(label network "$source_name") == "$owner" ]]; then
            "${docker_cmd[@]}" network rm "$source_name" >/dev/null || status=1
        elif (( network_created )); then status=1; fi
    fi
    [[ -z $fake_archive ]] || rm -f -- "$fake_archive" || status=1
    if [[ -n $work && -d $work && ${work##*/} == notes-hub-bk-test-harness.* ]]; then
        rm -rf -- "$work" || status=1
    fi
    if (( status == 0 )); then printf 'test: PASS owned source resources removed; synthetic backups retained in %s\n' "$BACKUP_DIR"; fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

[[ $# == 1 && $1 == /* && $1 != / ]] || die 'usage: test-docker01.sh /absolute/private/scratch'
for command in docker curl cmp tar sha256sum gzip stat df truncate mktemp sort tail tr; do
    command -v "$command" >/dev/null || die "required command is missing: $command"
done
[[ -x /usr/bin/docker ]] || die '/usr/bin/docker is required for the forwarding failure shim'
[[ ${TEST_RETENTION:-1} == 0 || ${TEST_RETENTION:-1} == 1 ]] || die 'TEST_RETENTION must be 0 or 1'
mkdir -p -- "$1"
test_root=$(cd -- "$1" && pwd -P)
chmod 700 "$test_root"
BACKUP_DIR=${BACKUP_DIR:-$test_root/backups}
[[ $BACKUP_DIR == "$test_root/"* && ! -L $BACKUP_DIR ]] || die 'BACKUP_DIR must be a non-symlink directory inside the supplied scratch directory'
mkdir -p -- "$BACKUP_DIR"
chmod 700 "$BACKUP_DIR"
BACKUP_DIR=$(cd -- "$BACKUP_DIR" && pwd -P)
[[ $BACKUP_DIR == "$test_root/"* ]] || die 'resolved BACKUP_DIR escaped the scratch directory'
shopt -s nullglob dotglob
existing=("$BACKUP_DIR"/*)
((${#existing[@]} == 0)) || die 'BACKUP_DIR must start empty; use a fresh scratch directory'
shopt -u dotglob
"${docker_cmd[@]}" image inspect "$HUB_IMAGE" >/dev/null
for resource in container volume network; do
    if "${docker_cmd[@]}" "$resource" inspect "$source_name" >/dev/null 2>&1; then
        die "source $resource already exists; refusing to reuse it"
    fi
done
work=$(mktemp -d "$test_root/notes-hub-bk-test-harness.XXXXXX")
chmod 700 "$work"
owner=${work##*/}
seed_name="notes-hub-bk-test-seed-$owner"
if "${docker_cmd[@]}" container inspect "$seed_name" >/dev/null 2>&1; then die 'seed helper already exists'; fi
cleanup_allowed=1
"${docker_cmd[@]}" volume create --label "notes.hub.backup-test-owner=$owner" "$source_name" >/dev/null
volume_created=1
"${docker_cmd[@]}" network create --label "notes.hub.backup-test-owner=$owner" "$source_name" >/dev/null
network_created=1

mkdir -p "$work/fixture/home/.claude" "$work/fixture/home/.codex"
touch "$work/fixture/hub.toml"
printf 'synthetic-provider-marker\n' >"$work/fixture/home/.claude/backup-test-marker"
printf 'synthetic-provider-marker\n' >"$work/fixture/home/.codex/backup-test-marker"
tar -C "$work/fixture" -cf - . | "${docker_cmd[@]}" run --rm -i \
    --name "$seed_name" --label "notes.hub.backup-test-owner=$owner" \
    --pull never --network none --user 0 --entrypoint sh \
    --mount "type=volume,src=$source_name,dst=/data,volume-nocopy" \
    "$HUB_IMAGE" -ec 'tar -C /data -xf -; chown -R 10001:10001 /data'
cat >"$work/compose.yaml" <<EOF
services:
  hub:
    image: $HUB_IMAGE
    container_name: $source_name
    labels:
      notes.hub.backup-test-owner: $owner
    environment:
      NOTES_HUB_LISTEN: 0.0.0.0:8080
      NOTES_HUB_DATA_DIR: /data
      NOTES_HUB_PUBLIC_URL: http://127.0.0.1
      NOTES_HUB_TRUST_PROXY: "false"
      NOTES_HUB_AGENT_ENABLED: "false"
      NOTES_HUB_OIDC_ISSUER: https://backup-test.invalid/application/o/notes-hub/
      NOTES_HUB_OIDC_CLIENT_ID: backup-test
      NOTES_HUB_OIDC_CLIENT_SECRET: synthetic-secret
      NOTES_HUB_OIDC_ALLOWED_USERS: ""
      NOTES_HUB_OIDC_ALLOWED_GROUPS: backup-test
    volumes:
      - source:/data
    networks:
      - source
    ports:
      - "127.0.0.1::8080"
    mem_limit: 1g
    pids_limit: 512
    security_opt:
      - no-new-privileges:true
volumes:
  source:
    external: true
    name: $source_name
networks:
  source:
    external: true
    name: $source_name
EOF
export BACKUP_DIR COMPOSE_FILE="$work/compose.yaml" HUB_IMAGE DOCKER_CONTEXT
export HUB_CONTAINER="$source_name" HUB_VOLUME="$source_name" HUB_SERVICE=hub
compose=("${docker_cmd[@]}" compose --project-name "$source_name" --file "$COMPOSE_FILE")
# Pass the same project name to the backup's compose invocation.
export COMPOSE_PROJECT_NAME="$source_name"
"${compose[@]}" up -d --pull never hub >/dev/null
container_created=1
printf notes-hub >"$work/expected-health"
healthy() {
    local deadline=$((SECONDS + 120)) port status
    while (( SECONDS < deadline )); do
        port=$("${docker_cmd[@]}" inspect --format '{{with index .NetworkSettings.Ports "8080/tcp"}}{{(index . 0).HostPort}}{{end}}' "$source_name")
        [[ $port =~ ^[0-9]+$ ]] || { printf 'test: Docker did not allocate a localhost source port\n' >&2; return 1; }
        if status=$(curl --noproxy '*' --silent --fail --max-time 2 \
            --output "$work/health" --write-out '%{http_code}' "http://127.0.0.1:$port/health" 2>/dev/null) \
            && [[ $status == 200 ]] && cmp --silent "$work/expected-health" "$work/health" \
            && [[ $("${docker_cmd[@]}" inspect --format '{{.State.Health.Status}}' "$source_name") == healthy ]]; then return 0; fi
        [[ $("${docker_cmd[@]}" inspect --format '{{.State.Running}}' "$source_name") == true ]] || return 1
        sleep 2
    done
    return 1
}
healthy || die 'source hub did not become healthy'
printf 'test: PASS source HTTP 200 notes-hub and Docker healthy\n'
# Seed real documents through the hub's agent MCP. The CLI prints the token on
# stdout only; it stays in a shell variable and a 0600 header file in $work and is
# never printed. Its stderr notice goes to a file in $work.
seed_token=$("${docker_cmd[@]}" exec "$source_name" notes-hub --config /data/hub.toml --agent-token add backup-test-writer --scope write 2>"$work/token-cli.err") \
    || die 'could not create the test agent token'
[[ -n $seed_token && $seed_token != *$'\n'* ]] || die 'test agent token output has an unexpected shape'
printf 'Authorization: Bearer %s\n' "$seed_token" >"$work/mcp-auth"
chmod 600 "$work/mcp-auth"
seed_port=$("${docker_cmd[@]}" inspect --format '{{with index .NetworkSettings.Ports "8080/tcp"}}{{(index . 0).HostPort}}{{end}}' "$source_name")
[[ $seed_port =~ ^[0-9]+$ ]] || die 'Docker did not allocate a localhost source port for seeding'
# Calls tools/call on the hub's /mcp; prints the JSON-RPC reply. No Origin header is sent.
call_tool() {
    curl --noproxy '*' --silent --show-error --fail --max-time 20 \
        -H @"$work/mcp-auth" -H 'Content-Type: application/json' \
        --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}" \
        "http://127.0.0.1:$seed_port/mcp"
}
seeded_ids=()
for index in 1 2; do
    reply=$(call_tool create_page "{\"title\":\"Synthetic page $index\",\"markdown\":\"Synthetic backup fixture $index. No private notes or credentials.\"}") \
        || die 'create_page call failed'
    [[ $reply == *'"isError":false'* ]] || die 'create_page returned an error'
    # shellcheck disable=SC2016 # the backticks around the id are literal in the sed pattern.
    id=$(printf '%s' "$reply" | sed -n 's/.*`\([0-9A-Za-z]\{26\}\)`.*/\1/p')
    [[ -n $id ]] || die 'create_page did not return a page id'
    seeded_ids+=("$id")
done
for id in "${seeded_ids[@]}"; do
    written=0
    for _ in $(seq 1 45); do
        if "${docker_cmd[@]}" exec "$source_name" sh -c "test -s /data/vault/.vault/docs/$id.loro && ls /data/vault/pages/*-$id.md >/dev/null 2>&1"; then written=1; break; fi
        sleep 2
    done
    (( written )) || die 'seeded page did not reach the vault files within 90 seconds'
done
printf 'test: PASS seeded %s pages through agent MCP; vault files written; the backup stops the hub\n' "${#seeded_ids[@]}"
if ! bash "$script_dir/notes-hub-backup.sh" >"$work/backup.log" 2>&1; then
    cat "$work/backup.log" >&2
    die 'baseline backup failed'
fi
archives=("$BACKUP_DIR"/notes-hub-data-*.tar.gz)
((${#archives[@]} == 1)) || die 'baseline backup did not publish exactly one archive'
archive=${archives[0]}
[[ $(stat --format='%a' "$BACKUP_DIR") == 700 && $(stat --format='%a' "$archive") == 600 && $(stat --format='%a' "$archive.sha256") == 600 ]] || die 'backup permissions are incorrect'
(cd "$BACKUP_DIR" && sha256sum --check --status "${archive##*/}.sha256") || die 'baseline checksum failed'
members=$(tar -tzf "$archive")
docs_count=$(grep -c '^\./vault/\.vault/docs/[^/]*\.loro$' <<<"$members" || true)
logs_count=$(grep -c '^\./vault/\.vault/logs/[^/]*\.log$' <<<"$members" || true)
md_count=$(grep -c '^\./vault/pages/[^/]*\.md$' <<<"$members" || true)
(( docs_count > 0 && logs_count > 0 && md_count > 0 )) || die 'archive lacks Loro documents, update logs or Markdown pages'
for id in "${seeded_ids[@]}"; do
    grep -qx "\./vault/\.vault/docs/$id\.loro" <<<"$members" || die 'archive lacks a seeded page document'
    listing=$(tar --numeric-owner -tvzf "$archive" "./vault/.vault/docs/$id.loro")
    [[ $listing == *' 10001/10001 '* ]] || die 'archive page document numeric ownership is incorrect'
done
for provider in .claude .codex; do
    marker="home/$provider/backup-test-marker"
    expected=$(sha256sum "$work/fixture/$marker"); expected=${expected%% *}
    actual=$(tar -xOzf "$archive" "./$marker" | sha256sum); actual=${actual%% *}
    [[ $actual == "$expected" ]] || die 'archive omitted the synthetic provider marker'
done
printf 'test: PASS archive SHA-256, private permissions, %s documents, %s update logs, %s Markdown pages, numeric ownership, provider markers\n' "$docs_count" "$logs_count" "$md_count"
bash "$script_dir/restore-test.sh" "$archive" || die 'isolated restore test failed'
printf 'test: PASS isolated restore\n'

mkdir "$work/shim"
cat >"$work/shim/docker" <<'EOF'
#!/usr/bin/env bash
set -eu
run=0; tar_entrypoint=0; previous=
for argument in "$@"; do
    [[ $argument != run ]] || run=1
    if [[ $previous == --entrypoint && $argument == tar ]]; then tar_entrypoint=1; fi
    previous=$argument
done
if (( run && tar_entrypoint )); then
    if [[ ${INJECT_TAR_SIGNAL:-0} == 1 ]]; then
        # PPID is the backup Bash process started by this harness.
        kill -TERM "$PPID"
        exit 143
    fi
    exit 44
fi
exec /usr/bin/docker "$@"
EOF
chmod 700 "$work/shim/docker"
before=$("${docker_cmd[@]}" inspect --format '{{.State.StartedAt}}' "$source_name")
if PATH="$work/shim:$PATH" bash "$script_dir/notes-hub-backup.sh" >"$work/failure.log" 2>&1; then
    die 'injected tar failure unexpectedly succeeded'
else
    failure_status=$?
fi
(( failure_status == 44 )) || die "injected tar failure returned unexpected status $failure_status"
[[ $("${docker_cmd[@]}" inspect --format '{{.State.Running}}' "$source_name") == true ]] || die 'tar failure left the source stopped'
[[ $("${docker_cmd[@]}" inspect --format '{{.State.StartedAt}}' "$source_name") != "$before" ]] || die 'tar failure did not exercise stop and restart'
archives=("$BACKUP_DIR"/notes-hub-data-*.tar.gz)
partials=("$BACKUP_DIR"/.notes-hub-data-*)
((${#archives[@]} == 1 && ${#partials[@]} == 0)) || die 'tar failure left an incomplete or published archive'
healthy || die 'source did not recover health after the tar failure'
printf 'test: PASS injected tar failure returns nonzero, restarts source, publishes nothing, removes partials\n'

before=$("${docker_cmd[@]}" inspect --format '{{.State.StartedAt}}' "$source_name")
if INJECT_TAR_SIGNAL=1 PATH="$work/shim:$PATH" bash "$script_dir/notes-hub-backup.sh" >"$work/signal.log" 2>&1; then
    die 'injected TERM unexpectedly succeeded'
else
    signal_status=$?
fi
if (( signal_status != 143 )); then
    cat "$work/signal.log" >&2
    die "injected TERM returned unexpected status $signal_status"
fi
[[ $("${docker_cmd[@]}" inspect --format '{{.State.Running}}' "$source_name") == true ]] || die 'TERM left the source stopped'
[[ $("${docker_cmd[@]}" inspect --format '{{.State.StartedAt}}' "$source_name") != "$before" ]] || die 'TERM did not exercise stop and restart'
archives=("$BACKUP_DIR"/notes-hub-data-*.tar.gz)
partials=("$BACKUP_DIR"/.notes-hub-data-*)
((${#archives[@]} == 1 && ${#partials[@]} == 0)) || die 'TERM left an incomplete or published archive'
healthy || die 'source did not recover health after TERM'
printf 'test: PASS injected TERM returns 143, restarts source, publishes nothing, removes partials\n'

fake_archive="$BACKUP_DIR/notes-hub-data-20991231T235959Z.tar.gz"
[[ ! -e $fake_archive ]] || die 'free-space fixture already exists'
free_bytes=$(df --block-size=1 --output=avail "$BACKUP_DIR" | tail -n 1 | tr -d '[:space:]')
truncate --size "$((free_bytes * 2 + 1))" "$fake_archive"
# ZFS can account a small metadata allocation even for a fully sparse file.
allocated_bytes=$(( $(stat --format='%b' "$fake_archive") * 512 ))
(( allocated_bytes < 1048576 )) || die 'free-space fixture allocated unexpected disk space'
before=$("${docker_cmd[@]}" inspect --format '{{.State.StartedAt}}' "$source_name")
if bash "$script_dir/notes-hub-backup.sh" >"$work/free-space.log" 2>&1; then die 'free-space guard unexpectedly succeeded'; fi
[[ $(cat "$work/free-space.log") == *'less than twice the last archive'* ]] || die 'backup failed for a reason other than the free-space guard'
[[ $("${docker_cmd[@]}" inspect --format '{{.State.Running}}' "$source_name") == true && $("${docker_cmd[@]}" inspect --format '{{.State.StartedAt}}' "$source_name") == "$before" ]] || die 'free-space guard changed the source running state'
rm -f -- "$fake_archive"
fake_archive=
archives=("$BACKUP_DIR"/notes-hub-data-*.tar.gz)
partials=("$BACKUP_DIR"/.notes-hub-data-*)
((${#archives[@]} == 1 && ${#partials[@]} == 0)) || die 'free-space guard left an incomplete or published archive'
printf 'test: PASS sparse oversized previous archive rejects backup before stopping source\n'

if [[ ${TEST_RETENTION:-1} == 1 ]]; then
    for ((round=1; round<8; round++)); do
        sleep 1
        if ! bash "$script_dir/notes-hub-backup.sh" >"$work/retention-$round.log" 2>&1; then
            cat "$work/retention-$round.log" >&2
            die "retention backup $round failed"
        fi
    done
    archives=("$BACKUP_DIR"/notes-hub-data-*.tar.gz)
    checksums=("$BACKUP_DIR"/notes-hub-data-*.tar.gz.sha256)
    ((${#archives[@]} == 7 && ${#checksums[@]} == 7)) || die 'retention did not leave exactly seven archive/checksum pairs'
    [[ ! -e $archive && ! -e $archive.sha256 ]] || die 'retention did not remove the oldest pair'
    for retained in "${archives[@]}"; do
        (cd "$BACKUP_DIR" && sha256sum --check --status "${retained##*/}.sha256") || die 'retained archive checksum failed'
    done
    healthy || die 'source did not recover health after retention backups'
    printf 'test: PASS eight backups retain newest seven complete, verified pairs\n'
fi
