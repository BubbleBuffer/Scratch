"""Create before/after sheets for each changed golden, without altering it."""
import json
import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw


old_root, new_root, output = map(Path, sys.argv[1:4])
output.mkdir(parents=True, exist_ok=True)
changed = []
tiles = []
for new_path in sorted(new_root.glob('*.png')):
    old_path = old_root / new_path.name
    if not old_path.exists():
        continue
    old = Image.open(old_path).convert('RGB')
    new = Image.open(new_path).convert('RGB')
    diff = ImageChops.difference(old, new)
    box = diff.getbbox()
    if box is None:
        continue
    changed.append({'name': new_path.name, 'bbox': box})
    x0, y0, x1, y1 = box
    box = (max(0, x0 - 16), max(0, y0 - 16), min(new.width, x1 + 16), min(new.height, y1 + 16))
    before, after = old.crop(box), new.crop(box)
    scale = min(1.0, 650 / before.width)
    size = (round(before.width * scale), round(before.height * scale))
    before, after = before.resize(size), after.resize(size)
    tile = Image.new('RGB', (1320, max(90, size[1] + 45)), 'white')
    draw = ImageDraw.Draw(tile)
    draw.text((8, 3), new_path.name, fill='black')
    draw.text((8, 20), 'BEFORE', fill='black')
    draw.text((668, 20), 'AFTER', fill='black')
    tile.paste(before, (8, 40))
    tile.paste(after, (668, 40))
    tiles.append(tile)

sheet, y, n = Image.new('RGB', (1320, 1600), '#dddddd'), 0, 1
for tile in tiles:
    if y and y + tile.height > sheet.height:
        sheet.crop((0, 0, 1320, y)).save(output / f'sheet-{n:02}.png')
        sheet, y, n = Image.new('RGB', (1320, 1600), '#dddddd'), 0, n + 1
    if tile.height > sheet.height:
        tile.save(output / f'sheet-{n:02}.png')
        n += 1
        continue
    sheet.paste(tile, (0, y))
    y += tile.height + 8
if y:
    sheet.crop((0, 0, 1320, y)).save(output / f'sheet-{n:02}.png')
(output / 'changed.json').write_text(json.dumps(changed, indent=2))
print(f'{len(changed)} changed goldens; {n} review sheets in {output}')
