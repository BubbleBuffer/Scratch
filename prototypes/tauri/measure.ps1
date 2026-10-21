<#
.SYNOPSIS
  Bake-off measurements for the Tauri prototype on an Android emulator
  (the Bake-off spec in the internal wiki; current state: wiki/prototypes/tauri.md).

.DESCRIPTION
  Runs the spec's steps against one device and prints a summary:
    1. cold start: `am start -W` TotalTime, median of 5 after force-stop
       (plus the app's own `list_visible` time from BAKEOFF lines);
    2. list scrolling: 20 `input swipe` flings, frame times from the app;
    3. big page: open it (page_open), 20 flings (frame times);
    4. typing: 200 characters via `input text` into the big page (latency);
    5. memory: `dumpsys meminfo` TOTAL PSS with the big page open;
    6. APK size.
  The app logs `BAKEOFF ...` lines to logcat (tag BAKEOFF); this script reads
  them with `adb logcat -d -s BAKEOFF`.

  The first launch after install seeds the vault (10,000 pages + the 2,000-block
  big page); the script waits for it before measuring. Use -Fresh to clear the
  app's data first.

.EXAMPLE
  ./measure.ps1 -Serial emulator-5570 -Apk src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk -Install -Fresh
#>
param(
  [string]$Serial = "",
  [string]$Apk = "",
  [string]$Package = "dev.notesapp.tauri",
  [string]$Activity = ".MainActivity",
  [switch]$Install,
  [switch]$Fresh,
  [int]$Swipes = 20,
  [int]$Chars = 200,
  [string]$Adb = "$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe"
)
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $Apk) { $Apk = Join-Path $here "src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk" }
$dev = @(); if ($Serial) { $dev = @("-s", $Serial) }

function A { & $Adb @dev @args }
function Log-Lines { (A logcat -d -s BAKEOFF:I) | Where-Object { $_ -match "BAKEOFF" } }
function Last-Line([string]$pattern) { Log-Lines | Where-Object { $_ -match $pattern } | Select-Object -Last 1 }
function Wait-Line([string]$pattern, [int]$timeoutS = 120) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  while ($sw.Elapsed.TotalSeconds -lt $timeoutS) {
    $l = Last-Line $pattern
    if ($l) { return $l }
    Start-Sleep -Milliseconds 300
  }
  throw "timed out waiting for '$pattern'"
}
function Field([string]$line, [string]$name) { if ($line -match "$name=([0-9.]+)") { [double]$Matches[1] } else { $null } }
function Median($xs) { $s = $xs | Sort-Object; $s[[int][math]::Floor(($s.Count - 1) / 2)] }

# Screen geometry for swipes and taps.
$size = (A shell wm size) -replace ".*: ", "" | Select-Object -Last 1
$w, $h = $size.Trim().Split("x") | ForEach-Object { [int]$_ }
$cx = [int]($w / 2)

function Fling-Up([int]$n) {
  for ($i = 0; $i -lt $n; $i++) {
    A shell input swipe $cx ([int]($h * 0.75)) $cx ([int]($h * 0.25)) 120 | Out-Null
    Start-Sleep -Milliseconds 700
  }
  Start-Sleep -Milliseconds 800
}

