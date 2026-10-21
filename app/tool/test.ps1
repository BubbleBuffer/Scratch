param(
    [ValidateSet('fast', 'golden', 'perf', 'all')]
    [string]$Tier = 'fast',
    [string]$ReportDir
)

$ErrorActionPreference = 'Stop'
$appRoot = Split-Path $PSScriptRoot -Parent
$repoRoot = Split-Path $appRoot -Parent
$targetRoot = Join-Path $repoRoot 'target/release'
$bridge = Join-Path $targetRoot 'notes_bridge.dll'
$agent = Join-Path $targetRoot 'examples/fake_agent.exe'
$sources = @(
    Get-ChildItem (Join-Path $repoRoot 'crates'), (Join-Path $appRoot 'rust') -Recurse -File |
        Where-Object { $_.Extension -eq '.rs' -or $_.Name -eq 'Cargo.toml' }
    Get-Item (Join-Path $repoRoot 'Cargo.toml'), (Join-Path $repoRoot 'Cargo.lock')
)
$newest = ($sources | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1).LastWriteTimeUtc
$oldBridge = $env:NOTES_BRIDGE_LIB
$oldAgent = $env:NOTES_FAKE_AGENT
$oldIncremental = $env:CARGO_INCREMENTAL
$env:CARGO_INCREMENTAL = '0'
Push-Location $repoRoot
try {
    if (!(Test-Path $bridge) -or (Get-Item $bridge).LastWriteTimeUtc -lt $newest) {
        & cargo build --release -p notes_bridge
        if ($LASTEXITCODE -ne 0) { throw 'Bridge build failed' }
    }
    if (!(Test-Path $agent) -or (Get-Item $agent).LastWriteTimeUtc -lt $newest) {
        & cargo build --release -p vault_mcp --examples
        if ($LASTEXITCODE -ne 0) { throw 'Fake agent build failed' }
    }
    $env:NOTES_BRIDGE_LIB = $bridge
    $env:NOTES_FAKE_AGENT = $agent
    Set-Location $appRoot
    $flutter = if (Get-Command flutter -ErrorAction SilentlyContinue) { 'flutter' } else { (Join-Path $HOME 'flutter\bin\flutter.bat') }
    $tiers = if ($Tier -eq 'all') { @('fast', 'golden', 'perf') } else { @($Tier) }
    $total = 0.0
    $results = @()
    if ($ReportDir) { New-Item -ItemType Directory -Force $ReportDir | Out-Null }
    foreach ($part in $tiers) {
        $watch = [Diagnostics.Stopwatch]::StartNew()
        if ($part -eq 'perf') {
            Push-Location $repoRoot
            try {
                & cargo test --release -p vault_core --test search_perf -- --ignored --nocapture
                if ($LASTEXITCODE -ne 0) { throw 'Core search performance tests failed' }
            } finally {
                Pop-Location
            }
        }
        $arguments = switch ($part) {
            'fast' { @('test', '-x', 'golden || perf || drive') }
            'golden' { @('test', '-t', 'golden') }
            'perf' { @('test', '-t', 'perf', '--concurrency=1') }
        }
        if ($ReportDir) {
            & $flutter @arguments --reporter=json > (Join-Path $ReportDir "$part.jsonl")
        } else {
            & $flutter @arguments
        }
        $code = $LASTEXITCODE
        $watch.Stop()
        $seconds = $watch.Elapsed.TotalSeconds
        $total += $seconds
        $results += @{ tier = $part; seconds = $seconds; exitCode = $code }
        Write-Host ('{0}: {1:N2}s (exit {2})' -f $part, $seconds, $code)
        if ($ReportDir) { $results | ConvertTo-Json -AsArray | Set-Content (Join-Path $ReportDir 'timings.json') }
        if ($code -ne 0) { throw "$part tests failed (exit $code)" }
    }
    Write-Host ('Total tests: {0:N2}s' -f $total)
} finally {
    $env:NOTES_BRIDGE_LIB = $oldBridge
    $env:NOTES_FAKE_AGENT = $oldAgent
    $env:CARGO_INCREMENTAL = $oldIncremental
    Pop-Location
}
