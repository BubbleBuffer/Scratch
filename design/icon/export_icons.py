#!/usr/bin/env python3
"""Export the app icon from design/icon/notes-icon.svg.

Writes (all paths relative to design/icon):

  glyph/scratch-glyph.svg, glyph/png/scratch-glyph-{16..1024}.png, glyph/png/scratch-glyph-256-small.png
      The "Notebook cat" group alone, background removed, viewBox cropped to the glyph plus padding.
  notes-icon-icons/png/icon-{16,32,48,64,128,256,512,1024}.png
      Full icon (desk plus glyph), square, unmasked.
  notes-icon-icons/windows/app_icon.ico
      Frames 16, 24, 32, 48, 64, 256; PNG-encoded entries.
  notes-icon-icons/android/mipmap-<density>/ic_launcher.png and ic_launcher_round.png
      Full icon at 48, 72, 96, 144, 192 px; the round variant is masked to a circle.
  notes-icon-icons/android/adaptive/ic_launcher_foreground.png
      Glyph alone at 432 px, scaled about the canvas centre so the whole glyph sits inside the
      Android safe zone (66/108 of the diameter). Transparent background.
  notes-icon-icons/android/adaptive/ic_launcher_monochrome.png
      The same placement as the foreground, flattened to white with the glyph's alpha.
  notes-icon-icons/android/adaptive/ic_launcher_background.png
      Not written: the background is kept as it is.
  notes-icon-icons/review.png, notes-icon-icons/manifest.json

With --install, the exports are copied to the app paths listed in INSTALL below.

Rendering is headless Chrome (pawthctl needs the Pawth app open). Needs Python 3 with Pillow.

Usage:
  python design/icon/export_icons.py [--chrome "C:/Program Files/Google/Chrome/Application/chrome.exe"] [--install]
"""
import argparse
import json
import re
import shutil
import struct
import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
SRC = HERE / "notes-icon.svg"
GLYPH = HERE / "glyph"
OUT = HERE / "notes-icon-icons"
CANVAS = 1280
SAFE_DIAM = 66 / 108            # Android adaptive safe zone, fraction of the canvas diameter
PNG_SIZES = [16, 32, 48, 64, 128, 256, 512, 1024]
ICO_SIZES = [16, 24, 32, 48, 64, 256]
DENSITY = {"mdpi": 48, "hdpi": 72, "xhdpi": 96, "xxhdpi": 144, "xxxhdpi": 192}
ADAPTIVE = 432
CHROME_DEFAULT = "C:/Program Files/Google/Chrome/Application/chrome.exe"

# Files in the app that ship an icon, and the export each one takes.
INSTALL = {
    "app/assets/icons/app_icon_64.png": "png/icon-64.png",
    "app/windows/runner/resources/app_icon.ico": "windows/app_icon.ico",
}
for _d in DENSITY:
    INSTALL[f"app/android/app/src/main/res/mipmap-{_d}/ic_launcher.png"] = f"android/mipmap-{_d}/ic_launcher.png"
    INSTALL[f"app/android/app/src/main/res/mipmap-{_d}/ic_launcher_round.png"] = f"android/mipmap-{_d}/ic_launcher_round.png"
INSTALL["app/android/app/src/main/res/mipmap-xxxhdpi/ic_launcher_foreground.png"] = "android/adaptive/ic_launcher_foreground.png"
INSTALL["app/android/app/src/main/res/mipmap-xxxhdpi/ic_launcher_monochrome.png"] = "android/adaptive/ic_launcher_monochrome.png"


def find_block_end(s, start, tag):
    depth = 0
    for m in re.finditer(r"<(/?)%s\b[^>]*?(/?)>" % tag, s[start:]):
        closing, selfclosing = m.group(1), m.group(2)
        if selfclosing:
            if depth == 0:
                return start + m.end()
            continue
        if closing:
            depth -= 1
            if depth == 0:
                return start + m.end()
        else:
            depth += 1
    raise ValueError("unbalanced <%s>" % tag)


def render(chrome, svg_text, size, out_png, tmp):
    (tmp / "icon.svg").write_text(svg_text, encoding="utf-8")
    (tmp / "page.html").write_text(
        '<!doctype html><html><head><meta charset="utf-8"><style>'
        "html,body{margin:0;padding:0;background:transparent;overflow:hidden}"
        "img{display:block;width:%dpx;height:%dpx}"
        '</style></head><body><img src="icon.svg"></body></html>' % (size, size),
        encoding="utf-8")
    subprocess.run(
        [chrome, "--headless=new", "--disable-gpu", "--hide-scrollbars",
         "--default-background-color=00000000", "--window-size=%d,%d" % (size, size),
         "--screenshot=" + str(Path(out_png).resolve()), (tmp / "page.html").as_uri()],
        check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=180)
    return Image.open(out_png).convert("RGBA")


