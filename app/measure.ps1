<#
.SYNOPSIS
  Bake-off measurements for the Flutter app on an Android emulator
  (the Bake-off spec in the internal wiki; current state: wiki/prototypes/flutter.md).

.DESCRIPTION
  Expects one freshly booted emulator (or pass -Serial) and a release APK
  built with the bench vault and the instrumentation on:
    flutter build apk --release --target-platform android-x64 `
      --dart-define=SEED_BENCH=true --dart-define=BAKEOFF=true
  Steps: install, first launch (seeds the vault, waits for the list), cold
  start x5, 20 flings on the list, open "Big page", 20 flings on it, memory,
  200 typed characters, APK size and build folder sizes. The app logs
  BAKEOFF lines (logcat tag "flutter"); this script reads them back.

  Tap targets come from the app's own "BAKEOFF layout" lines, so the script
  does not depend on the screen size.

.EXAMPLE
  ./measure.ps1 -Apk build/app/outputs/flutter-apk/app-release.apk
#>
param(
  [string]$Apk = "$PSScriptRoot/build/app/outputs/flutter-apk/app-release.apk",
  [string]$Serial = "",
  [string]$Package = "dev.notesapp.app",
  [string]$Activity = ".MainActivity",
  [int]$Flings = 20,
  [int]$TypeChars = 200,
  [switch]$SkipInstall,
  [string]$Out = "$PSScriptRoot/measure-results.txt"
)
$ErrorActionPreference = "Stop"
$adbExe = Join-Path $env:LOCALAPPDATA "Android/Sdk/platform-tools/adb.exe"
if (-not (Test-Path $adbExe)) { $adbExe = "adb" }
function Adb { if ($Serial) { & $adbExe -s $Serial @args } else { & $adbExe @args } }
$results = [System.Collections.Generic.List[string]]::new()
function Note($s) { Write-Host $s; $results.Add($s) }

function Logcat { (Adb logcat -d -s flutter:I) -join "`n" }
function BakeoffLines { (Logcat) -split "`n" | Where-Object { $_ -match "BAKEOFF" } }
function LastLine($pattern) { BakeoffLines | Where-Object { $_ -match $pattern } | Select-Object -Last 1 }
function WaitFor($pattern, [int]$timeoutSec = 180) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  while ($sw.Elapsed.TotalSeconds -lt $timeoutSec) {
    $l = LastLine $pattern
    if ($l) { return $l }
    Start-Sleep -Milliseconds 500
  }
  throw "Timed out waiting for '$pattern'"
}
function Field($line, $name) { if ($line -match "$name=(-?[\d\.]+)") { return [double]$Matches[1] } return $null }
function Fling($x, $yFrom, $yTo) { Adb shell input swipe $x $yFrom $x $yTo 120 | Out-Null }

Note "== Flutter bake-off measurements $(Get-Date -Format s)"
Note "device: $((Adb shell getprop ro.product.model).Trim()) API $((Adb shell getprop ro.build.version.sdk).Trim())"

# ---- install and first launch (seeding) ----
if (-not $SkipInstall) {
  Adb install -r $Apk | Out-Null
}
Adb shell am force-stop $Package | Out-Null
Adb logcat -c
Adb shell am start -W -n "$Package/$Activity" | Out-Null
$seed = WaitFor "BAKEOFF list-visible" 600
$seeded = LastLine "BAKEOFF seeded"
if ($seeded) { Note "first launch: $seeded" }
Start-Sleep -Seconds 3

# ---- cold start, median of 5 ----
$times = @()
for ($i = 0; $i -lt 5; $i++) {
  Adb shell am force-stop $Package | Out-Null
  Start-Sleep -Seconds 2
  $o = Adb shell am start -W -n "$Package/$Activity"
  $t = ($o | Select-String "TotalTime: (\d+)").Matches[0].Groups[1].Value
  $times += [int]$t
  Start-Sleep -Seconds 3
}
$sorted = $times | Sort-Object
Note "cold start TotalTime ms: $($times -join ', ') -> median $($sorted[2])"

# App is now on the list (last cold start). Read its layout.
Adb logcat -c
Adb shell am force-stop $Package | Out-Null
Adb shell am start -W -n "$Package/$Activity" | Out-Null
$layout = WaitFor "BAKEOFF layout dpr" 60
$x = [int](Field $layout "first-row-x"); $rowY = [int](Field $layout "first-row-y")
$top = [int](Field $layout "list-top"); $bottom = [int](Field $layout "list-bottom")
# The showcase is created last, so "Big page" is usually the second row.
$rowH = [int](Field $layout "row-h"); $bigIdx = [int](Field $layout "big-index")
if ($bigIdx -lt 0) { throw "Big page not in the first 20 rows" }
$bigY = $rowY + $bigIdx * $rowH
Start-Sleep -Seconds 2

