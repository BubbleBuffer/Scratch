// Inline text of one block: marks, rendering to DOM, reading back from DOM,
// and caret positions. Inside the editor all offsets are UTF-16 code units
// (what the DOM uses); conversion to the core's code-point offsets happens
// only at the API boundary (toApiMarks / fromApiMarks).
import type { ApiMark } from "../api";

export type MarkKey = "b" | "i" | "s" | "c" | "a";
export interface Mark { start: number; end: number; key: MarkKey; href?: string }

// ---------- API boundary ----------

/** UTF-16 index -> code point index for every position of `text`. */
function u16ToCp(text: string): Int32Array {
  const map = new Int32Array(text.length + 1);
  let cp = 0;
  for (let i = 0; i < text.length; i++) {
    map[i] = cp;
    const c = text.charCodeAt(i);
    if (c >= 0xd800 && c <= 0xdbff && i + 1 < text.length) { map[i + 1] = cp; i++; }
    cp++;
  }
  map[text.length] = cp;
  return map;
}

function isAscii(text: string) {
  for (let i = 0; i < text.length; i++) if (text.charCodeAt(i) > 0xd7ff) return false;
  return true;
}

export function fromApiMarks(text: string, marks: ApiMark[]): Mark[] {
  if (!marks.length) return [];
  let toU16 = (cp: number) => cp;
  if (!isAscii(text)) {
    const cps: number[] = [];
    for (let i = 0; i < text.length; i++) {
      cps.push(i);
      const c = text.charCodeAt(i);
      if (c >= 0xd800 && c <= 0xdbff) i++;
    }
    cps.push(text.length);
    toU16 = (cp) => cps[Math.min(cp, cps.length - 1)];
  }
  return marks.map((m) => {
    const k = m.kind;
    const key: MarkKey = typeof k === "object" ? "a" : k === "Bold" ? "b" : k === "Italic" ? "i" : k === "Strike" ? "s" : "c";
    return { start: toU16(m.start), end: toU16(m.end), key, href: typeof k === "object" ? k.Link : undefined };
  });
}

export function toApiMarks(text: string, marks: Mark[]): ApiMark[] {
  if (!marks.length) return [];
  const map = isAscii(text) ? null : u16ToCp(text);
  const cp = (i: number) => (map ? map[i] : i);
  const out: ApiMark[] = [];
  for (const m of marks) {
    const s = cp(m.start), e = cp(m.end);
    if (e <= s) continue;
    const kind = m.key === "a" ? { Link: m.href ?? "" } : m.key === "b" ? "Bold" : m.key === "i" ? "Italic" : m.key === "s" ? "Strike" : "Code";
    out.push({ start: s, end: e, kind: kind as ApiMark["kind"] });
  }
  out.sort((a, b) => a.start - b.start);
  return out;
}

// ---------- mark algebra ----------

const sameKind = (a: Mark, b: Mark) => a.key === b.key && (a.key !== "a" || a.href === b.href);

/** Merges overlapping or touching ranges of the same kind, drops empty ones. */
export function normalize(marks: Mark[], len: number): Mark[] {
  const clipped = marks
    .map((m) => ({ ...m, start: Math.max(0, Math.min(m.start, len)), end: Math.max(0, Math.min(m.end, len)) }))
    .filter((m) => m.end > m.start)
    .sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : a.start - b.start));
  const out: Mark[] = [];
  for (const m of clipped) {
    const last = out[out.length - 1];
    if (last && sameKind(last, m) && m.start <= last.end) last.end = Math.max(last.end, m.end);
    else out.push(m);
  }
  return out.sort((a, b) => a.start - b.start);
}

export function sliceMarks(marks: Mark[], a: number, b: number): Mark[] {
  const out: Mark[] = [];
  for (const m of marks) {
    const s = Math.max(m.start, a), e = Math.min(m.end, b);
    if (e > s) out.push({ ...m, start: s - a, end: e - a });
  }
  return out;
}

export const shiftMarks = (marks: Mark[], d: number) => marks.map((m) => ({ ...m, start: m.start + d, end: m.end + d }));

/** True when every character of [a, b) carries `key`. */
export function hasMark(marks: Mark[], key: MarkKey, a: number, b: number): boolean {
  if (b <= a) return false;
  let pos = a;
  for (const m of [...marks].filter((m) => m.key === key).sort((x, y) => x.start - y.start)) {
    if (m.start > pos) break;
    if (m.end > pos) pos = m.end;
    if (pos >= b) return true;
  }
  return pos >= b;
}

