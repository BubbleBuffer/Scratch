# Bake-off measurements (2026-09-30)

Emulator: `notes-flutter` AVD, API 35, x86_64, headless (`-no-window -no-audio -no-boot-anim`), GPU mode auto, renderer Google SwiftShader (OpenGL ES 3.0). Screen 1080x2400. Release APKs, one app at a time, driven with adb only. Emulator numbers are for comparing the two apps, not phone numbers.

| Metric | Tauri (SolidJS) | Flutter |
| --- | --- | --- |
| First-launch seed (10,002 pages) | 19.0 s | not captured (log cleared before reading); list visible < 600 s timeout, app was usable in well under a minute |
| Cold start, `am start -W` TotalTime, 5 runs | 619, 524, 468, 698, 671 ms; median **619 ms** | 1103, 1071, 1108, 1048, 1179 ms; median **1103 ms** (an earlier run that also included a just-installed first start gave 2513, 2171, 1638, 1344, 1207; median 1638) |
| Cold start, app's own list-visible | 374.6 ms median (webview time origin; TotalTime ends before the list is drawn) | 745-784 ms since main() |
| List scrolling, 20 flings (frame ms) | n=390, p50 33.4, p95 83.3, p99 100.0, max 166.7, 69.2% over 16.7 ms | n=987, p50 19.8, p95 27.4, p99 30.4, max 39.2, 88.7% over 16.7 ms |
| Big page scrolling, 20 flings | n=117, p50 66.6, p95 166.7, p99 200.0, max 250, 89.7% over 16.7 ms | n=982, p50 19.0, p95 28.0, p99 31.8, max 44.3, 72.2% over 16.7 ms |
| Big page open | 279.5 ms to first blocks (core load 180.4); fully rendered 2353 ms | 103.0 ms (core load 25.0); earlier run 166.8 ms |
| Typing latency, 200 chars | n=200, p50 76.5 ms, p95 141.6 ms, max 163.3 | NOT COLLECTED (see below) |
| Memory, `dumpsys meminfo` total PSS, big page open | 111,269 KB (app process only; the WebView renderer process is separate) | 99,143 KB |
| APK size (release, x86_64) | 14,178,548 B (13.5 MiB) | 23,967,338 B (22.9 MiB) |

The two apps count frames differently. Tauri uses `requestAnimationFrame` deltas (only frames that occur; the low n means the WebView produced far fewer frames, each long). Flutter counts build+raster durations of every frame in the scroll windows including idle-ish frames, so n is not comparable. Percentiles and "over 16.7" are not like-for-like across toolkits.

## Build folders on disk

| Folder | Size |
| --- | --- |
| prototypes/tauri/node_modules | 94 MB |
| prototypes/tauri/src-tauri/gen/android/app/build | 86 MB |
| prototypes/tauri/src-tauri/gen/android/build | 1 MB |
| prototypes/tauri/src-tauri/gen/android/.gradle | 3 MB |
| prototypes/tauri/dist | < 1 MB |
| prototypes/flutter/build | 472 MB |
| prototypes/flutter/.dart_tool | 50 MB |
| target/x86_64-linux-android (debug 102 MB, release 1,377 MB) | 1,479 MB (shared; Flutter's bridge is the main user, Tauri's Android build goes here too) |
| target/aarch64-linux-android | 80 MB |
| target/x86_64-pc-windows-msvc (release) | 336 MB |
| target/debug / target/release | 1,214 MB / 2,970 MB |
| target/tauri-bridge-data, tauri-bridge-shots | 60 MB, 54 MB (Tauri test vaults) |
| target total | about 6,192 MB |

The split of `target/` per prototype is not cleanly attributable (shared workspace target dir), so no per-prototype share is claimed.

## Raw BAKEOFF lines (trimmed)

Tauri (logcat tag BAKEOFF):
```
BAKEOFF seed ms=19011 pages=10002
BAKEOFF list_visible ms=19512.8 count=10002   (first launch; later runs ~375)
BAKEOFF frames surface=list bursts=1 n=390 p50=33.4 p95=83.3 p99=100.0 max=166.7 over16.7=69.2%
BAKEOFF page_open ms=279.5 load_ms=180.4 blocks=2000 title="Big page"
BAKEOFF page_rendered ms=2353.0 blocks=2000
BAKEOFF frames surface=page bursts=2 n=117 p50=66.6 p95=166.7 p99=200.0 max=250.0 over16.7=89.7%
BAKEOFF input n=200 p50=76.5 p95=141.6 max=163.3
```

