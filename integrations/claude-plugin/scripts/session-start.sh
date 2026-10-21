#!/usr/bin/env bash
# SessionStart hook: puts the memory rules and the notes memory index into the session context.
# Always exits 0. Stdout goes into context, so the token is never printed or cached.
set -u

if [ -n "${CLAUDE_PROJECT_DIR:-}" ]; then
  cd "$CLAUDE_PROJECT_DIR" 2>/dev/null || exit 0
fi

# The rules come from the CLI (notes memory rules), so the hook keeps no copy.
# An older or missing notes CLI gets one line instead.
print_rules() {
  local rules
  if rules=$(notes memory rules 2>/dev/null); then
    printf '%s\n' "$rules"
  else
    echo "Memory rules unavailable: update the notes CLI (notes memory rules). Full rules: the notes:memory skill."
  fi
}

# Cache directory for the last good global index: one file, global.txt.
cache_dir() {
  if [ -n "${LOCALAPPDATA:-}" ] && command -v cygpath >/dev/null 2>&1; then
    printf '%s/notes/memory-index' "$(cygpath -u "$LOCALAPPDATA")"
  else
    printf '%s/notes/memory-index' "${XDG_CACHE_HOME:-$HOME/.cache}"
  fi
}

if ! command -v notes >/dev/null 2>&1; then
  print_rules
  exit 0
fi

dir=$(cache_dir)
cache_file="$dir/global.txt"

# Config resolution for the cache identity. An environment/plugin
# hub override is a request target, not the user's configured cache identity.
notes_config_path() {
  local drive bs
  if [ -n "${APPDATA:-}" ]; then
    drive=${APPDATA%%:*}
    config="/${drive,,}${APPDATA#*:}"
    bs="\\"
    config="${config//"$bs"//}/notes/config.toml"
  else
    config="${XDG_CONFIG_HOME:-$HOME/.config}/notes/config.toml"
  fi
}

# The hub the CLI config names. There is no built-in hub, so without one the
# config identifies nothing and the cache stays off.
configured_hub=
config=
notes_config_path
if [ -e "$config" ]; then
  # Recognize simple top-level TOML hub strings without evaluating the file.
  # Unreadable/complex hub settings disable cache access conservatively.
  if [ ! -r "$config" ]; then
    configured_hub=
  else
    double_hub='^[[:space:]]*hub[[:space:]]*=[[:space:]]*"([^"\\]*)"[[:space:]]*(#.*)?$'
    single_hub="^[[:space:]]*hub[[:space:]]*=[[:space:]]*'([^']*)'[[:space:]]*(#.*)?$"
    double_token='^[[:space:]]*token[[:space:]]*=[[:space:]]*"[^"\\]*"[[:space:]]*(#.*)?$'
    single_token="^[[:space:]]*token[[:space:]]*=[[:space:]]*'[^']*'[[:space:]]*(#.*)?$"
    ignored_line='^[[:space:]]*(#.*)?$'
    hub_seen=0
    while IFS= read -r line || [ -n "$line" ]; do
      line=${line%$'\r'}
      [[ $line =~ ^[[:space:]]*\[ ]] && break
      if [[ $line =~ ^[[:space:]]*hub[[:space:]]*= ]]; then
        configured_hub=
        [ "$hub_seen" -eq 0 ] || break
        hub_seen=1
        if [[ $line =~ $double_hub || $line =~ $single_hub ]]; then
          configured_hub=${BASH_REMATCH[1]}
        else
          break
        fi
      elif [[ ! $line =~ $ignored_line && ! $line =~ $double_token && ! $line =~ $single_token ]]; then
        configured_hub=
        break
      fi
    done < "$config"
  fi
fi
effective_hub=${CLAUDE_PLUGIN_OPTION_HUB_URL:-$configured_hub}
hook_token=${CLAUDE_PLUGIN_OPTION_HUB_TOKEN:-${NOTES_TOKEN:-}}
cache_allowed=0
if [ -n "$configured_hub" ] && [ "${effective_hub%/}" = "${configured_hub%/}" ]; then
  cache_allowed=1
fi

index=$(
  # An empty token environment value leaves config token resolution to the CLI.
  # Global only. Cap the hub call at 3 s so an unreachable hub cannot hold up
  # session start (a healthy hub answers in 0.6-1.1 s). Git Bash's GNU timeout is
  # at a fixed path, which avoids picking up Windows' timeout.exe; without it the call is uncapped.
  cap=()
  if [ -x /usr/bin/timeout ]; then cap=(/usr/bin/timeout 3); fi
  NOTES_HUB_URL="$effective_hub" NOTES_TOKEN="$hook_token" "${cap[@]}" notes memory index --project global --max-chars 8000 2>/dev/null
)
status=$?

print_rules

if [ "$status" -eq 0 ]; then
  echo "# Memory index (notes vault)"
  printf '%s\n' "$index"
  # Cache the good index for offline sessions. Failures here are ignored.
  if [ "$cache_allowed" -eq 1 ] && mkdir -p "$dir" 2>/dev/null; then
    tmp="$dir/.global.$$.tmp"
    if printf '%s\n' "$index" > "$tmp" 2>/dev/null; then
      mv -f "$tmp" "$cache_file" 2>/dev/null || rm -f "$tmp" 2>/dev/null
    fi
  fi
  exit 0
fi

if [ "$cache_allowed" -eq 1 ] && [ -f "$cache_file" ] && cached=$(cat "$cache_file" 2>/dev/null); then
  stamp=$(date -u -r "$cache_file" '+%Y-%m-%d %H:%M UTC' 2>/dev/null)
  echo "# Memory index (notes vault), cached from ${stamp:-an unknown time}. The hub is unreachable, so memory writes will fail until it is back."
  printf '%s\n' "$cached"
  exit 0
fi

echo "Memory index unavailable: \`notes memory index\` failed (hub unreachable or token rejected)."
exit 0