export function removeMark(marks: Mark[], key: MarkKey, a: number, b: number): Mark[] {
  const out: Mark[] = [];
  for (const m of marks) {
    if (m.key !== key || m.end <= a || m.start >= b) { out.push(m); continue; }
    if (m.start < a) out.push({ ...m, end: a });
    if (m.end > b) out.push({ ...m, start: b });
  }
  return out;
}

/** Toggles `key` over [a, b): removes it if the whole range has it, else adds it. */
export function toggleMark(marks: Mark[], len: number, key: MarkKey, a: number, b: number, href?: string): Mark[] {
  if (key !== "a" && hasMark(marks, key, a, b)) return normalize(removeMark(marks, key, a, b), len);
  let next = key === "a" ? removeMark(marks, "a", a, b) : marks;
  if (key === "a" && !href) return normalize(next, len);
  next = [...next, { start: a, end: b, key, href }];
  // Inline code and links do not nest with each other well; keep it simple.
  return normalize(next, len);
}

// ---------- DOM rendering ----------

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

const ORDER: MarkKey[] = ["a", "c", "b", "i", "s"];
const TAG: Record<MarkKey, string> = { a: "a", c: "code", b: "b", i: "i", s: "s" };

export function inlineHtml(text: string, marks: Mark[]): string {
  let html: string;
  if (!marks.length) html = esc(text);
  else {
    const cuts = new Set<number>([0, text.length]);
    for (const m of marks) { cuts.add(m.start); cuts.add(m.end); }
    const pts = [...cuts].sort((a, b) => a - b);
    html = "";
    for (let k = 0; k + 1 < pts.length; k++) {
      const a = pts[k], b = pts[k + 1];
      if (b <= a) continue;
      const on = marks.filter((m) => m.start <= a && m.end >= b);
      let open = "", close = "";
      for (const key of ORDER) {
        const m = on.find((x) => x.key === key);
        if (!m) continue;
        open += key === "a" ? `<a data-href="${esc(m.href ?? "")}" title="${esc(m.href ?? "")}">` : `<${TAG[key]}>`;
        close = `</${TAG[key]}>` + close;
      }
      html += open + esc(text.slice(a, b)) + close;
    }
  }
  // A trailing newline needs something after it to show as an empty line.
  if (text.endsWith("\n")) html += "<br>";
  return html;
}

export function renderInline(el: HTMLElement, text: string, marks: Mark[]) {
  if (!marks.length && !text.includes("\n")) {
    if (el.textContent !== text || el.childNodes.length > 1) el.textContent = text;
    return;
  }
  el.innerHTML = inlineHtml(text, marks);
}

/** Reads text and marks back from a block's editable element. `<br>` is a
 *  newline, except a trailing one (the browser's or ours, see above). */
export function readInline(root: HTMLElement): { text: string; marks: Mark[] } {
  let text = "";
  const marks: Mark[] = [];
  const lastBr = trailingBr(root);
  const walk = (node: Node, active: Mark[]) => {
    for (let c = node.firstChild; c; c = c.nextSibling) {
      if (c.nodeType === 3) {
        const s = (c as Text).data;
        if (!s) continue;
        const a = text.length;
        text += s;
        for (const m of active) marks.push({ ...m, start: a, end: text.length });
      } else if (c.nodeType === 1) {
        const el = c as HTMLElement;
        const tag = el.tagName;
        if (tag === "BR") { if (el !== lastBr) text += "\n"; continue; }
        let key: MarkKey | null = null;
        let href: string | undefined;
        if (tag === "B" || tag === "STRONG") key = "b";
        else if (tag === "I" || tag === "EM") key = "i";
        else if (tag === "S" || tag === "STRIKE" || tag === "DEL") key = "s";
        else if (tag === "CODE") key = "c";
        else if (tag === "A") { key = "a"; href = el.dataset.href ?? el.getAttribute("href") ?? ""; }
        else if (tag === "SPAN" && /bold|[67]00/.test(el.style.fontWeight)) key = "b";
        const block = tag === "DIV" || tag === "P";
        if (block && text.length && !text.endsWith("\n")) text += "\n";
        walk(el, key ? [...active, { start: 0, end: 0, key, href }] : active);
      }
    }
  };
  walk(root, []);
  return { text, marks: normalize(marks, text.length) };
}

function trailingBr(root: Node): Node | null {
  let n: Node | null = root.lastChild;
  while (n && n.nodeType === 1 && (n as Element).tagName !== "BR" && n.lastChild) n = n.lastChild;
  return n && n.nodeType === 1 && (n as Element).tagName === "BR" ? n : null;
}

