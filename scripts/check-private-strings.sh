#!/usr/bin/env bash
# Fails when tracked files in this public repository contain private strings:
# private host names and addresses, local paths, personal accounts, secret
# formats, or a workflow job on a self-hosted runner.
#
# The private patterns themselves are private, so they are not in this
# repository. CI passes them in the PRIVATE_STRINGS environment variable (an
# Actions secret); locally, pass the pattern file with --patterns <file>. One
# rule per line, blank lines and lines starting with # ignored:
#   deny  <ERE>            matched case-insensitively
#   host  <ERE>            matched case-sensitively as whole words
#   allow <path>:<literal> a hit in <path> whose line contains <literal> is allowed
#
# Secret formats and self-hosted runners are always checked. Without patterns
# the private-string part is skipped with a warning, or fails with
# --require-patterns.
#
# Usage: scripts/check-private-strings.sh [--patterns <file>] [--require-patterns]
set -euo pipefail

patterns_file=
require=0
while (($#)); do
  case $1 in
    --patterns) patterns_file=$2; shift 2 ;;
    --require-patterns) require=1; shift ;;
    -h|--help) sed -n '2,19p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

cd "$(git rev-parse --show-toplevel)"

if [[ -n $patterns_file ]]; then
  rules=$(cat "$patterns_file")
else
  rules=${PRIVATE_STRINGS:-}
fi

deny=() hosts=() allow=()
while IFS= read -r line; do
  line=${line%$'\r'}
  [[ -z ${line//[[:space:]]/} || $line == \#* ]] && continue
  kind=${line%%[[:space:]]*}
  value=${line#"$kind"}
  value=${value#"${value%%[![:space:]]*}"}
  case $kind in
    deny) deny+=("$value") ;;
    host) hosts+=("$value") ;;
    allow) allow+=("$value") ;;
    *) echo "bad pattern rule (kind '$kind')" >&2; exit 2 ;;
  esac
done <<< "$rules"

join() { local IFS='|'; echo "$*"; }
fail=0

# Private strings, in file contents and in file names.
if ((${#deny[@]} + ${#hosts[@]})); then
  hits=
  if ((${#deny[@]})); then
    re=$(join "${deny[@]}")
    hits+=$(git grep -n -I -i -E -e "$re" || true)$'\n'
    hits+=$(git ls-files | grep -i -E -e "$re" | sed 's/$/:0:(file name)/' || true)$'\n'
  fi
  if ((${#hosts[@]})); then
    re="\\b($(join "${hosts[@]}"))\\b"
    hits+=$(git grep -n -I -E -e "$re" || true)$'\n'
    hits+=$(git ls-files | grep -E -e "$re" | sed 's/$/:0:(file name)/' || true)$'\n'
  fi
  left=
  while IFS= read -r h; do
    [[ -n $h ]] || continue
    for a in "${allow[@]}"; do
      [[ $h == "${a%%:*}:"* && $h == *"${a#*:}"* ]] && continue 2
    done
    left+=$h$'\n'
  done < <(printf '%s' "$hits" | sort -u)
  if [[ -n $left ]]; then
    echo "private strings found (file:line):" >&2
    # File and line only: the matched text stays out of the public CI log.
    printf '%s' "$left" | cut -d: -f1,2 >&2
    fail=1
  else
    echo "private strings: none (${#deny[@]} deny, ${#hosts[@]} host, ${#allow[@]} allow rules)"
  fi
elif ((require)); then
  echo "no private-string patterns given (PRIVATE_STRINGS or --patterns)" >&2
  exit 2
else
  echo "warning: no private-string patterns given; checked secret formats and runners only" >&2
fi

# Secret formats.
secret='-----BEGIN ([A-Z]+ )?PRIVATE KEY-----|gh[pousr]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{40,}|AKIA[0-9A-Z]{16}|xox[baprs]-[A-Za-z0-9-]{10,}|sk-[A-Za-z0-9_-]{32,}|AIza[0-9A-Za-z_-]{35}|-----BEGIN OPENSSH PRIVATE KEY-----'
secrets=$(git grep -n -I -E -e "$secret" -- . ":!scripts/check-private-strings.sh" | cut -d: -f1,2 || true)
if [[ -n $secrets ]]; then
  echo "possible secrets (file:line):" >&2
  printf '%s\n' "$secrets" >&2
  fail=1
else
  echo "secret formats: none"
fi

# No workflow job may run on a self-hosted runner: workflows in a public
# repository can be triggered from forks.
runners=$(git grep -n -i -E -e 'self-hosted' -- '.github/workflows/' | cut -d: -f1,2 || true)
if [[ -n $runners ]]; then
  echo "self-hosted runner in a workflow (file:line):" >&2
  printf '%s\n' "$runners" >&2
  fail=1
else
  echo "self-hosted runners: none"
fi

# Personal agent, editor and tool folders stay out of the public repository
# (they are in .gitignore too). The repo keeps only the dot-files the build
# uses: .github/, .gitignore, .gitattributes, .dockerignore, .cargo/,
# app/.metadata, deploy/hub/.env.example and the shipped Claude plugin's
# .claude-plugin/ and .mcp.json.
tooldirs='(^|/)\.(claude|impeccable|codex|cursor|vscode|idea|fleet|zed|windsurf|continue|aider[^/]*|cline|roo|kiro|gemini|junie|opencode|serena|bubblepaw|agents?)/|(^|/)\.(cursorrules|windsurfrules|aider[^/]*)$'
dirs=$(git ls-files | grep -E -e "$tooldirs" || true)
if [[ -n $dirs ]]; then
  echo "personal tool folders or files tracked:" >&2
  printf '%s\n' "$dirs" >&2
  fail=1
else
  echo "personal tool folders: none"
fi

exit "$fail"
