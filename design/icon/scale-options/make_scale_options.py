#!/usr/bin/env python3
"""Generate the glyph scale options for the app icon.

Reads ../notes-icon.svg and writes next to this file:

  scale-094.svg                    copy of the current icon (baseline, glyph height 62.8%)
  scale-070.svg, -076, -082        the same icon with the "Notebook cat" group wrapped in one
                                   extra transform so its visible bbox is 70%, 76% or 82% of the
                                   canvas HEIGHT, centred on the canvas
  scale-<id>-1024.png, -64, -16    full-icon renders (desk background plus glyph)
  scale-<id>-android-round-1024.png
                                   the 1024 render masked to a circle (launcher look), with a ring
                                   at the Android adaptive safe zone (66/108 of the diameter)
  sheet.png                        contact sheet: four options on light and dark backgrounds

Only the existing paths are used. The Notebook cat group is not edited; the wrapper transform
is the only change. Rendering is headless Chrome (pawthctl needs the Pawth app open).

Usage (Git Bash or PowerShell, Python 3 with Pillow):
  python make_scale_options.py [--chrome "C:/Program Files/Google/Chrome/Application/chrome.exe"]
"""
import argparse
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
SRC = HERE.parent / "notes-icon.svg"
CANVAS = 1280
HEIGHTS = [70, 76, 82]        # glyph bbox height, percent of canvas height (baseline is measured)
SAFE_DIAM = 66 / 108          # Android adaptive safe zone as a fraction of the canvas diameter
CHROME_DEFAULT = "C:/Program Files/Google/Chrome/Application/chrome.exe"


def find_block_end(s, start, tag):
    """Index just past the element of type `tag` that opens at `start`."""
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
    """Rasterize an SVG string to a transparent size x size PNG with headless Chrome."""
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
         "--screenshot=" + os.path.abspath(out_png), (tmp / "page.html").as_uri()],
        check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=180)
    return out_png


def circle_mask(n, diam_frac):
    m = Image.new("L", (n, n), 0)
    r = diam_frac * n / 2
    c = n / 2
    ImageDraw.Draw(m).ellipse((c - r, c - r, c + r, c + r), fill=255)
    return m


def glyph_stats(png):
    """Visible glyph bbox (alpha > 127) and how much of it lies outside the safe zone and the mask."""
    im = Image.open(png).convert("RGBA")
    n = im.size[0]
    a = im.getchannel("A").point(lambda v: 255 if v > 127 else 0)
    total = a.histogram()[255]
    outside_safe = ImageChops.subtract(a, circle_mask(n, SAFE_DIAM)).histogram()[255]
    outside_mask = ImageChops.subtract(a, circle_mask(n, 1.0)).histogram()[255]
    return a.getbbox(), total, outside_safe, outside_mask


