#!/bin/sh
# Rebuild notes-icon.svg from the BubbleBuffer kit through Pawth (pawthctl).
P=${PAWTHCTL:-~/Documents/Projects/Pawth/artifacts/pawthctl.exe}; K='"kit":"kits/bubblebuffer.kit.svg"'
cat > notes-icon.svg <<'SVG'
<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1280 1280">
  <g id="background" data-name="Background" data-icon-layer="background">
    <rect id="background-desk" data-name="Desk" x="0" y="0" width="1280" height="1280" fill="#F3EFE7"/>
  </g>
</svg>
SVG
$P open notes-icon.svg --workspace . --json >/dev/null
$P call kit.use --input "{$K}" --json >/dev/null
for p in book/pages book/ribbon book/cover book/ears book/outline book/rings face/cover; do
  $P call kit.insert_part --input "{$K,\"part\":\"$p\"}" --json >/dev/null
done
$P call elements.group --input '{"elements":["/Page block","/Bookmark tail","/Cat cover","/Notebook ears","/Cover outline","/Notebook rings","/Cover face"],"name":"Notebook cat"}' --json >/dev/null
$P call elements.transform --input '{"elements":"/Notebook cat","transform":{"scaleX":1.197214,"scaleY":1.197214,"x":-161.751,"y":-198.05}}' --json >/dev/null
$P call files.save --json >/dev/null
$P screenshot --output "${1:-options/a-spiral.png}" --max-dimension 640 --background milk
