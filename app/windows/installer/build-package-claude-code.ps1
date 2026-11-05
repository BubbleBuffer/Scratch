<#
.SYNOPSIS
  Builds the claude-code runtime package for Windows and its manifest entry.

.DESCRIPTION
  Claude Code is not part of the installer: the app and scratch-cli download it
  on demand (wiki/addons/packages.md). This builds the package the release
  attaches, one zip per platform:

    scratch-pkg-claude-code-<version>-windows-x64.zip
      claude-code\   package.json, bun.lock, NOTICE, src\, node_modules\
      bun\bun.exe

  and adds its entry (name, version, platform, url, sha256, size) to
  <OutDir>\manifest.json, replacing an entry of the same name and platform.
  The app build embeds that manifest (crates/packages/manifest.json), so the
  zip's checksum is pinned in the binary.

  The extension comes from BubblePaw-Core at the commit Cargo.lock pins, with
  its production dependencies installed, the same files and command as
  deploy/hub/Dockerfile uses for the hub. Claude Code itself is one of those
  dependencies (@anthropic-ai/claude-agent-sdk and its win32-x64 package).
  Bun is the caller's: the workflows install the hub's version with
  oven-sh/setup-bun.

  The version is the content version, <claude-agent-sdk version>-<first 7 of
  the BubblePaw-Core commit>; it names the install folder. The zip is built
  with fixed timestamps and sorted entries so the same inputs give the same
  bytes where the .NET deflater allows; the app decides on the sha256 anyway.

.PARAMETER OutDir
  Folder for the zip and manifest.json (created).

.PARAMETER BaseUrl
  HTTPS URL of the release's download folder, for example
  https://github.com/BubbleBuffer/Scratch/releases/download/v0.1.0-prealpha.12.

.PARAMETER Platform
  The platform key. Default: windows-x64.

.PARAMETER Bun
  The Bun executable to ship. Default: `bun` on PATH.

.PARAMETER Source
  The BubblePaw-Core repository.

.PARAMETER LockFile
  The Cargo.lock naming the pinned BubblePaw-Core commit. Default: this repo's.

.EXAMPLE
  ./app/windows/installer/build-package-claude-code.ps1 -OutDir dist/packages -BaseUrl https://github.com/BubbleBuffer/Scratch/releases/download/v0.1.0-prealpha.12
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$OutDir,
    [Parameter(Mandatory = $true)][string]$BaseUrl,
    [string]$Platform = 'windows-x64',
    [string]$Bun,
    [string]$Source = 'https://github.com/BubbleBuffer/BubblePaw-Core',
    [string]$LockFile = (Join-Path $PSScriptRoot '../../../Cargo.lock')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Errors are one line on stderr and a non-zero exit.
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