Flutter (logcat tag flutter):
```
BAKEOFF list-visible ms-since-main=784 pages=10002
BAKEOFF frames-total screen=list n=987 p50=19.80 p95=27.42 p99=30.40 max=39.16 over16.7=88.7%
BAKEOFF page-open ms=103.0 title="Big page" blocks=2000 core-load-ms=25.0
BAKEOFF frames-total screen=page n=982 p50=19.00 p95=28.02 p99=31.80 max=44.28 over16.7=72.2%
```

## Problems

- Flutter typing: focus and typed text worked (verified by screenshot), but the app logged no `BAKEOFF input` lines, so latency is not collected. Not estimated. Cause not found; fixing the app was out of scope.
- measure.ps1 (Flutter) tapped the gap between blocks on the Big page, so no block was focused. Fixed: tap at `text-y + 110`, and typing now runs before the big-page scroll.
- measure.ps1 (Tauri) failed once with "no node with text 'Big page'" (WebView not ready yet). Fixed with a retry loop in `Tap-Text`.
- Tauri PSS covers only the app process. A separate WebView renderer process used about 53-60 MB PSS (measured on another page, not the Big page).

---

# Pixel 8 Pro (real phone, arm64 release builds, 2026-09-30)

Device: Pixel 8 Pro over wireless adb, Android API 36, 1008x2244 at 360 dpi (Flutter dpr 2.25), 120 Hz display. Apps were already installed; not reinstalled, data not cleared, left installed. Driven with the measure scripts plus read-only adb queries. The soft keyboard was closed before every scroll step (Flutter script now sends one BACK after typing if `mInputShown=true`; Tauri scrolls run before typing) and is closed at the end. Emulator numbers above and these are not comparable.

| Metric | Tauri (SolidJS) | Flutter |
| --- | --- | --- |
| First-launch seed (10,002 pages) | 3,355 ms (`seed ms=3355`) | 4,535 ms (`seeded pages=10001 ms=4535`) |
| Cold start, `am start -W` TotalTime, 5 runs | 215, 208, 116, 107, 123 ms; median **123 ms** | 191, 192, 183, 198, 206 ms; median **192 ms** (first run before: 205) |
| Cold start, app's own list-visible | 163.7 ms median | 94 ms since main() (82-220 across runs) |
| List scrolling, 20 flings (frame ms) | n=1441, p50 8.4, p95 16.7, p99 25.0, max 25.1, 12.8% over 16.7 ms | n=2409, p50 3.90, p95 5.46, p99 6.85, max 19.81, 0.0% over 16.7 ms |
| Big page scrolling, 20 flings | n=816, p50 8.4, p95 25.0, p99 33.2, max 58.4, 13.6% over 16.7 ms | n=2419, p50 4.24, p95 6.21, p99 7.88, max 19.19, 0.1% over 16.7 ms |
| Big page open | 160.9 ms (core load 70.8); fully rendered 394.6 ms. Second open: 112.3 ms | 96.8 ms (core load 59.0); 74.8 ms in the first run |
| Typing latency, 200 chars | n=200, p50 41.2 ms, p95 82.3 ms, max 94.8 | n=175 of 200 sent, p50 8.61 ms, p95 15.05 ms, p99 17.60, max 18.46; key-clock=none, so measured from Dart dispatch (see below) |
| Memory, total PSS, app process, Big page open | 183,332 KB (manual run); 197,699 KB in the script run (taken after typing) | 178,782 KB (before typing; 194,801 KB in the earlier run that typed first) |
| Memory, WebView renderer process, Big page open | 96,485 KB (sandboxed_process0 of the app, confirmed via its uid) -> about 279.8 MB combined | n/a (no separate renderer) |
| APK size (installed base.apk, arm64) | 13,097,207 B (12.5 MiB) | 28,866,238 B (27.5 MiB) |

Frame stat caveats: Tauri counts `requestAnimationFrame` deltas, Flutter counts build+raster time per frame, so n and percentiles are not like for like. Flutter's page-scroll total (n=2419) also includes about 148 frames logged during the typing step (the app counts them as scroll segments); the remaining 2271 frames are the flings (p50 4.26, p95 6.25, p99 8.10, max 19.19, 0.1% over).

## Raw BAKEOFF lines (trimmed)

Tauri:
```
BAKEOFF seed ms=3355 pages=10002
BAKEOFF list_visible ms=4205.1 count=10002       (first launch)
BAKEOFF frames surface=list bursts=1 n=1441 p50=8.4 p95=16.7 p99=25.0 max=25.1 over16.7=12.8%
BAKEOFF page_open ms=160.9 load_ms=70.8 blocks=2000 title="Big page"
BAKEOFF page_rendered ms=394.6 blocks=2000
BAKEOFF frames surface=page bursts=1 n=816 p50=8.4 p95=25.0 p99=33.2 max=58.4 over16.7=13.6%
BAKEOFF input n=200 p50=41.2 p95=82.3 max=94.8
```

