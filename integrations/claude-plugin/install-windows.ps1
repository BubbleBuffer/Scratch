# Switches Claude Code on this PC from built-in memory to the notes vault.
# Run from PowerShell 7 with Claude Code closed:
#   pwsh -File V:\projects\notes-app\integrations\claude-plugin\install-windows.ps1
# Safe to re-run. The agent token is read from the notes CLI config and never printed.

$ErrorActionPreference = 'Stop'

# Windows PowerShell 5.1 lacks the JSON and encoding options used below: rerun in pwsh 7.
if ($PSVersionTable.PSVersion.Major -lt 7) {
    if (-not (Get-Command pwsh -ErrorAction SilentlyContinue)) { throw 'PowerShell 7 (pwsh) is required' }
    & pwsh -NoProfile -File $PSCommandPath
    exit $LASTEXITCODE
}

$repo    = 'V:\projects\notes-app'
$cliDir   = 'F:\build\notes-cli'
$settings = Join-Path $HOME '.claude\settings.json'
$cliCfg   = Join-Path $env:APPDATA 'notes\config.toml'

# The hub URL comes from the notes CLI config (there is no built-in hub).
$cfg = Get-Content -LiteralPath $cliCfg -Raw -ErrorAction SilentlyContinue
$hubUrl = $null
if ($cfg -and $cfg -match '(?m)^\s*hub\s*=\s*"([^"]+)"') { $hubUrl = $Matches[1].TrimEnd('/') }
if (-not $hubUrl) { $hubUrl = (Read-Host "Hub URL (for example https://notes.example.com)").Trim().TrimEnd('/') }
if (-not $hubUrl) { throw 'No hub URL given' }

# 1. notes CLI on the user PATH
if (-not (Test-Path "$cliDir\notes.exe")) { throw "notes.exe not found in $cliDir" }
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $cliDir) {
    [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ";$cliDir"), 'User')
    Write-Host "PATH: added $cliDir (new terminals only)"
} else {
    Write-Host "PATH: already has $cliDir"
}
$env:Path += ";$cliDir"

# 2. Marketplace and plugin (the folder marketplace reads the repo checkout directly)
$markets = (claude plugin marketplace list) | Out-String
if ($markets -notmatch 'notes-app') {
    claude plugin marketplace add "$repo\integrations"
} else {
    claude plugin marketplace update notes-app
}
$plugins = (claude plugin list) | Out-String
if ($plugins -notmatch 'notes@notes-app') {
    claude plugin install notes@notes-app --config "hub_url=$hubUrl"
}

# 3. Plugin options: hub URL and the agent token from the CLI config
$token = $null
if ($cfg -and $cfg -match '(?m)^\s*token\s*=\s*"([^"]+)"') { $token = $Matches[1] }
if (-not $token) {
    Write-Host "No token readable in $cliCfg."
    $secure = Read-Host 'Paste the agent token (input hidden)' -AsSecureString
    $token  = [Net.NetworkCredential]::new('', $secure).Password
    if (-not $token) { throw 'No token given' }
}
@{ hub_url = $hubUrl; hub_token = $token } | ConvertTo-Json -Compress |
    claude plugin configure notes@notes-app --values-stdin | Out-Null
$Matches = $null; $token = $null
Write-Host 'Plugin options: set (token not shown)'

# 4. Turn off built-in auto memory (backup first)
$json = if (Test-Path $settings) { Get-Content $settings -Raw } else { '{}' }
$obj  = $json | ConvertFrom-Json -AsHashtable
if ($obj['autoMemoryEnabled'] -ne $false) {
    if (Test-Path $settings) { Copy-Item $settings "$settings.bak-notes-memory" -Force }
    $obj['autoMemoryEnabled'] = $false
    $obj | ConvertTo-Json -Depth 100 | Set-Content $settings -Encoding utf8NoBOM
    Write-Host "settings.json: autoMemoryEnabled = false (backup: settings.json.bak-notes-memory)"
} else {
    Write-Host 'settings.json: autoMemoryEnabled already false'
}

# 5. Check
Write-Host ''
claude plugin list | Select-String -Context 0,4 'notes@notes-app'
Write-Host ''
Write-Host 'Memory index the next session will see:'
& "$cliDir\notes.exe" memory index --project global --max-chars 8000