if (-not $BaseUrl.StartsWith('https://')) {
    Stop-With "BaseUrl must be an https:// URL: $BaseUrl"
}
$BaseUrl = $BaseUrl.TrimEnd('/')

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

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("scratch-pkg-claude-code-" + [guid]::NewGuid().ToString('N'))
$stage = Join-Path $work 'stage'
$extension = Join-Path $stage 'claude-code'
$bunDir = Join-Path $stage 'bun'
try {
    New-Item -ItemType Directory -Path $stage | Out-Null
    $repo = Join-Path $work 'repo'
    New-Item -ItemType Directory -Path $repo | Out-Null
    # The extension folder at the pinned commit, without the rest of the tree.
    Invoke-Git @('-C', $repo, 'init', '--quiet')
    Invoke-Git @('-C', $repo, 'remote', 'add', 'origin', $Source)
    Invoke-Git @('-C', $repo, 'fetch', '--quiet', '--depth', '1', '--filter=blob:none', 'origin', $revision)
    Invoke-Git @('-C', $repo, 'checkout', '--quiet', '--detach', 'FETCH_HEAD')

    $from = Join-Path $repo 'extensions/claude-code'
    foreach ($file in @('package.json', 'bun.lock', 'NOTICE')) {
        if (-not (Test-Path -LiteralPath (Join-Path $from $file))) {
            Stop-With "BubblePaw-Core $revision has no extensions/claude-code/$file"
        }
    }
    New-Item -ItemType Directory -Path $extension | Out-Null
    foreach ($file in @('package.json', 'bun.lock', 'NOTICE')) {
        Copy-Item -LiteralPath (Join-Path $from $file) -Destination $extension
    }
    Copy-Item -LiteralPath (Join-Path $from 'src') -Destination $extension -Recurse

    Push-Location $extension
    try {
        & $Bun install --production --frozen-lockfile
        if ($LASTEXITCODE -ne 0) {
            Stop-With "bun install --production --frozen-lockfile failed ($LASTEXITCODE)"
        }
    } finally {
        Pop-Location
    }

    New-Item -ItemType Directory -Path $bunDir | Out-Null
    Copy-Item -LiteralPath $Bun -Destination (Join-Path $bunDir 'bun.exe')

    $required = @(
        (Join-Path $extension 'package.json'),
        (Join-Path $extension 'src/main.ts'),
        (Join-Path $extension 'node_modules/@anthropic-ai/claude-agent-sdk/package.json'),
        (Join-Path $bunDir 'bun.exe')
    )
    $missing = @($required | Where-Object { -not (Test-Path -LiteralPath $_) })
    if ($missing.Count -gt 0) {
        Stop-With ("the claude-code package is incomplete, missing: " + ($missing -join ', '))
    }

    $sdk = (Get-Content -Raw -LiteralPath $required[2] | ConvertFrom-Json).version
    if (-not $sdk) {
        Stop-With 'the claude-agent-sdk package.json has no version'
    }
    $version = "$sdk-$($revision.Substring(0, 7))"
    $asset = "scratch-pkg-claude-code-$version-$Platform.zip"
    $zipPath = Join-Path $OutDir $asset
    if (Test-Path -LiteralPath $zipPath) {
        Remove-Item -LiteralPath $zipPath -Force
    }

    # Sorted entries, fixed timestamps, forward slashes. The app refuses zip64,
    # so the entry count stays below 65535.
    Add-Type -AssemblyName System.IO.Compression
    $prefix = $stage.TrimEnd('\') + '\'
    $files = @(Get-ChildItem -LiteralPath $stage -Recurse -File -Force |
        ForEach-Object { [pscustomobject]@{ Full = $_.FullName; Rel = $_.FullName.Substring($prefix.Length).Replace('\', '/') } } |
        Sort-Object -Property Rel -CaseSensitive)
    if ($files.Count -ge 65000) {
        Stop-With "the claude-code package has $($files.Count) files; the limit is 65000"
    }
    $stamp = [DateTimeOffset]::new(2000, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
    $stream = [System.IO.File]::Create($zipPath)
    try {
        $zip = New-Object System.IO.Compression.ZipArchive($stream, [System.IO.Compression.ZipArchiveMode]::Create)
        try {
            foreach ($f in $files) {
                $entry = $zip.CreateEntry($f.Rel, [System.IO.Compression.CompressionLevel]::Optimal)
                $entry.LastWriteTime = $stamp
                $in = [System.IO.File]::OpenRead($f.Full)
                $out = $entry.Open()
                try { $in.CopyTo($out) } finally { $out.Dispose(); $in.Dispose() }
            }
        } finally {
            $zip.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
} finally {
    if (Test-Path -LiteralPath $work) {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }
}

$item = Get-Item -LiteralPath $zipPath
$entryJson = [ordered]@{
    name     = 'claude-code'
    version  = $version
    platform = $Platform
    url      = "$BaseUrl/$asset"
    sha256   = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    size     = $item.Length
}

$manifestPath = Join-Path $OutDir 'manifest.json'
$packages = @()
if (Test-Path -LiteralPath $manifestPath) {
    $existing = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
    $packages = @($existing.packages | Where-Object { -not ($_.name -eq 'claude-code' -and $_.platform -eq $Platform) })
}
$packages += [pscustomobject]$entryJson
$json = [ordered]@{ packages = $packages } | ConvertTo-Json -Depth 5
[System.IO.File]::WriteAllText($manifestPath, $json + "`n", (New-Object System.Text.UTF8Encoding($false)))

Write-Output ("claude-code package: {0} ({1} MB, {2} files)" -f $asset, [math]::Round($item.Length / 1MB), $files.Count)
Write-Output 'claude-code package ok'
