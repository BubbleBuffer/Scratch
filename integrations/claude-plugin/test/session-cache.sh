#!/usr/bin/env bash
# Isolated cache identity contract: fake CLI, config and both cache roots.
set -euo pipefail
# SessionStart consumes JSON on stdin; these cases model an absent event.
exec </dev/null
repo_root=$(cd "$(dirname "$0")/../../.." && pwd)
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/bin" "$scratch/config/notes" "$scratch/project"
export XDG_CONFIG_HOME="$scratch/config" XDG_CACHE_HOME="$scratch/xdg"
export LOCALAPPDATA="$scratch/localappdata" CLAUDE_PROJECT_DIR="$scratch/project"
export TEST_INDEX=production-index TEST_INDEX_EXIT=0
export TEST_RULES=fake-memory-rules TEST_RULES_EXIT=0
unset APPDATA CLAUDE_PLUGIN_OPTION_HUB_TOKEN CLAUDE_PLUGIN_OPTION_HUB_URL NOTES_HUB_URL
cat > "$scratch/bin/notes" <<'EOF'
#!/usr/bin/env bash
if [ "$1 $2" = 'memory rules' ]; then
  [ "$TEST_RULES_EXIT" -eq 0 ] || exit "$TEST_RULES_EXIT"
  printf '%s\n' "$TEST_RULES"
  exit 0
fi
[ "$1 $2" = 'memory index' ] || exit 1
[ "$TEST_INDEX_EXIT" -eq 0 ] || exit "$TEST_INDEX_EXIT"
printf '%s\n' "$TEST_INDEX"
EOF
# Exercise the Git Bash LOCALAPPDATA branch while running on Linux.
cat > "$scratch/bin/cygpath" <<'EOF'
#!/usr/bin/env bash
[ "$1" = -u ] || exit 1
printf '%s\n' "$2"
EOF
chmod +x "$scratch/bin/notes" "$scratch/bin/cygpath"
export PATH="$scratch/bin:$PATH"
hook="$repo_root/integrations/claude-plugin/scripts/session-start.sh"
check() {
  local message=$1
  shift
  if ! "$@"; then printf 'session cache test FAILED: %s\n' "$message" >&2; exit 1; fi
}
run_hook() { bash "$hook" > "$scratch/output" 2> "$scratch/stderr"; }
cache="$LOCALAPPDATA/notes/memory-index/global.txt"

# Without a configured hub nothing identifies the cache: the index is shown live only.
run_hook
check 'unconfigured hub wrote a cache' test ! -e "$cache"
check 'unconfigured hub live index missing' grep -qx production-index "$scratch/output"

# The configured hub qualifies without a plugin option; LOCALAPPDATA wins over XDG.
printf 'hub = "https://notes.example.com"\n' >"$XDG_CONFIG_HOME/notes/config.toml"
run_hook
check 'configured hub index not cached in LOCALAPPDATA' grep -qx production-index "$cache"
check 'LOCALAPPDATA branch wrote to XDG cache' test ! -e "$XDG_CACHE_HOME/notes"
# The rules text comes from `notes memory rules`; a failing CLI gets the one-line fallback.
check 'rules not printed from notes memory rules' grep -qx fake-memory-rules "$scratch/output"
export TEST_RULES_EXIT=1
run_hook
check 'rules fallback line missing' grep -q '^Memory rules unavailable: update the notes CLI (notes memory rules)' "$scratch/output"
check 'fallback kept the index' grep -qx production-index "$scratch/output"
export TEST_RULES_EXIT=0

# A staging override may be shown live but cannot replace or read global.txt.
export CLAUDE_PLUGIN_OPTION_HUB_URL=http://staging.invalid TEST_INDEX=staging-index
run_hook
check 'staging live index missing' grep -qx staging-index "$scratch/output"
check 'staging replaced production cache' grep -qx production-index "$cache"
export TEST_INDEX_EXIT=1
run_hook
check 'staging failure exposed production cache' grep -q 'Memory index unavailable' "$scratch/output"
check 'staging failure exposed cached production text' test "$(grep -c production-index "$scratch/output" || true)" -eq 0

# A configured custom hub qualifies, including a harmless trailing slash.
printf "hub = 'https://custom.invalid/' # configured hub\n" > "$XDG_CONFIG_HOME/notes/config.toml"
export CLAUDE_PLUGIN_OPTION_HUB_URL=https://custom.invalid TEST_INDEX=custom-index TEST_INDEX_EXIT=0
run_hook
check 'configured custom hub was not cached' grep -qx custom-index "$cache"
export TEST_INDEX_EXIT=1
run_hook
check 'configured hub offline cache missing' grep -qx custom-index "$scratch/output"
check 'configured hub offline label missing' grep -q 'cached from' "$scratch/output"

# Common double-quoted config, token lines and comments also preserve identity.
printf '# synthetic config\ntoken = "synthetic-config-token"\nhub = "https://custom.invalid"\n' > "$XDG_CONFIG_HOME/notes/config.toml"
export TEST_INDEX=double-quoted-index TEST_INDEX_EXIT=0
run_hook
check 'double-quoted configured hub was not cached' grep -qx double-quoted-index "$cache"

# Unsupported TOML forms fail closed instead of qualifying the configured hub.
printf '"hub" = "https://custom.invalid"\n' > "$XDG_CONFIG_HOME/notes/config.toml"
export CLAUDE_PLUGIN_OPTION_HUB_URL=https://notes.example.com TEST_INDEX=unsupported-index
run_hook
check 'unsupported config enabled the default cache' grep -qx double-quoted-index "$cache"
printf "hub = 'https://custom.invalid'\n" > "$XDG_CONFIG_HOME/notes/config.toml"

# An environment target override cannot redefine the configured cache identity.
export NOTES_HUB_URL=http://staging.invalid CLAUDE_PLUGIN_OPTION_HUB_URL=http://staging.invalid
export TEST_INDEX=staging-index TEST_INDEX_EXIT=0
run_hook
check 'environment override replaced configured hub cache' grep -qx double-quoted-index "$cache"

# With no cygpath/LOCALAPPDATA branch the XDG root remains isolated as well.
unset LOCALAPPDATA NOTES_HUB_URL
export CLAUDE_PLUGIN_OPTION_HUB_URL=https://custom.invalid TEST_INDEX=xdg-index
run_hook
check 'XDG fallback index not cached' grep -qx xdg-index "$XDG_CACHE_HOME/notes/memory-index/global.txt"
check 'cache hook wrote stderr' test ! -s "$scratch/stderr"
printf 'session cache: passed (configured/unconfigured hub, staging guard, LOCALAPPDATA and XDG roots, offline fallback)\n'
