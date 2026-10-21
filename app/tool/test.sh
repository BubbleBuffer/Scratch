#!/usr/bin/env bash
set -euo pipefail
tier=${1:-fast}
case "$tier" in fast|golden|perf|all) ;; *) echo 'Usage: tool/test.sh fast|golden|perf|all' >&2; exit 2 ;; esac
app_root=$(cd -- "$(dirname -- "$0")/.." && pwd)
repo_root=$(cd -- "$app_root/.." && pwd)
cd "$repo_root"
case "$(uname -s)" in
  Darwin) library=libnotes_bridge.dylib; agent=fake_agent ;;
  MINGW*|MSYS*|CYGWIN*) library=notes_bridge.dll; agent=fake_agent.exe ;;
  *) library=libnotes_bridge.so; agent=fake_agent ;;
esac
bridge="$repo_root/target/release/$library"
fake_agent="$repo_root/target/release/examples/$agent"
stale() {
  [[ ! -f "$1" ]] || [[ -n "$(find crates app/rust -type f \( -name '*.rs' -o -name Cargo.toml \) -newer "$1" -print -quit)" ]] ||
    [[ Cargo.toml -nt "$1" || Cargo.lock -nt "$1" ]]
}
if stale "$bridge"; then cargo build --release -p notes_bridge; fi
if stale "$fake_agent"; then cargo build --release -p vault_mcp --examples; fi
export NOTES_BRIDGE_LIB="$bridge" NOTES_FAKE_AGENT="$fake_agent"
cd "$app_root"
run_tier() {
  local part=$1 started=$SECONDS
  case "$part" in
    fast) flutter test -x 'golden || perf || drive' ;;
    golden) flutter test -t golden ;;
    perf) flutter test -t perf --concurrency=1 ;;
  esac
  echo "$part: $((SECONDS - started))s"
}
started=$SECONDS
if [[ "$tier" == all ]]; then
  run_tier fast
  run_tier golden
  run_tier perf
else
  run_tier "$tier"
fi
echo "Total tests: $((SECONDS - started))s"
