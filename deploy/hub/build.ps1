#requires -Version 7.4
[CmdletBinding()]
param(
    [string] $ContextPath = (Join-Path $PSScriptRoot '../..'),
    [string] $BubblePawPath = (Join-Path $PSScriptRoot '../../../BubblePaw-base-agent')
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$repoPath = (Resolve-Path -LiteralPath $ContextPath).Path
$bubblePawContext = (Resolve-Path -LiteralPath $BubblePawPath).Path
$dockerfilePath = Join-Path $PSScriptRoot 'Dockerfile'
$archiveDir = Join-Path (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path 'target'
New-Item -ItemType Directory -Path $archiveDir -Force | Out-Null
$archivePath = Join-Path $archiveDir 'notes-hub.tar.gz'

$docker = (Get-Command docker.exe -ErrorAction Stop).Source
$gzip = (Get-Command gzip.exe -ErrorAction Stop).Source
& $docker buildx build --platform linux/amd64 --load --tag notes-hub:latest `
    --build-context "bubblepaw=$bubblePawContext" --file $dockerfilePath $repoPath
# PowerShell 7.4+ preserves the byte stream between native commands and redirection.
& $docker save notes-hub:latest | & $gzip > $archivePath
& $gzip --test $archivePath
Write-Host "Image archive: $archivePath"
& $docker image inspect notes-hub:latest --format '{{.Os}}/{{.Architecture}} {{.Size}} bytes'
