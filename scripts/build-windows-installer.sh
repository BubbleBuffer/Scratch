#!/usr/bin/env bash
# Builds the Windows app and its Inno Setup installer for a release tag.
# Used by .github/workflows/release.yml and nightly.yml on windows-latest
# (Git Bash), after Flutter, Rust and Bun are set up.
#
#   scripts/build-windows-installer.sh <tag> <numeric> <build-number> <out-dir>
#
#   tag           v0.1.0-prealpha.11 or v0.1.0-nightly.20261027. Built into the
#                 app as SCRATCH_RELEASE_TAG: the updater's build identity
#                 (app/lib/features/updates). Builds without it are
#                 development builds and never offer updates.
#   numeric       0.1.0: the Windows file version (numbers only).
#   build-number  the Flutter build number.
#   out-dir       gets Scratch-Setup-<tag without v>.exe
#
# Inno Setup is installed with Chocolatey when ISCC.exe is missing.
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: $0 <tag> <numeric> <build-number> <out-dir>" >&2
  exit 2
fi
tag=$1
numeric=$2
code=$3
out=$4
version=${tag#v}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
app_dir="$(cd "$script_dir/../app" && pwd)"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
cd "$app_dir"

flutter pub get
# Windows file versions are numeric, so the pre-release part stays out of
# --build-name; the full tag goes in through the dart-define.
flutter build windows --release \
  --build-name="$numeric" --build-number="$code" \
  --dart-define=SCRATCH_RELEASE_TAG="$tag"

# The app and scratch-cli find Claude Code as a `claude-code` folder and
# `bun\bun.exe` beside the executable, so ship both in the Release folder
# the installer packs.
release_dir="$(cygpath -w "$app_dir/build/windows/x64/runner/Release")"
pwsh -NoProfile -File "$(cygpath -w "$app_dir/windows/installer/bundle-claude-code.ps1")" -ReleaseDir "$release_dir"

iscc="/c/Program Files (x86)/Inno Setup 6/ISCC.exe"
if [[ ! -x $iscc ]]; then
  choco install innosetup --no-progress -y
fi
installer_dir="$app_dir/build/windows/installer"
rm -f "$installer_dir"/Scratch-Setup-*.exe
MSYS_NO_PATHCONV=1 "$iscc" /Q "/DAppVersion=$numeric" \
  "/DVersionText=$version" "/DSha7=$(git rev-parse --short=7 HEAD)" \
  "/DSourceDir=$release_dir" "/DOutputDir=$(cygpath -w "$installer_dir")" \
  "$(cygpath -w "$app_dir/windows/installer/scratch.iss")"
mv "$installer_dir"/Scratch-Setup-*.exe "$out/Scratch-Setup-$version.exe"
echo "built $out/Scratch-Setup-$version.exe"
