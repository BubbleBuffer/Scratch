"""Build the README lockup (glyph + "Scratch" wordmark as outlines) in light and dark."""
import re, sys
from pathlib import Path
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen

repo = Path(sys.argv[1])
out = repo / "design" / "readme"
out.mkdir(parents=True, exist_ok=True)

font = TTFont(repo / "app/assets/fonts/BricolageGrotesque[opsz,wdth,wght].ttf")
font = instantiateVariableFont(font, {"wght": 700, "opsz": 96, "wdth": 88})
gs = font.getGlyphSet()
cmap = font.getBestCmap()
upm = font["head"].unitsPerEm
asc = font["hhea"].ascent

# Pair kerning from the kern feature, via GPOS PairPos format 1/2 lookups.
def kerning():
    pairs = {}
    if "GPOS" not in font:
        return pairs
    gpos = font["GPOS"].table
    idx = [i for fr in gpos.FeatureList.FeatureRecord if fr.FeatureTag == "kern" for i in fr.Feature.LookupListIndex]
    for li in idx:
        lk = gpos.LookupList.Lookup[li]
        for st in lk.SubTable:
            if lk.LookupType == 9:
                st = st.ExtSubTable
            if getattr(st, "LookupType", 2) != 2 and lk.LookupType not in (2, 9):
                continue
            cov = st.Coverage.glyphs
            if st.Format == 1:
                for i, first in enumerate(cov):
                    for pvr in st.PairSet[i].PairValueRecord:
                        v = pvr.Value1.XAdvance if pvr.Value1 and hasattr(pvr.Value1, "XAdvance") else 0
                        pairs.setdefault((first, pvr.SecondGlyph), v or 0)
            elif st.Format == 2:
                c1 = st.ClassDef1.classDefs
                c2 = st.ClassDef2.classDefs
                for first in cov:
                    r = st.Class1Record[c1.get(first, 0)]
                    for second in set(cmap.values()):
                        v = r.Class2Record[c2.get(second, 0)].Value1
                        x = getattr(v, "XAdvance", 0) if v else 0
                        if x:
                            pairs.setdefault((first, second), x)
    return pairs

kern = kerning()
text = "Scratch"
names = [cmap[ord(c)] for c in text]
tracking = -0.01 * upm
x = 0.0
paths = []
for i, n in enumerate(names):
    pen = SVGPathPen(gs)
    gs[n].draw(TransformPen(pen, (1, 0, 0, -1, x, asc)))
    paths.append(pen.getCommands())
    x += gs[n].width + tracking
    if i + 1 < len(names):
        x += kern.get((n, names[i + 1]), 0)
text_w = x - tracking
cap = font["OS/2"].sCapHeight

# Lockup geometry, in font units. The glyph is as tall as the cap height times 1.5.
glyph_svg = (repo / "design/icon/glyph/scratch-glyph.svg").read_text(encoding="utf-8")
vb = [float(v) for v in re.search(r'viewBox="([^"]+)"', glyph_svg).group(1).split()]
inner = re.search(r"<svg[^>]*>(.*)</svg>", glyph_svg, re.S).group(1)
glyph_h = cap * 1.55
glyph_w = glyph_h * vb[2] / vb[3]
gap = upm * 0.22
baseline = asc
glyph_y = baseline - cap / 2 - glyph_h / 2
pad = upm * 0.06
width = glyph_w + gap + text_w + 2 * pad
top = min(glyph_y, baseline - asc * 0.98) - pad
height = max(glyph_y + glyph_h, baseline + upm * 0.06) + pad - top

for theme, color in (("light", "#2C2C2B"), ("dark", "#F0EFED")):
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 {top:.1f} {width:.1f} {height:.1f}" width="{width / upm * 96:.0f}" height="{height / upm * 96:.0f}" role="img" aria-label="Scratch">
  <title>Scratch</title>
  <svg x="{pad:.1f}" y="{glyph_y:.1f}" width="{glyph_w:.1f}" height="{glyph_h:.1f}" viewBox="{" ".join(f"{v:g}" for v in vb)}">{inner}</svg>
  <g transform="translate({pad + glyph_w + gap:.1f} 0)" fill="{color}">
    {"".join(f'<path d="{p}"/>' for p in paths)}
  </g>
</svg>
'''
    (out / f"scratch-{theme}.svg").write_text(svg, encoding="utf-8")
    print(theme, round(width), round(height), len(svg))
