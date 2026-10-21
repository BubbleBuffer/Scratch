#!/usr/bin/env bash
# Regenerates the flutter_rust_bridge bindings from app/flutter_rust_bridge.yaml:
#   app/rust/src/frb_generated.rs and app/lib/src/rust/frb_generated*.dart
# Works from any directory. Extra arguments are passed to the codegen.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
app_dir="$repo_root/app"

if [[ ! -f "$app_dir/flutter_rust_bridge.yaml" ]]; then
  echo "error: $app_dir/flutter_rust_bridge.yaml not found" >&2
  exit 1
fi
if ! command -v flutter_rust_bridge_codegen >/dev/null 2>&1; then
  echo "error: flutter_rust_bridge_codegen is not on PATH" >&2
  exit 1
fi

cd "$app_dir"
exec flutter_rust_bridge_codegen generate "$@"