// ---------- caret ----------

/** UTF-16 offset of a DOM position inside `root` (as readInline counts). */
export function domOffset(root: HTMLElement, node: Node, offset: number): number {
  let count = 0;
  let done = false;
  const lastBr = trailingBr(root);
  // An element position means "before child #offset".
  let target: Node | null = node;
  let targetOff = offset;
  if (node.nodeType === 1) {
    if (offset < node.childNodes.length) { target = node.childNodes[offset]; targetOff = -1; }
    else { target = null; targetOff = -2; } // after all children of `node`
  }
  const walk = (n: Node): void => {
    for (let c = n.firstChild; c && !done; c = c.nextSibling) {
      if (c === target && targetOff === -1) { done = true; return; }
      if (c.nodeType === 3) {
        if (c === target) { count += targetOff; done = true; return; }
        count += (c as Text).data.length;
      } else if (c.nodeType === 1) {
        const el = c as HTMLElement;
        if (el.tagName === "BR") { if (el !== lastBr) count++; continue; }
        const block = el.tagName === "DIV" || el.tagName === "P";
        if (block && count > 0) count++;
        walk(el);
      }
    }
    if (!done && targetOff === -2 && n === node) done = true;
  };
  walk(root);
  return count;
}

export interface Sel { start: number; end: number; collapsed: boolean }

export function getSel(root: HTMLElement): Sel | null {
  const s = window.getSelection();
  if (!s || !s.rangeCount) return null;
  const r = s.getRangeAt(0);
  if (!root.contains(r.startContainer) || !root.contains(r.endContainer)) return null;
  const start = domOffset(root, r.startContainer, r.startOffset);
  const end = r.collapsed ? start : domOffset(root, r.endContainer, r.endOffset);
  return { start, end, collapsed: r.collapsed };
}

/** The DOM position of UTF-16 offset `off` inside `root`. */
export function domPoint(root: HTMLElement, off: number): [Node, number] {
  let remaining = off;
  const lastBr = trailingBr(root);
  let found: [Node, number] | null = null;
  const walk = (n: Node): void => {
    for (let c = n.firstChild; c && !found; c = c.nextSibling) {
      if (c.nodeType === 3) {
        const len = (c as Text).data.length;
        if (remaining <= len) { found = [c, remaining]; return; }
        remaining -= len;
      } else if (c.nodeType === 1) {
        const el = c as HTMLElement;
        if (el.tagName === "BR") {
          if (el === lastBr) continue;
          if (remaining === 0) { found = [el.parentNode!, indexOf(el)]; return; }
          remaining--;
          if (remaining === 0) { found = [el.parentNode!, indexOf(el) + 1]; return; }
          continue;
        }
        walk(el);
      }
    }
  };
  walk(root);
  return found ?? [root, root.childNodes.length && lastBr ? indexOf(lastBr) : root.childNodes.length];
}

const indexOf = (n: Node) => Array.prototype.indexOf.call(n.parentNode!.childNodes, n) as number;

export function setSel(root: HTMLElement, start: number, end = start) {
  const s = window.getSelection();
  if (!s) return;
  const [an, ao] = domPoint(root, start);
  const r = document.createRange();
  r.setStart(an, ao);
  if (end !== start) { const [bn, bo] = domPoint(root, end); r.setEnd(bn, bo); } else r.collapse(true);
  s.removeAllRanges();
  s.addRange(r);
}

/** Bounding rect of the caret or selection, for placing menus. */
export function selRect(): DOMRect | null {
  const s = window.getSelection();
  if (!s || !s.rangeCount) return null;
  const r = s.getRangeAt(0);
  let rect = r.getBoundingClientRect();
  if (!rect.width && !rect.height) {
    const rects = r.getClientRects();
    if (rects.length) rect = rects[0];
    else {
      const el = (r.startContainer.nodeType === 1 ? r.startContainer : r.startContainer.parentElement) as HTMLElement;
      rect = el.getBoundingClientRect();
    }
  }
  return rect;
}

/** Word around a collapsed caret, for formatting on mobile without a selection. */
export function wordAt(text: string, at: number): [number, number] {
  let a = at, b = at;
  const w = /[\p{L}\p{N}_'-]/u;
  while (a > 0 && w.test(text[a - 1])) a--;
  while (b < text.length && w.test(text[b])) b++;
  return [a, b];
}
