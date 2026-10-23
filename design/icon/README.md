# App icon

The app icon source is `notes-icon.svg` (1280 x 1280): a `background` layer (the cream "Desk" rect `#F2EBDD`) and a "Notebook cat" group, the glyph. The glyph is scaled to **80% of the canvas height** (the user's choice), centred on (640, 640): `matrix(1.197214 0 0 1.197214 -161.751 -198.05)`. Paths are unchanged. `rebuild.sh` rebuilds the source through Pawth (`pawthctl`) with the same transform. `export_icons.py` exports every platform icon from the source.

## Folders

| Path | What it is |
|---|---|
| `notes-icon.svg` | Current full icon (desk + glyph at 80% height). |
| `rebuild.sh` | Rebuilds `notes-icon.svg` from `kits/bubblebuffer.kit.svg` through pawthctl, same transform. |
| `export_icons.py` | Exports everything below from `notes-icon.svg`; `--install` copies the shipped files into `app/`. |
| `kits/` | BubbleBuffer kit SVG used by `rebuild.sh`. |
| `notes-icon-icons/` | Exported icon pack (PNG, Windows ICO, Android mipmaps and adaptive layers, `manifest.json`, `review.png`). |
| `glyph/scratch-glyph.svg` | "Notebook cat" group alone, background removed. viewBox `199 87 881 1106`: the glyph bbox (x 240 to 1039, y 128 to 1152 on the 1280 canvas) plus 4% padding. |
| `glyph/png/` | Transparent renders at 16, 32, 64, 128, 256, 512, 1024 px, and `scratch-glyph-256-small.png` (4,233 bytes, 256 colours). |
| `options/` | Earlier notebook-shape studies (a to e). |
| `scale-options/` | The glyph-height study that chose 80% (generated from commit a9798695; see below). |

## Shipped export

`export_icons.py` writes `notes-icon-icons/`:

- `png/icon-{16,32,48,64,128,256,512,1024}.png`: full icon, RGBA, opaque.
- `windows/app_icon.ico`: frames 16, 24, 32, 48, 64, 256, PNG-encoded, 32 bpp.
- `android/mipmap-{mdpi,hdpi,xhdpi,xxhdpi,xxxhdpi}/ic_launcher.png` (48, 72, 96, 144, 192 px) and `ic_launcher_round.png` (same sizes, circle mask).
- `android/adaptive/ic_launcher_foreground.png` and `ic_launcher_monochrome.png` (432 px). The glyph is scaled by 0.6597 about the centre so the whole glyph sits inside the 66/108 safe circle (radius 391.1 px at 1280 scale; the glyph's farthest point is 129.6 of 132.0 px at 432). The monochrome layer is white with the foreground's alpha.
- `android/adaptive/ic_launcher_background.png`: not regenerated; kept as it is.

Rendering uses headless Chrome (`pawthctl screenshot` needs the Pawth app open). Each render is an HTML page with one `<img>` at N x N and a transparent background. Run from this folder:

    python export_icons.py [--chrome "C:/Program Files/Google/Chrome/Application/chrome.exe"] [--install]

## Files replaced in app/

| File | Export |
|---|---|
| `app/assets/icons/app_icon_64.png` | `png/icon-64.png` |
| `app/windows/runner/resources/app_icon.ico` | `windows/app_icon.ico` |
| `app/android/app/src/main/res/mipmap-{mdpi,hdpi,xhdpi,xxhdpi,xxxhdpi}/ic_launcher.png` (5 files) | `android/mipmap-<density>/ic_launcher.png` |
| `app/android/app/src/main/res/mipmap-{mdpi,hdpi,xhdpi,xxhdpi,xxxhdpi}/ic_launcher_round.png` (5 files) | `android/mipmap-<density>/ic_launcher_round.png` |
| `app/android/app/src/main/res/mipmap-xxxhdpi/ic_launcher_foreground.png` | `android/adaptive/ic_launcher_foreground.png` |
| `app/android/app/src/main/res/mipmap-xxxhdpi/ic_launcher_monochrome.png` | `android/adaptive/ic_launcher_monochrome.png` |

Not replaced: `ic_launcher_background.png`. No Linux, tray or hub favicon file is tracked, so there is nothing to replace there. The installer (`scratch.iss`) and `Runner.rc` both use the replaced `app_icon.ico`.

## Notes

- The desk colour is `#F2EBDD` (242, 235, 221), the same as the previously shipped icons; the desk token in the kit, the source and `rebuild.sh` all use it.
- The adaptive foreground is smaller than the full icon by design: the 80% glyph is 1024 px tall, which does not fit the 66/108 safe circle at full size (61% of the diameter), so the foreground is scaled to 52.8% of canvas height.
- `crates/core/tests/feedback_markdown.rs` embeds `app_icon_64.png` and checks only a byte round trip, so the new bytes should not affect it. Not run on this PC (no builds here); CI covers it.

## Scale study (how 80% was chosen)

Scaling was measured by glyph bbox height, centred on the canvas, with the unchanged glyph paths. Produced by `scale-options/make_scale_options.py` (committed at a9798695). The script's docstring still describes its baseline as the source; that is stale. Its baseline is now 80%, and the study's 0.94 baseline is the old source.

| Option | Glyph height | Glyph width | Glyph pixels outside the safe circle |
|---|---|---|---|
| 0.94 (old source) | 62.8% | 49.0% | 4% |
| 70% | 70.0% | 54.6% | 12% |
| 76% | 75.9% | 59.3% | 20% |
| 80% (shipped) | 80.0% | 62.5% | 26.7% |
| 82% | 81.9% | 64.0% | 30% |

"Outside the safe circle" means glyph pixels (alpha > 127) outside the 66/108 safe circle in the 1280 canvas. The 80% row was measured on the glyph-only render, not the full icon, so it may differ slightly from the script's output. Width-based options (72%, 80%, 88%) were the first proposal and are superseded by the height rule.