def round_preview(png_in, png_out):
    im = Image.open(png_in).convert("RGBA")
    n = im.size[0]
    im.putalpha(ImageChops.multiply(im.getchannel("A"), circle_mask(n, 1.0)))
    d = ImageDraw.Draw(im)
    r = SAFE_DIAM * n / 2
    c = (n - 1) / 2
    d.ellipse((c - r, c - r, c + r, c + r), outline=(224, 69, 123, 255), width=max(2, n // 300))
    im.save(png_out)


def build(chrome, out_dir):
    s = SRC.read_text(encoding="utf-8")
    cat_m = re.search(r'<g id="[^"]+" data-name="Notebook cat"', s)
    cat_start = cat_m.start()
    cat_end = find_block_end(s, cat_start, "g")
    cat = s[cat_start:cat_end]
    assert s.count(cat) == 1
    defs_start = s.index("<defs")
    defs = s[defs_start:find_block_end(s, defs_start, "defs")]
    tmp = Path(tempfile.mkdtemp(prefix="scale-options-"))

    def glyph_only(wrap_open="", wrap_close=""):
        return ('<?xml version="1.0" encoding="UTF-8"?>\n'
                '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %d %d">\n  %s\n  %s%s%s\n</svg>\n'
                % (CANVAS, CANVAS, defs, wrap_open, cat, wrap_close))

    # Baseline: measure the file as it is.
    base_png = render(chrome, glyph_only(), CANVAS, str(tmp / "base.png"), tmp)
    bl, bt, br, bb = glyph_stats(base_png)[0]
    H0 = bb - bt
    cx0, cy0 = (bl + br) / 2, (bt + bb) / 2

    results = []
    options = [("094", None)] + [("%03d" % h, h) for h in HEIGHTS]
    for oid, pct in options:
        if pct is None:
            svg_text = s
            shutil.copyfile(SRC, out_dir / "scale-094.svg")
            gonly = glyph_only()
        else:
            k = (pct / 100) * CANVAS / H0
            tx, ty = CANVAS / 2 - k * cx0, CANVAS / 2 - k * cy0
            wrap_open = '<g id="glyph-scale" transform="matrix(%.6f 0 0 %.6f %.3f %.3f)">\n    ' % (k, k, tx, ty)
            svg_text = s.replace(cat, wrap_open + cat + "\n  </g>", 1)
            (out_dir / ("scale-%s.svg" % oid)).write_text(svg_text, encoding="utf-8", newline="\n")
            gonly = glyph_only(wrap_open, "</g>")
        gpng = render(chrome, gonly, CANVAS, str(tmp / ("g_%s.png" % oid)), tmp)
        bbox, total, out_safe, out_mask = glyph_stats(gpng)
        l, t, r, b = bbox
        for n in (1024, 64, 16):
            render(chrome, svg_text, n, str(out_dir / ("scale-%s-%d.png" % (oid, n))), tmp)
        round_preview(out_dir / ("scale-%s-1024.png" % oid),
                      out_dir / ("scale-%s-android-round-1024.png" % oid))
        results.append(dict(
            id=oid, pct=pct,
            height=(b - t) / CANVAS * 100, width=(r - l) / CANVAS * 100,
            centre=((l + r) / 2, (t + b) / 2),
            out_safe=out_safe / total * 100 if total else 0.0,
            out_mask=out_mask))
    shutil.rmtree(tmp, ignore_errors=True)
    return results


def sheet(out_dir, results):
    def font(sz, bold=False):
        name = "arialbd.ttf" if bold else "arial.ttf"
        try:
            return ImageFont.truetype("C:/Windows/Fonts/" + name, sz)
        except OSError:
            return ImageFont.load_default(sz)

    COLW, PAD, ICON, ROUND, BAND_H = 360, 30, 240, 210, 660
    bands = [("Light background", (255, 250, 241), (40, 42, 47)),
             ("Dark background", (40, 42, 47), (255, 250, 241))]
    W = PAD + len(results) * (COLW + PAD)
    H = 80 + len(bands) * BAND_H
    img = Image.new("RGB", (W, H), (255, 255, 255))
    ImageDraw.Draw(img).text((PAD, 22), "Scratch icon: glyph scale options (height of the glyph bbox)",
                             font=font(34, True), fill=(40, 42, 47))
    for bi, (title, bg, fg) in enumerate(bands):
        T = 80 + bi * BAND_H
        d = ImageDraw.Draw(img)
        d.rectangle((0, T, W, T + BAND_H - 1), fill=bg)
        d.text((PAD, T + 12), title, font=font(22, True), fill=fg)
        for ci, res in enumerate(results):
            x = PAD + ci * (COLW + PAD)
            if res["pct"] is None:
                head = "Baseline 0.94 (current)"
            else:
                head = "Glyph height %d%%" % res["pct"]
            clip = ("clips safe zone: yes, %.0f%% of glyph outside" % res["out_safe"]
                    if res["out_safe"] > 0 else "clips safe zone: no")
            d.text((x, T + 50), head, font=font(20, True), fill=fg)
            d.text((x, T + 78), "height %.1f%%, width %.1f%%" % (res["height"], res["width"]),
                   font=font(15), fill=fg)
            d.text((x, T + 100), clip, font=font(15, True), fill=fg)
            full = Image.open(out_dir / ("scale-%s-1024.png" % res["id"])).convert("RGBA")
            img.paste(full.resize((ICON, ICON), Image.LANCZOS), (x + (COLW - ICON) // 2, T + 125), full.resize((ICON, ICON), Image.LANCZOS))
            ry = T + 385
            rnd = Image.open(out_dir / ("scale-%s-android-round-1024.png" % res["id"])).convert("RGBA")
            rnd_small = rnd.resize((ROUND, ROUND), Image.LANCZOS)
            img.paste(rnd_small, (x, ry), rnd_small)
            d.text((x, ry + ROUND + 8), "Android round, ring = 66/108 safe zone", font=font(14), fill=fg)
            s64 = Image.open(out_dir / ("scale-%s-64.png" % res["id"])).convert("RGBA")
            s16 = Image.open(out_dir / ("scale-%s-16.png" % res["id"])).convert("RGBA")
            sx = x + ROUND + 30
            img.paste(s64, (sx, ry), s64)
            d.text((sx + 74, ry + 22), "64 px", font=font(14), fill=fg)
            img.paste(s16, (sx, ry + 100), s16)
            d.text((sx + 74, ry + 106), "16 px", font=font(14), fill=fg)
    img.save(out_dir / "sheet.png")
    return img.size


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--chrome", default=CHROME_DEFAULT)
    args = ap.parse_args()
    results = build(args.chrome, HERE)
    for r in results:
        print("%-6s height %.1f%%  width %.1f%%  centre (%.1f, %.1f)  outside safe zone %.1f%%  outside full circle %d px"
              % (r["id"], r["height"], r["width"], r["centre"][0], r["centre"][1],
                 r["out_safe"], r["out_mask"]))
    print("sheet", sheet(HERE, results))


if __name__ == "__main__":
    main()