# Taps the centre of the first accessibility node whose text matches.
function Tap-Text([string]$text) {
  $m = $null
  for ($try = 0; $try -lt 8; $try++) {
    A shell uiautomator dump /sdcard/ui.xml | Out-Null
    $xml = (A shell cat /sdcard/ui.xml) -join ""
    $m = [regex]::Match($xml, "text=`"$([regex]::Escape($text))`"[^>]*?bounds=`"\[(\d+),(\d+)\]\[(\d+),(\d+)\]`"")
    if ($m.Success) { break }
    Start-Sleep -Seconds 2
  }
  if (-not $m.Success) { throw "no node with text '$text' on screen" }
  $x = ([int]$m.Groups[1].Value + [int]$m.Groups[3].Value) / 2
  $y = ([int]$m.Groups[2].Value + [int]$m.Groups[4].Value) / 2
  A shell input tap ([int]$x) ([int]$y) | Out-Null
}

$result = [ordered]@{}

if ($Install) { A install -r $Apk | Out-Host }
if ($Fresh) { A shell pm clear $Package | Out-Null }

# First launch (seeds the vault when needed).
A shell am force-stop $Package | Out-Null
A logcat -c
A shell am start -n "$Package/$Activity" | Out-Null
$seed = Wait-Line "list_visible" 600
$seedLine = Last-Line "seed ms="
if ($seedLine) { $result["seed_ms"] = Field $seedLine "ms" }
Start-Sleep -Seconds 3

# 1. Cold start, median of 5.
$total = @(); $visible = @()
for ($i = 0; $i -lt 5; $i++) {
  A shell am force-stop $Package | Out-Null
  Start-Sleep -Seconds 2
  A logcat -c
  $out = A shell am start -W -n "$Package/$Activity"
  $t = ($out | Where-Object { $_ -match "TotalTime" }) -replace "\D", ""
  $total += [int]$t
  $visible += Field (Wait-Line "list_visible" 60) "ms"
  Start-Sleep -Seconds 1
}
$result["cold_start_TotalTime_ms_median"] = Median $total
$result["cold_start_TotalTime_ms_all"] = ($total -join ", ")
$result["list_visible_ms_median (webview time origin)"] = Median $visible
Start-Sleep -Seconds 2

# 2. Scrolling the list.
Fling-Up $Swipes
$result["list_frames"] = (Last-Line "frames surface=list") -replace ".*BAKEOFF ", ""

# 3. Big page: back to the top of the list, open it, scroll it.
A shell am force-stop $Package | Out-Null
A logcat -c
A shell am start -n "$Package/$Activity" | Out-Null
Wait-Line "list_visible" 60 | Out-Null
Start-Sleep -Seconds 2
Tap-Text "Big page"
$result["page_open"] = (Wait-Line "page_open" 30) -replace ".*BAKEOFF ", ""
$result["page_rendered"] = (Wait-Line "page_rendered" 30) -replace ".*BAKEOFF ", ""
Start-Sleep -Seconds 2
Fling-Up $Swipes
$result["page_frames"] = (Last-Line "frames surface=page") -replace ".*BAKEOFF ", ""

# 4. Typing: tap into a block near the middle of the screen, type 200 letters.
A shell input tap $cx ([int]($h * 0.45)) | Out-Null
Start-Sleep -Seconds 2
A shell input keyevent KEYCODE_MOVE_END | Out-Null
$alphabet = "abcdefghijklmnopqrstuvwxyz"
$typed = 0
while ($typed -lt $Chars) {
  $n = [math]::Min(10, $Chars - $typed)
  $chunk = -join (0..($n - 1) | ForEach-Object { $alphabet[($typed + $_) % 26] })
  A shell input text $chunk | Out-Null
  $typed += $n
}
Start-Sleep -Seconds 2
$result["input"] = (Last-Line "input n=") -replace ".*BAKEOFF ", ""

# 5. Memory with the big page open.
$mem = A shell dumpsys meminfo $Package
$pss = ($mem | Where-Object { $_ -match "TOTAL PSS:" }) -replace ".*TOTAL PSS:\s*(\d+).*", '$1'
if (-not $pss) { $pss = ($mem | Where-Object { $_ -match "^\s*TOTAL\s" } | Select-Object -First 1) -replace "^\s*TOTAL\s+(\d+).*", '$1' }
$result["total_pss_kb"] = $pss

# 6. APK size.
if (Test-Path $Apk) { $result["apk_bytes"] = (Get-Item $Apk).Length }

A shell input keyevent KEYCODE_BACK | Out-Null
A shell input keyevent KEYCODE_BACK | Out-Null

$result.GetEnumerator() | ForEach-Object { "{0,-48} {1}" -f $_.Key, $_.Value }
