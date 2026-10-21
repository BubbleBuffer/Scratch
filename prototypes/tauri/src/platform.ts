// Platform flags and the on-screen keyboard's position (mobile).
import { createSignal } from "solid-js";

export const isAndroid = /Android/i.test(navigator.userAgent);
export const isMobile = isAndroid || /iPhone|iPad/i.test(navigator.userAgent) ||
  (matchMedia("(pointer: coarse)").matches && !matchMedia("(pointer: fine)").matches);

document.documentElement.classList.add(isMobile ? "mobile" : "desktop");
// The Android system font; headless tests use an Android user agent too but
// run on Windows, where the desktop stack applies.
if (isAndroid && "__TAURI_INTERNALS__" in window) document.documentElement.classList.add("android");

/** Bottom edge of the visible area in layout-viewport pixels (the top of the
 *  keyboard when it is open) and whether the keyboard is open. */
const [kb, setKb] = createSignal({ open: false, bottom: window.innerHeight });
export const keyboard = kb;

/** Headless tests have no on-screen keyboard; they call
 *  `window.__bakeoffKeyboard(px)` to simulate one of that height (0 = closed). */
const [kbSim, setKbSim] = createSignal(0);
export const keyboardSim = kbSim;

if (isMobile) {
  let fullH = window.innerHeight;
  let lastW = window.innerWidth;
  const update = () => {
    const sim = kbSim();
    if (sim > 0) {
      setKb({ open: true, bottom: window.innerHeight - sim });
      return;
    }
    const vv = window.visualViewport;
    const h = vv ? vv.height : window.innerHeight;
    const top = vv ? vv.offsetTop : 0;
    // The largest height seen per orientation is the height without keyboard.
    if (window.innerHeight > fullH || Math.abs(window.innerWidth - lastW) > 50) fullH = Math.max(window.innerHeight, h);
    lastW = window.innerWidth;
    const open = fullH - h > 150;
    const next = { open, bottom: top + h };
    const cur = kb();
    if (cur.open !== next.open || Math.abs(cur.bottom - next.bottom) > 0.5) setKb(next);
  };
  window.visualViewport?.addEventListener("resize", update);
  window.visualViewport?.addEventListener("scroll", update);
  window.addEventListener("resize", update);
  (window as any).__bakeoffKeyboard = (px: number) => { setKbSim(px); update(); };
  update();
}