def circle_mask(n, diam_frac):
    m = Image.new("L", (n, n), 0)
    r = diam_frac * n / 2
    c = n / 2
    ImageDraw.Draw(m).ellipse((c - r, c - r, c + r, c + r), fill=255)
    return m


def write_ico(path, frames):
    """frames: list of (size, png_bytes). Writes an ICO with PNG-encoded entries."""
    n = len(frames)
    header = struct.pack("<HHH", 0, 1, n)
    offset = 6 + 16 * n
    entries, data = b"", b""
    for size, png in frames:
        w = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", w, w, 0, 0, 1, 32, len(png), offset)
        data += png
        offset += len(png)
    path.write_bytes(header + entries + data)


def fit_radius(alpha):
    """Smallest radius (canvas px, centred) that contains every pixel of alpha > 0."""
    n = alpha.size[0]
    c = n / 2
    lo, hi = 0.0, n * 0.9
    for _ in range(24):
        mid = (lo + hi) / 2
        m = Image.new("L", (n, n), 0)
        ImageDraw.Draw(m).ellipse((c - mid, c - mid, c + mid, c + mid), fill=255)
        if ImageChops.subtract(alpha, m).getbbox() is None:
            hi = mid
        else:
            lo = mid
    return hi


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--chrome", default=CHROME_DEFAULT)
    ap.add_argument("--install", action="store_true", help="copy the exports into the app paths")
    args = ap.parse_args()
    chrome = args.chrome
    s = SRC.read_text(encoding="utf-8")
    cat_start = re.search(r'<g id="[^"]+" data-name="Notebook cat"', s).start()
    cat_end = find_block_end(s, cat_start, "g")
    cat = s[cat_start:cat_end]
    assert s.count(cat) == 1
    defs_start = s.index("<defs")
    defs = s[defs_start:find_block_end(s, defs_start, "defs")]
    tmp = Path(tempfile.mkdtemp(prefix="icon-export-"))

    # 1. Glyph alone: bbox at canvas scale, viewBox with 4% padding.
    gonly = ('<?xml version="1.0" encoding="UTF-8"?>\n<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %d %d">\n'
             "  %s\n  %s\n</svg>\n" % (CANVAS, CANVAS, defs, cat))
    g1280 = render(chrome, gonly, CANVAS, str(tmp / "glyph1280.png"), tmp)
    l, t, r, b = g1280.getchannel("A").point(lambda v: 255 if v > 127 else 0).getbbox()
    pad = round(max(r - l, b - t) * 0.04)
    vb = (l - pad, t - pad, (r - l) + 2 * pad, (b - t) + 2 * pad)
    glyph_svg = ('<?xml version="1.0" encoding="UTF-8"?>\n<svg xmlns="http://www.w3.org/2000/svg" viewBox="%d %d %d %d">\n'
                 "  %s\n  %s\n</svg>\n" % (vb[0], vb[1], vb[2], vb[3], defs, cat))
    (GLYPH / "scratch-glyph.svg").write_text(glyph_svg, encoding="utf-8", newline="\n")
    (GLYPH / "png").mkdir(exist_ok=True)
    for n in [16, 32, 64, 128, 256, 512, 1024]:
        render(chrome, glyph_svg, n, str(GLYPH / "png" / ("scratch-glyph-%d.png" % n)), tmp)
    im256 = Image.open(GLYPH / "png" / "scratch-glyph-256.png").convert("RGBA")
    small = im256.quantize(colors=256, method=Image.Quantize.FASTOCTREE, dither=Image.Dither.NONE)
    small.save(GLYPH / "png" / "scratch-glyph-256-small.png", optimize=True)
    small_bytes = (GLYPH / "png" / "scratch-glyph-256-small.png").stat().st_size
    assert small_bytes < 10000, small_bytes

    # 2. Full icon PNGs.
    OUT.mkdir(exist_ok=True)
    (OUT / "png").mkdir(exist_ok=True)
    for n in PNG_SIZES:
        render(chrome, s, n, str(tmp / ("p_%d.png" % n)), tmp).save(OUT / "png" / ("icon-%d.png" % n))

    # 3. Windows ICO.
    (OUT / "windows").mkdir(exist_ok=True)
    frames = []
    for n in ICO_SIZES:
        buf = tmp / ("ico_%d.png" % n)
        render(chrome, s, n, str(tmp / ("ico_raw_%d.png" % n)), tmp).save(buf)
        frames.append((n, buf.read_bytes()))
    write_ico(OUT / "windows" / "app_icon.ico", frames)

    # 4. Android mipmaps: square and round.
    for dens, px in DENSITY.items():
        d = OUT / "android" / ("mipmap-%s" % dens)
        d.mkdir(parents=True, exist_ok=True)
        full = render(chrome, s, px, str(tmp / ("m_%d.png" % px)), tmp)
        full.save(d / "ic_launcher.png")
        rnd = full.copy()
        rnd.putalpha(ImageChops.multiply(rnd.getchannel("A"), circle_mask(px, 1.0)))
        rnd.save(d / "ic_launcher_round.png")

    # 5. Adaptive foreground and monochrome: glyph within the safe zone.
    adaptive = OUT / "android" / "adaptive"
    adaptive.mkdir(parents=True, exist_ok=True)
    r_fit = fit_radius(g1280.getchannel("A"))              # glyph radius from the canvas centre
    safe_r = SAFE_DIAM * CANVAS / 2
    f = 0.98 * safe_r / r_fit                              # 2% margin inside the safe circle
    e = CANVAS / 2 * (1 - f)
    fg_svg = ('<?xml version="1.0" encoding="UTF-8"?>\n<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %d %d">\n'
              '  %s\n  <g transform="matrix(%.6f 0 0 %.6f %.3f %.3f)">\n  %s\n  </g>\n</svg>\n'
              % (CANVAS, CANVAS, defs, f, f, e, e, cat))
    fg = render(chrome, fg_svg, ADAPTIVE, str(tmp / "fg.png"), tmp)
    fg.save(adaptive / "ic_launcher_foreground.png")
    mono = Image.new("RGBA", fg.size, (255, 255, 255, 0))
    mono.putalpha(fg.getchannel("A"))
    mono.save(adaptive / "ic_launcher_monochrome.png")
    # The background layer is kept as it is (not rewritten here).

    # 6. Manifest and review sheet.
    mpath = OUT / "manifest.json"
    man = json.loads(mpath.read_text(encoding="utf-8"))
    man["sourceRevision"] = int(man.get("sourceRevision", 1)) + 1
    man["glyph"] = {"heightPercent": 80, "adaptiveScale": round(f, 6),
                    "adaptiveSafeZoneDiameterPercent": round(SAFE_DIAM * 100, 2)}
    mpath.write_text(json.dumps(man, indent=2, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n")
    make_review(OUT / "review.png", OUT)

    shutil.rmtree(tmp, ignore_errors=True)
    print("glyph bbox (1280):", (l, t, r, b), "viewBox", vb, "small PNG bytes", small_bytes)
    print("glyph fit radius %.1f, safe radius %.1f, adaptive scale %.4f, foreground glyph height %.1f%% of canvas"
          % (r_fit, safe_r, f, (b - t) * f / CANVAS * 100))

    if args.install:
        for app_rel, exp_rel in INSTALL.items():
            dst = REPO / app_rel
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(OUT / exp_rel, dst)
            print("installed", app_rel)


def make_review(path, out):
    def font(sz, bold=False):
        try:
            return ImageFont.truetype("C:/Windows/Fonts/arial%s.ttf" % ("bd" if bold else ""), sz)
        except OSError:
            return ImageFont.load_default(sz)
    items = [("Full 256", out / "png" / "icon-256.png", (255, 250, 241)),
             ("Round 192", out / "android" / "mipmap-xxxhdpi" / "ic_launcher_round.png", (255, 250, 241)),
             ("Foreground", out / "android" / "adaptive" / "ic_launcher_foreground.png", (242, 235, 221)),
             ("Monochrome", out / "android" / "adaptive" / "ic_launcher_monochrome.png", (40, 42, 47))]
    W, H = 1040, 360
    img = Image.new("RGB", (W, H), (255, 255, 255))
    d = ImageDraw.Draw(img)
    d.text((24, 16), "Scratch icon export (80% glyph height)", font=font(24, True), fill=(40, 42, 47))
    for i, (label, p, bg) in enumerate(items):
        x = 24 + i * 256
        d.rectangle((x, 60, x + 232, 60 + 232), fill=bg)
        im = Image.open(p).convert("RGBA")
        im = im.resize((220, 220), Image.LANCZOS)
        img.paste(im, (x + 6, 66), im)
        d.text((x, 306), label, font=font(16), fill=(40, 42, 47))
    img.save(path)


if __name__ == "__main__":
    main()
