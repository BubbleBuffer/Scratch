"""Build the Notes icon options in Pawth from the BubbleBuffer kit (run from design/icon)."""
import json, os, subprocess, sys

P = os.path.expanduser('~/Documents/Projects/Pawth/artifacts/pawthctl.exe')
KIT = 'kits/bubblebuffer.kit.svg'
BLANK = '''<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1280 1280">
  <g id="background" data-name="Background" data-icon-layer="background">
    <rect id="background-desk" data-name="Desk" x="0" y="0" width="1280" height="1280" fill="#F3EFE7"/>
  </g>
</svg>
'''
# name: (parts in stacking order, {part name: (dx, dy)}, (scale, x, y))
OPTIONS = {
    'b-hardcover': (['book/pages', 'book/ribbon', 'book/cover', 'book/spine', 'book/corner-spot', 'book/ears', 'book/outline', 'face/cover'],
                    {'Cover face': (36, 0)}, (0.94, 30, -24)),
    'c-open-book': (['spread/cover', 'spread/pages', 'spread/ears', 'face/spread'], {}, (0.9, 64, -30)),
    'd-strap-journal': (['book/pages', 'book/cover', 'book/eye-spot', 'book/ears', 'book/outline', 'book/strap', 'face/cover'],
                        {'Cover face': (-30, 0)}, (0.94, 23, -24)),
    'e-tail-bookmark': (['book/pages', 'book/tail', 'book/cover', 'book/ears', 'book/outline', 'face/cover'], {}, (0.94, 10, -24)),
}

def call(tool, payload=None):
    args = [P, 'call', tool, '--json'] + (['--input', json.dumps(payload)] if payload is not None else [])
    out = json.loads(subprocess.run(args, capture_output=True, text=True, check=True).stdout)
    if not out.get('ok'):
        sys.exit(f'{tool} failed: {out}')
    return out['result']

def top_names():
    return [e['name'] for e in call('elements.list')['elements'] if e['path'].count('/') == 1]

for name, (parts, nudges, (scale, x, y)) in OPTIONS.items():
    if len(sys.argv) > 1 and name not in sys.argv[1:]:
        continue
    doc = f'options/{name}.svg'
    open(doc, 'w', encoding='utf-8').write(BLANK)
    subprocess.run([P, 'open', doc, '--workspace', '.', '--json'], capture_output=True, check=True)
    call('kit.use', {'kit': KIT})
    before = set(top_names())
    for part in parts:
        call('kit.insert_part', {'kit': KIT, 'part': part})
    inserted = [n for n in top_names() if n not in before and n != 'Kit definitions']
    assert len(inserted) == len(parts), inserted
    for part_name, (dx, dy) in nudges.items():
        call('elements.transform', {'elements': f'/{part_name}', 'mode': 'delta', 'transform': {'x': dx, 'y': dy}})
    call('elements.group', {'elements': [f'/{n}' for n in inserted], 'name': 'Notebook cat'})
    call('elements.transform', {'elements': '/Notebook cat', 'transform': {'scaleX': scale, 'scaleY': scale, 'x': x, 'y': y}})
    call('files.save')
    subprocess.run([P, 'screenshot', '--output', f'options/{name}.png', '--max-dimension', '640', '--background', 'milk'], check=True)