Flutter (complete logcat of the final run, from the relaunch before opening the Big page through the end of the scroll step):
```
BAKEOFF start platform=android pid=19791
BAKEOFF clock rasterFinish-lag-us=757 (small positive means Timeline.now and FrameTiming share a clock)
BAKEOFF list-visible ms-since-main=94 pages=10002
BAKEOFF layout dpr=2.25 first-row-x=504 first-row-y=788 row-h=99 big-index=1 list-top=113 list-bottom=2053 screen-w=1008 screen-h=2244
BAKEOFF layout page text-x=378 text-y=1102 list-top=212 list-bottom=2190
BAKEOFF page-open ms=96.8 title="Big page" blocks=2000 core-load-ms=59.0
BAKEOFF focus kind=paragraph index=4
BAKEOFF frames screen=page n=9 p50=3.33 p95=5.94 p99=5.94 max=5.94 over16.7=0.0%      (during typing)
BAKEOFF frames screen=page n=139 p50=2.81 p95=5.44 p99=6.38 max=7.73 over16.7=0.0%    (during typing)
BAKEOFF frames-total screen=page n=148 ...
BAKEOFF frames screen=page n=2271 p50=4.26 p95=6.25 p99=8.10 max=19.19 over16.7=0.1%  (20 flings)
BAKEOFF frames-total screen=page n=2419 p50=4.24 p95=6.21 p99=7.88 max=19.19 over16.7=0.1%
```
First-launch lines: `seeded pages=10001 ms=4535`, `list-visible ms-since-main=4841 pages=10002`. List run: `frames-total screen=list n=2409 p50=3.90 p95=5.46 p99=6.85 max=19.81 over16.7=0.0%`.

## Flutter typing diagnosis data

- The clock line is healthy (lag 304-1624 us on phone and emulator, so Timeline.now and FrameTiming share a clock).
- The `focus` line appears, and typed text does appear on screen (screenshot on the phone shows `abcdefghij` inserted into a bullet item, IME composing strip shows the word). No `BAKEOFF input` and no `input-total` line is ever logged, on the emulator or the phone, after 10, 100 or 200 characters, and no error lines in the flutter tag.
- During typing the log does show two `frames screen=page` segments (n=9, then n=139), so scroll-tracking fires for the typing; the input path (`Bakeoff.textChanged` -> `_pendingInputs` -> `_onTimings` -> `_dumpInput` after 2.5 s) never produces output. Either `onChanged` is not reached by IME input, or `_pendingInputs` is never drained (`buildStart >= pending.$1` never true).

## Problems (phone run)

- The Flutter script originally typed before the big-page fling step, leaving the keyboard open for the flings. Redone after adding the keyboard close (BACK only when `mInputShown=true`; verified it stayed on the page, `topResumedActivity` still MainActivity). The numbers above are from the redo.
- My manual Flutter test tap landed on the showcase page, not the Big page, because the list order changes after edits (`big-index` moved between 0 and 1); the script re-reads it each run, so the measured runs opened the Big page. Typing tests edited an existing page's text (abcdefghij in the showcase, and 200 letters in a Big page paragraph), so the user's phone vault now contains those edits.
- Tauri's script seeds memory reading after typing; the manual reading with the Big page open before typing is listed first.

## Flutter typing, rerun after the bakeoff.dart fix (commit 2439845)

Rebuilt arm64 release, `adb install -r` on the Pixel (data kept), typed 200 characters (10 x `input text` of 20) into a Big page paragraph, keyboard closed afterwards with one BACK (stayed on MainActivity).
```
BAKEOFF page-open ms=77.3 title="Big page" blocks=2000 core-load-ms=36.6
BAKEOFF focus kind=paragraph index=4
BAKEOFF input n=175 p50=8.61 p95=15.05 p99=17.60 max=18.46 key-clock=none
BAKEOFF input-total n=175 p50=8.61 p95=15.05 p99=17.60 max=18.46 from-dart-dispatch: n=175 p50=8.61 p95=15.05 p99=17.60 max=18.46
```
- key-clock=none: `input text` produces no hardware key events, so latency is from the Dart text-change dispatch to the raster finish of the frame, not from the key event. The Tauri figure (41.2 ms p50) is measured differently (key event to next painted frame), so the two are not strictly like for like.
- n=175, not 200: 25 of the 200 changes were not counted (likely several changes landing in one frame, or the 50-entry pending cap). Not investigated.
