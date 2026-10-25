#requires -Version 7.4
[CmdletBinding()]
param([string] $Image = 'notes-hub:latest')

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$suffix = [guid]::NewGuid().ToString('N').Substring(0, 12)
$container = "notes-hub-verify-$suffix"
$volume = "notes-hub-verify-$suffix"
$tempDir = Join-Path ([IO.Path]::GetTempPath()) $container
New-Item -ItemType Directory -Path $tempDir | Out-Null
$createdVolume = $false
$createdContainer = $false

function Assert([bool] $Condition, [string] $Message) {
    if (!$Condition) { throw $Message }
}

function Read-Var([byte[]] $Bytes, [ref] $Index) {
    [System.Numerics.BigInteger] $value = 0
    for ($shift = 0; $shift -lt 70; $shift += 7) {
        Assert ($Index.Value -lt $Bytes.Length) 'Truncated sync response'
        $b = $Bytes[$Index.Value]
        $Index.Value++
        $value = $value -bor ([System.Numerics.BigInteger]($b -band 127) -shl $shift)
        if (($b -band 128) -eq 0) { return $value }
    }
    throw 'Invalid sync varint'
}

try {
    docker volume create $volume | Out-Null
    $createdVolume = $true
    docker run -d --name $container --mount "type=volume,src=$volume,dst=/data" `
        -p '127.0.0.1::8080' --memory 1g --pids-limit 512 --security-opt no-new-privileges:true `
        -e NOTES_HUB_PUBLIC_URL=https://notes.example.com `
        -e NOTES_HUB_OIDC_ISSUER=https://fake-issuer.invalid/application/o/notes-hub/ `
        -e NOTES_HUB_OIDC_CLIENT_ID=local-smoke -e NOTES_HUB_OIDC_CLIENT_SECRET=synthetic-secret `
        -e NOTES_HUB_OIDC_ALLOWED_GROUPS=notes-users $Image | Out-Null
    $createdContainer = $true
    $port = (docker inspect $container --format '{{(index (index .NetworkSettings.Ports "8080/tcp") 0).HostPort}}').Trim()
    $url = "http://127.0.0.1:$port"
    $ready = $false
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        try {
            $health = Invoke-WebRequest "$url/health" -TimeoutSec 2
            if ($health.StatusCode -eq 200 -and $health.Content -eq 'notes-hub') { $ready = $true; break }
        } catch { Start-Sleep -Seconds 2 }
    }
    Assert $ready 'Hub did not become healthy'
    $healthBody = curl.exe --fail --silent --show-error "$url/health"
    Assert ($healthBody -eq 'notes-hub') 'Host curl health response was unexpected'
    Write-Host "PASS health: HTTP 200 notes-hub ($url)"
    $uid = (docker exec $container id -u).Trim()
    Assert ($uid -eq '10001') "Unexpected runtime uid: $uid"
    docker exec $container sh -c 'test "$HOME" = /data/home && test -f /data/hub.toml && test -d /data/home/.claude && test -d /data/vault && touch /data/home/.claude/smoke-write'
    Write-Host 'PASS uid=10001, HOME=/data/home, volume config/vault/CLI homes and writable CLI homes'
    docker exec $container notes-hub --devices
    Write-Host 'PASS notes-hub --devices (empty before pairing)'
    $pins = @{ claude = '2.1.293'; bun = '1.4.2'; node = '22.18.0' }
    foreach ($cli in @('claude', 'bun', 'node')) {
        $version = docker exec $container $cli --version
        Assert (($version -join "`n").Contains($pins[$cli])) "Wrong $cli version: $version"
        Write-Host "PASS $cli --version: $version"
    }
    docker exec $container find /data -maxdepth 3 -printf '%u:%g %p\n'
    $token = (docker exec $container notes-hub --config /data/hub.toml --pair test).Trim()
    Assert (![string]::IsNullOrEmpty($token)) 'Emergency pair did not return a token'
    # The CLI opens a separate token store. Restart this test server to reload it.
    docker restart $container | Out-Null
    # Docker may choose a new host port for an ephemeral mapping on restart.
    $port = (docker inspect $container --format '{{(index (index .NetworkSettings.Ports "8080/tcp") 0).HostPort}}').Trim()
    $url = "http://127.0.0.1:$port"
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        try { Invoke-WebRequest "$url/health" -TimeoutSec 2 | Out-Null; break }
        catch { Start-Sleep -Seconds 2 }
    }
    $request = Join-Path $tempDir 'sync-request.bin'
    $response = Join-Path $tempDir 'sync-response.bin'
    [IO.File]::WriteAllBytes($request, [byte[]]@(1, 1, 0, 0, 0, 0, 0, 0))
    # Send the synthetic bearer token in curl's stdin config, never in tool output.
    $curlConfig = "header = `"Authorization: Bearer $token`"`nheader = `"X-Device: local-smoke-$suffix`""
    $status = $curlConfig | curl.exe --silent --show-error --fail --config - `
        --header 'Content-Type: application/octet-stream' --data-binary "@$request" `
        --output $response --write-out '%{http_code}' "$url/sync"
    $token = $null
    Assert ($status -eq '200') "Sync HTTP status: $status"
    $bytes = [IO.File]::ReadAllBytes($response)
    Assert ($bytes.Length -gt 0 -and $bytes[0] -eq 1) 'Wrong sync protocol version'
    $index = 1
    $epoch = Read-Var $bytes ([ref] $index)
    Assert ($epoch -gt 0) 'Sync epoch is zero'
    foreach ($field in @('seq', 'reset', 'updates', 'versions', 'need', 'changed', 'purged', 'purged_total', 'need_blobs', 'more')) {
        $value = Read-Var $bytes ([ref] $index)
        Assert ($value -eq 0) "Empty full sync did not converge: $field=$value"
    }
    Assert ($index -eq $bytes.Length) 'Sync response has trailing bytes'
    Write-Host 'PASS emergency --pair test plus host curl POST /sync: HTTP 200, valid v1 response, empty full sync converged'
    $devices = docker exec $container notes-hub --devices
    Assert (($devices -join "`n").Contains("test`tlocal-smoke-$suffix")) 'Token did not bind to the test device'
    Write-Host "PASS device binding: $devices"
    docker exec $container sh -c 'test -f /data/home/.claude/smoke-write'
    Write-Host 'PASS CLI-home writes persisted across test container restart'
    # Wait for the image HEALTHCHECK itself, in addition to the host request.
    $healthStatus = ''
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        $healthStatus = (docker inspect $container --format '{{.State.Health.Status}}').Trim()
        if ($healthStatus -eq 'healthy') { break }
        Start-Sleep -Seconds 2
    }
    Assert ($healthStatus -eq 'healthy') "Docker HEALTHCHECK state: $healthStatus"
    Write-Host 'PASS Docker HEALTHCHECK: healthy'
    docker logs $container
} finally {
    if ($createdContainer) { docker rm -f $container | Out-Null }
    if ($createdVolume) { docker volume rm $volume | Out-Null }
    $resolvedTemp = [IO.Path]::GetFullPath($tempDir)
    $expectedRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if ($resolvedTemp.StartsWith($expectedRoot, [StringComparison]::OrdinalIgnoreCase) -and
        [IO.Path]::GetFileName($resolvedTemp) -eq $container) {
        Remove-Item -LiteralPath $resolvedTemp -Recurse -Force
    }
    Write-Host "Cleaned up test container and volume: $container"
}
