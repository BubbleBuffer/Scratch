#!/usr/bin/env bash
# Dispatch the build01 APK job, watch it, and print the linux01 publish result.
#   deploy/android/build01-apk.sh [--ref <branch>]
set -euo pipefail
ref=master
while [ $# -gt 0 ]; do
  case "$1" in
    --ref) ref=${2:?--ref needs a branch}; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done
repo=BubbleBuffer/notes-app
before=$(gh run list -R "$repo" -w android-apk.yml -L 1 --json databaseId -q '.[0].databaseId // 0')
gh workflow run android-apk.yml -R "$repo" --ref "$ref"
run=''
for attempt in {1..12}; do
  sleep 5
  run=$(gh run list -R "$repo" -w android-apk.yml -b "$ref" -e workflow_dispatch -L 10 \
    --json databaseId -q ".[] | select(.databaseId > $before) | .databaseId" | head -1)
  [ -z "$run" ] || break
done
[ -n "$run" ] || { echo 'dispatched, but no new run appeared within 60 seconds' >&2; exit 1; }
echo "run: https://github.com/$repo/actions/runs/$run"
gh run watch "$run" -R "$repo" --exit-status --interval 20 >/dev/null
gh run view "$run" -R "$repo" --log | sed -n '/Sign and publish on linux01/ { /added\|signer\|fingerprint\|uploaded\|code /p; }'
echo "build and publish succeeded: https://github.com/$repo/actions/runs/$run"
