<#
.SYNOPSIS
  Adds the BubblePaw Claude Code extension and Bun to the Windows app folder.

.DESCRIPTION
  The app and scratch-cli look for Claude Code as a `claude-code` folder and
  Bun as `bun\bun.exe` beside the executable. This adds both to the
  `flutter build windows --release` Release folder, which the installer packs
  as it is (scratch.iss [Files] recurses):

    <Release>\claude-code\   package.json, bun.lock, NOTICE, src\, node_modules\
    <Release>\bun\bun.exe

  The extension comes from BubblePaw-Core at the commit Cargo.lock pins, with
  its production dependencies installed, the same files and command as
  deploy/hub/Dockerfile uses for the hub. Claude Code itself is one of those
  dependencies (@anthropic-ai/claude-agent-sdk and its win32-x64 package), so
  no separate Claude Code install is involved. Bun is the caller's: the
  workflows install the hub's version with oven-sh/setup-bun.

.PARAMETER ReleaseDir
  The Release folder to add the bundle to.

.PARAMETER Bun
  The Bun executable to ship. Default: `bun` on PATH.

.PARAMETER Source
  The BubblePaw-Core repository.

.PARAMETER LockFile
  The Cargo.lock naming the pinned BubblePaw-Core commit. Default: this repo's.

.EXAMPLE
  ./app/windows/installer/bundle-claude-code.ps1 -ReleaseDir ./app/build/windows/x64/runner/Release
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ReleaseDir,
    [string]$Bun,
    [string]$Source = 'https://github.com/BubbleBuffer/BubblePaw-Core',
    [string]$LockFile = (Join-Path $PSScriptRoot '../../../Cargo.lock')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Errors are one line on stderr and a non-zero exit: the marker line is only
# printed when the bundle is complete.
function Stop-With([string]$Message) {
    [Console]::Error.WriteLine($Message)
    exit 1
}

function Invoke-Git([string[]]$Arguments) {
    & git @Arguments
    if ($LASTEXITCODE -ne 0) {
        Stop-With ("git " + ($Arguments -join ' ') + " failed ($LASTEXITCODE)")
    }
}

if (-not (Test-Path -LiteralPath $ReleaseDir -PathType Container)) {
    Stop-With "no Release folder at $ReleaseDir; run 'flutter build windows --release' first"
}
$ReleaseDir = (Resolve-Path -LiteralPath $ReleaseDir).Path
if (-not (Test-Path -LiteralPath (Join-Path $ReleaseDir 'notes.exe'))) {
    Stop-With "$ReleaseDir does not hold notes.exe; pass the Release folder"
}

if (-not $Bun) {
    $found = Get-Command bun -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if (-not $found) {
        Stop-With 'bun was not found on PATH; install Bun 1.4.2 or pass -Bun'
    }
    $Bun = $found.Source
}
$Bun = (Resolve-Path -LiteralPath $Bun).Path

if (-not (Test-Path -LiteralPath $LockFile -PathType Leaf)) {
    Stop-With "no Cargo.lock at $LockFile"
}
$pin = [regex]::Match(
    (Get-Content -Raw -LiteralPath $LockFile),
    'BubblePaw-Core\?[a-z]+=[^"#]+#(?<sha>[0-9a-f]{40})'
)
if (-not $pin.Success) {
    Stop-With "Cargo.lock pins no BubblePaw-Core commit: $LockFile"
}
$revision = $pin.Groups['sha'].Value

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("bubblepaw-claude-code-" + [guid]::NewGuid().ToString('N'))
$extension = Join-Path $ReleaseDir 'claude-code'
$bunDir = Join-Path $ReleaseDir 'bun'
try {
    # The extension folder at the pinned commit, without the rest of the tree.
    New-Item -ItemType Directory -Path $work | Out-Null
    Invoke-Git @('-C', $work, 'init', '--quiet')
    Invoke-Git @('-C', $work, 'remote', 'add', 'origin', $Source)
    Invoke-Git @('-C', $work, 'fetch', '--quiet', '--depth', '1', '--filter=blob:none', 'origin', $revision)
    Invoke-Git @('-C', $work, 'checkout', '--quiet', '--detach', 'FETCH_HEAD')

    $source = Join-Path $work 'extensions/claude-code'
    foreach ($file in @('package.json', 'bun.lock', 'NOTICE')) {
        if (-not (Test-Path -LiteralPath (Join-Path $source $file))) {
            Stop-With "BubblePaw-Core $revision has no extensions/claude-code/$file"
        }
    }
    if (Test-Path -LiteralPath $extension) {
        Remove-Item -LiteralPath $extension -Recurse -Force
    }
    New-Item -ItemType Directory -Path $extension | Out-Null
    foreach ($file in @('package.json', 'bun.lock', 'NOTICE')) {
        Copy-Item -LiteralPath (Join-Path $source $file) -Destination $extension
    }
    Copy-Item -LiteralPath (Join-Path $source 'src') -Destination $extension -Recurse

    Push-Location $extension
    try {
        & $Bun install --production --frozen-lockfile
        if ($LASTEXITCODE -ne 0) {
            Stop-With "bun install --production --frozen-lockfile failed ($LASTEXITCODE)"
        }
    } finally {
        Pop-Location
    }

    if (Test-Path -LiteralPath $bunDir) {
        Remove-Item -LiteralPath $bunDir -Recurse -Force
    }
    New-Item -ItemType Directory -Path $bunDir | Out-Null
    Copy-Item -LiteralPath $Bun -Destination (Join-Path $bunDir 'bun.exe')
} finally {
    if (Test-Path -LiteralPath $work) {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }
}

$files = @(
    (Join-Path $extension 'package.json'),
    (Join-Path $extension 'src/main.ts'),
    (Join-Path $extension 'node_modules/@anthropic-ai/claude-agent-sdk'),
    (Join-Path $bunDir 'bun.exe')
)
$missing = @($files | Where-Object { -not (Test-Path -LiteralPath $_) })
if ($missing.Count -gt 0) {
    Stop-With ("the claude-code bundle is incomplete, missing: " + ($missing -join ', '))
}

$size = [math]::Round(
    ((Get-ChildItem -LiteralPath $extension -Recurse -File | Measure-Object -Property Length -Sum).Sum +
        (Get-Item -LiteralPath (Join-Path $bunDir 'bun.exe')).Length) / 1MB
)
Write-Output "claude-code bundle: $size MB"
Write-Output 'claude-code bundle ok'
