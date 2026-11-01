"""Capture the settings mockup into shots/ with headless Chromium.

Needs the design/ folder served at http://localhost:8781 (python -m http.server 8781
--directory design) and a chrome-headless-shell (Playwright's, or set CHROME).
Each shot is taken in a tall window, then the empty desk below the page is trimmed.
Usage: python capture.py [name ...]
"""
import os, subprocess, sys, glob
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
BASE = "http://localhost:8781/settings-redesign/index.html#chrome=0&tall=1&"
CHROME = os.environ.get("CHROME") or sorted(glob.glob(os.path.expanduser(
    "~/AppData/Local/ms-playwright/chromium_headless_shell-*/chrome-headless-shell-win64/chrome-headless-shell.exe")))[-1]

DESK = (1440, 3200, 1)
PHONE = (390, 3200, 2)
SHOTS = {
    "desktop-light-general": (DESK, "page=general"),
    "desktop-light-sync": (DESK, "page=sync"),
    "desktop-light-sync-not-set-up": (DESK, "page=sync&sync=off&data=empty"),
    "desktop-light-conflicts": (DESK, "page=conflicts"),
    "desktop-light-storage-empty": (DESK, "page=storage&data=empty"),
    "desktop-light-agent": (DESK, "page=agent"),
    "desktop-light-agent-help-open": (DESK, "page=agent&help=1"),
    "desktop-light-agent-help-hover": (DESK, "page=agent&hoverHelp=model"),
    "desktop-light-agent-tools-write": (DESK, "page=agent&tools=write"),
    "desktop-light-agent-hub": (DESK, "page=agent&runner=hub"),
    "desktop-light-connectors": (DESK, "page=connectors"),
    "desktop-light-connectors-open": (DESK, "page=connectors&open=tools,apps,hub"),
    "desktop-light-connectors-tools-locked": (DESK, "page=connectors&open=tools&tools=write"),
    "desktop-light-connectors-empty-add-server": (DESK, "page=connectors&open=tools&data=empty&edit=server"),
    "desktop-light-connectors-signed-out": (DESK, "page=connectors&open=apps&sync=off"),
    "desktop-light-connectors-new-token": (DESK, "page=connectors&open=apps&data=empty&edit=token"),
    "desktop-dark-agent": (DESK, "page=agent&theme=dark"),
    "desktop-dark-sync": (DESK, "page=sync&theme=dark"),
    "desktop-dark-connectors-open": (DESK, "page=connectors&open=tools,apps,hub&theme=dark"),
    "phone-light-list": (PHONE, "page=agent&phoneList=1&runner=hub"),
    "phone-light-agent": (PHONE, "page=agent&runner=hub"),
    "phone-light-sync": (PHONE, "page=sync"),
    "phone-light-reminders": (PHONE, "page=reminders"),
    "phone-light-connectors": (PHONE, "page=connectors&open=apps"),
    "phone-dark-list": (PHONE, "page=agent&phoneList=1&runner=hub&theme=dark"),
    "phone-dark-agent": (PHONE, "page=agent&runner=hub&theme=dark"),
}


def trim(path, scale):
    im = Image.open(path).convert("RGB")
    w, h = im.size
    bg = im.getpixel((2, h - 2))
    y = h - 1
    while y > 0 and all(im.getpixel((x, y)) == bg for x in range(0, w, max(1, w // 64))):
        y -= 1
    im.crop((0, 0, w, min(h, y + 24 * scale))).save(path, optimize=True)


def main(names):
    os.makedirs(os.path.join(HERE, "shots"), exist_ok=True)
    for name in names or SHOTS:
        (w, h, scale), q = SHOTS[name]
        out = os.path.join(HERE, "shots", name + ".png")
        subprocess.run([CHROME, "--headless", "--hide-scrollbars", "--disable-gpu", f"--window-size={w},{h}",
                        f"--force-device-scale-factor={scale}", "--virtual-time-budget=4000", f"--screenshot={out}", BASE + q],
                       check=True, capture_output=True, timeout=60)
        trim(out, scale)
        print("shot", name)


if __name__ == "__main__":
    main(sys.argv[1:])