# ---- list scrolling ----
$y1 = [int]($bottom - ($bottom - $top) * 0.15); $y2 = [int]($top + ($bottom - $top) * 0.15)
for ($i = 0; $i -lt $Flings; $i++) {
  if ($i % 2 -eq 0) { Fling $x $y1 $y2 } else { Fling $x $y1 $y2 }
  Start-Sleep -Milliseconds 700
}
Start-Sleep -Seconds 4
Note "list scrolling: $(LastLine 'frames-total screen=list')"

# Back to the top so "Big page" is at big-index again.
Adb shell am force-stop $Package | Out-Null
Adb logcat -c
Adb shell am start -W -n "$Package/$Activity" | Out-Null
WaitFor "BAKEOFF layout dpr" 60 | Out-Null
Start-Sleep -Seconds 2

# ---- open the big page ----
Adb shell input tap $x $bigY | Out-Null
$open = WaitFor "BAKEOFF page-open" 30
Note "page open: $open"
$pl = WaitFor "BAKEOFF layout page" 30
$tx = [int](Field $pl "text-x"); $ty = [int](Field $pl "text-y")
$ptop = [int](Field $pl "list-top"); $pbottom = [int](Field $pl "list-bottom")
Start-Sleep -Seconds 2

# ---- memory with the big page open ----
$mem = Adb shell dumpsys meminfo $Package
$pss = ($mem | Select-String "TOTAL PSS:\s+(\d+)").Matches
if (-not $pss) { $pss = ($mem | Select-String "TOTAL\s+(\d+)").Matches }
Note "memory TOTAL PSS KB (big page open): $($pss[0].Groups[1].Value)"

# ---- typing 200 characters in the big page ----
Adb shell input tap $tx ($ty + 110) | Out-Null
Start-Sleep -Seconds 2
$focus = LastLine "BAKEOFF focus"
Note "typing target: $focus"
$alphabet = "abcdefghijklmnopqrstuvwxyz"
$sent = 0
while ($sent -lt $TypeChars) {
  $n = [Math]::Min(20, $TypeChars - $sent)
  $chunk = -join (0..($n - 1) | ForEach-Object { $alphabet[($sent + $_) % 26] })
  Adb shell input text $chunk | Out-Null
  $sent += $n
}
Start-Sleep -Seconds 4
Note "typing: $(LastLine 'BAKEOFF input-total')"
# Close the soft keyboard (one BACK) before scrolling.
if ((Adb shell dumpsys input_method) -match "mInputShown=true") { Adb shell input keyevent KEYCODE_BACK | Out-Null; Start-Sleep -Seconds 2 }

# ---- big page scrolling ----
$py1 = [int]($pbottom - ($pbottom - $ptop) * 0.15); $py2 = [int]($ptop + ($pbottom - $ptop) * 0.15)
for ($i = 0; $i -lt $Flings; $i++) {
  if ($i -lt $Flings / 2) { Fling $tx $py1 $py2 } else { Fling $tx $py2 $py1 }
  Start-Sleep -Milliseconds 700
}
Start-Sleep -Seconds 4
Note "big page scrolling: $(LastLine 'frames-total screen=page')"

# ---- sizes ----
if (Test-Path $Apk) { Note ("APK size: {0:N1} MB ({1} bytes)" -f ((Get-Item $Apk).Length / 1MB), (Get-Item $Apk).Length) }
function DirMB($p) { if (Test-Path $p) { "{0:N0} MB" -f ((Get-ChildItem $p -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum / 1MB) } else { "n/a" } }
$root = Resolve-Path "$PSScriptRoot/../.."
Note "build/: $(DirMB "$PSScriptRoot/build")"
Note ".dart_tool/: $(DirMB "$PSScriptRoot/.dart_tool")"
Note "android/.gradle + app build: $(DirMB "$PSScriptRoot/android/.gradle")"
Note "target/x86_64-linux-android (shared target/, Flutter bridge is its main user): $(DirMB "$root/target/x86_64-linux-android")"
Note "target/x86_64-pc-windows-msvc: $(DirMB "$root/target/x86_64-pc-windows-msvc")"

Adb shell am force-stop $Package | Out-Null
$results | Set-Content -Encoding utf8 $Out
Write-Host "Saved to $Out"
