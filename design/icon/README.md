# App icon

The app icon source is `notes-icon.svg` (1280 x 1280): a `background` layer (the cream "Desk" rect `#F3EFE7`) and a "Notebook cat" group (the glyph, `matrix(0.94 0 0 0.94 23 -24)`). `rebuild.sh` rebuilds it through Pawth (`pawthctl`). The exported platform icons live in `notes-icon-icons/` and are copied into `app/`. Nothing in `app/` has changed for the options below: the user picks an option first.

## Folders

| Path | What it is |
|---|---|
| `notes-icon.svg` | Current full icon (desk + glyph at 0.94). Unchanged. |
| `rebuild.sh` | Rebuilds `notes-icon.svg` from `kits/bubblebuffer.kit.svg` through pawthctl. |
| `kits/` | BubbleBuffer kit SVG used by `rebuild.sh`. |
| `notes-icon-icons/` | Exported Android, Windows and PNG icon pack. |
| `options/` | Earlier notebook-shape studies (a to e). |
| `glyph/` | The glyph alone, transparent. |
| `glyph/scratch-glyph.svg` | "Notebook cat" group with the background removed. viewBox cropped to the glyph plus padding. |
| `glyph/png/` | Transparent renders at 16, 32, 64, 128, 256, 512, 1024 px, and `scratch-glyph-256-small.png` (4,194 bytes). |
| `scale-options/` | Full icon with the glyph scaled by height, one set per option. |
| `scale-options/make_scale_options.py` | Generator for everything in `scale-options/` (see below). |
| `scale-options/scale-094.svg` | Baseline: a copy of `notes-icon.svg`. |
| `scale-options/scale-070.svg`, `-076`, `-082` | Glyph bbox height at 70%, 76% and 82% of the canvas, centred. |
| `scale-options/scale-<id>-1024.png`, `-64.png`, `-16.png` | Full-icon renders per option. |
| `scale-options/scale-<id>-android-round-1024.png` | 1024 render masked to a circle, with a ring at the Android safe zone (66/108 of the diameter). |
| `scale-options/sheet.png` | Contact sheet of the four options on light and dark backgrounds, labelled with height and safe-zone clip. |

## Options

Scaling is by glyph bbox height, centred on the canvas. The baseline is the current file. The glyph is taller than wide, so height is the limiting dimension.

| Option | Glyph height | Glyph width | Outside the safe zone |
|---|---|---|---|
| 0.94 (current) | 62.8% | 49.0% | 4% of glyph pixels |
| 70% | 70.0% | 54.6% | 12% |
| 76% | 75.9% | 59.3% | 20% |
| 82% | 81.9% | 64.0% | 30% |

"Outside the safe zone" means glyph pixels (alpha > 127) that lie outside the 66/108 safe circle in the round preview. All four options clip it. None reaches the full 1024 mask circle, so nothing is cut by the round mask itself.

Not rendered: width-based options at 72%, 80% and 88%. Those were the first proposal, and they are superseded by the height rule above.

## How it was produced

Rendering uses headless Chrome, since `pawthctl screenshot` needs the Pawth app open. Each render is an HTML page with one `<img>` of the SVG at size N x N and a transparent background:

    chrome.exe --headless=new --disable-gpu --hide-scrollbars \
      --default-background-color=00000000 --window-size=N,N \
      --screenshot=out.png file:///.../page.html

**Glyph files** (`glyph/`). Copy `notes-icon.svg`, remove the `<g id="background">` element, and keep the `<defs>` block (the `ear-highlight` gradient is used by the ear patch) and the "Notebook cat" group, byte for byte. The viewBox is `306 200 693 868`: the visible glyph bbox (x 338 to 967, y 232 to 1036 on the 1280 canvas) plus 32 px, about 4% of the 804 px height. Render at each size with the Chrome command above; the square canvas is letterboxed with `xMidYMid meet`, so the glyph is centred. The small PNG is the 256 px render quantized to 256 colours with Pillow (`Image.quantize`, FASTOCTREE, no dither), `optimize=True`, alpha kept. The glyph helper code was run from a scratch folder and is not committed.

**Scale options** (`scale-options/`). Run `make_scale_options.py` from this folder with Python 3 and Pillow, and Chrome installed:

    python make_scale_options.py [--chrome "C:/Program Files/Google/Chrome/Application/chrome.exe"]

The script measures the baseline glyph bbox from a glyph-only render, then for each height h sets scale k = h x 1280 / bbox height, and wraps the unchanged "Notebook cat" group in `matrix(k 0 0 k tx ty)`, with the bbox centre mapped to (640, 640). Paths are not edited. It writes the SVGs, the 1024, 64 and 16 px renders, the round previews and `sheet.png`, and prints the measured fills and safe-zone numbers in the table above.

The round preview is the 1024 render with alpha set to zero outside a circle of full canvas diameter, plus a ring of diameter 66/108 of the canvas. The Android adaptive spec is 66 dp of a 108 dp canvas, so the ring is 61.1% of the diameter.
