// The block editor. Each block is its own small contenteditable element
// (rendered once, then left to the browser while typing); structure changes
// go through the Doc model. See wiki/prototypes/tauri.md for why.
import {
  For, Show, Switch, Match, createEffect, createMemo, createSelector, createSignal, on, onCleanup, onMount,
  type JSX,
} from "solid-js";
import { api, type Kind } from "../api";
import { afterPaint, blog, inputMeter, pageFrames } from "../bakeoff";
import { relTime } from "../Home";
import type { Route } from "../App";
import {
  IArrowLeft, IBack, IBullet, ICheck, IChevDown, IChevR, ICode, ICopy, IDivider, IDown, IGrip, IH1, IH2, IH3, IIndent,
  IKbHide, ILink, IMore, INumbered, IOutdent, IPage, IPlus, IQuote, IText, ITodo, ITrash, ITurn, IUp,
} from "../icons";
import { isMobile, keyboard } from "../platform";
import { Doc, saves, type Node } from "./model";
import {
  getSel, hasMark, readInline, renderInline, selRect, setSel, shiftMarks, sliceMarks, toggleMark, wordAt,
  type Mark, type MarkKey,
} from "./inline";

interface KindInfo { kind: Kind; name: string; icon: () => JSX.Element; hint: string; keys: string }
export const KINDS: KindInfo[] = [
  { kind: "Paragraph", name: "Text", icon: () => <IText />, hint: "", keys: "text plain paragraph" },
  { kind: "Heading1", name: "Heading 1", icon: () => <IH1 />, hint: "#", keys: "heading1 h1 title" },
  { kind: "Heading2", name: "Heading 2", icon: () => <IH2 />, hint: "##", keys: "heading2 h2 subtitle" },
  { kind: "Heading3", name: "Heading 3", icon: () => <IH3 />, hint: "###", keys: "heading3 h3" },
  { kind: "Bulleted", name: "Bulleted list", icon: () => <IBullet />, hint: "-", keys: "bulleted list ul bullet" },
  { kind: "Numbered", name: "Numbered list", icon: () => <INumbered />, hint: "1.", keys: "numbered list ol ordered" },
  { kind: "Todo", name: "To-do list", icon: () => <ITodo />, hint: "[]", keys: "todo to-do checkbox task" },
  { kind: "Quote", name: "Quote", icon: () => <IQuote />, hint: ">", keys: "quote blockquote" },
  { kind: "Code", name: "Code", icon: () => <ICode />, hint: "```", keys: "code snippet" },
  { kind: "Divider", name: "Divider", icon: () => <IDivider />, hint: "---", keys: "divider hr line separator" },
];
const kindName = (k: Kind | undefined) => KINDS.find((x) => x.kind === k)?.name ?? "Text";

const KCLASS: Record<Kind, string> = {
  Paragraph: "p", Heading1: "h1", Heading2: "h2", Heading3: "h3", Bulleted: "bul", Numbered: "num",
  Todo: "todo", Quote: "quote", Code: "code", Divider: "div",
};
const PH: Record<Kind, string> = {
  Paragraph: isMobile ? "Type '/' for commands" : "Write, press '/' for commands…", Heading1: "Heading 1",
  Heading2: "Heading 2", Heading3: "Heading 3", Bulleted: "List", Numbered: "List", Todo: "To-do",
  Quote: "Empty quote", Code: "", Divider: "",
};
/** Line height of the block's first line, for aligning the drag handle. */
const LINE: Partial<Record<Kind, number>> = { Heading1: 39, Heading2: 31.2, Heading3: 26 };

/** Nested numbered lists count 1., a., i. */
function numLabel(n: number, depth: number) {
  const d = depth % 3;
  if (d === 1) { let s = "", x = n; while (x > 0) { x--; s = String.fromCharCode(97 + (x % 26)) + s; x = Math.floor(x / 26); } return s; }
  if (d === 2) {
    const r: [number, string][] = [[10, "x"], [9, "ix"], [5, "v"], [4, "iv"], [1, "i"]];
    let s = "", x = n;
    for (const [v, t] of r) while (x >= v) { s += t; x -= v; }
    return s;
  }
  return String(n);
}

const MD_RULES: [string, Kind][] = [
  ["### ", "Heading3"], ["## ", "Heading2"], ["# ", "Heading1"], ["- ", "Bulleted"], ["* ", "Bulleted"],
  ["+ ", "Bulleted"], ["1. ", "Numbered"], ["[] ", "Todo"], ["[ ] ", "Todo"], ["> ", "Quote"],
  ["```", "Code"], ["---", "Divider"],
];

interface Slash { id: string; mode: "slash" | "turn"; filter: string; index: number; x: number; y: number; above: boolean }
interface SelMenu { id: string; start: number; end: number; x: number; y: number }
interface LinkPop { id: string; start: number; end: number; x: number; y: number; href: string }

/** Marks after replacing [a, b) with `ins` characters. */
function spliceMarks(marks: Mark[], a: number, b: number, ins: number): Mark[] {
  const cut = b - a;
  return marks.map((m) => {
    let s = m.start <= a ? m.start : m.start <= b ? a : m.start - cut;
    let e = m.end <= a ? m.end : m.end <= b ? a : m.end - cut;
    if (ins) {
      // Text typed inside a mark extends it; a mark starting at the cut moves right.
      if (s > a || (s === a && m.start >= a)) s += ins;
      if (e > a || (e === a && m.end > a && s < a)) e += ins;
    }
    return { ...m, start: s, end: e };
  });
}

export function Editor(props: {
  route: Route; onBack: () => void; onTitle: (t: string) => void; onSheet: (open: boolean) => void;
  registerBack: (fn: (() => boolean) | null) => void;
}) {
  const [doc, setDoc] = createSignal<Doc | null>(null);
  const [title, setTitle] = createSignal("");
  const [saveState, setSaveState] = createSignal("");
  const [limit, setLimit] = createSignal(40);
  const [active, setActive] = createSignal<string | null>(null);
  const isActive = createSelector(active);
  const [slash, setSlash] = createSignal<Slash | null>(null);
  const [selMenu, setSelMenu] = createSignal<SelMenu | null>(null);
  const [linkPop, setLinkPop] = createSignal<LinkPop | null>(null);
  const [sheet, setSheet] = createSignal<{ id: string; shown: boolean; page: "main" | "turn" } | null>(null);
  const [blockMenu, setBlockMenu] = createSignal<{ id: string; x: number; y: number; turn: boolean } | null>(null);
  const [hover, setHover] = createSignal<{ id: string; top: number; left: number } | null>(null);
  const [drop, setDrop] = createSignal<{ top: number; left: number } | null>(null);
  const [toast, setToast] = createSignal("");
  const [lined, setLined] = createSignal(false);
  const [fmtMode, setFmtMode] = createSignal(false);
  const [edited, setEdited] = createSignal<number | undefined>(props.route.updated);
  const [menuFilter, setMenuFilter] = createSignal("");

  const els = new Map<string, HTMLDivElement>();
  let scroller!: HTMLDivElement;
  let blocksEl!: HTMLDivElement;
  let titleEl!: HTMLDivElement;
  let lastKeyTs = 0;

  const shown = createMemo(() => { const d = doc(); return d ? d.order().slice(0, limit()) : []; });
  const node = (id: string) => doc()!.nodes.get(id);

  // ---------- loading ----------
  onMount(async () => {
    pageFrames.reset();
    pageFrames.attach(scroller);
    inputMeter.reset();
    const tReq = performance.now();
    let page;
    try { page = await api.loadPage(props.route.id); } catch (e) { setToast(`Could not open page: ${e}`); return; }
    const tLoaded = performance.now();
    const d = new Doc(page, (s) => {
      setSaveState(s.startsWith("error") ? "Not saved" : s === "saving" ? "Saving…" : "Saved");
      if (s === "saved") setEdited(Date.now());
      if (s.startsWith("error")) setToast(`Could not save: ${s.slice(7)}`);
    });
    setTitle(page.title);
    titleEl.textContent = page.title;
    if (!d.top.length) d.insertAfter(null, null, "Paragraph");
    setDoc(d);
    const total = d.order().length;
    afterPaint((t1) => {
      blog(`page_open ms=${(t1 - props.route.t0).toFixed(1)} load_ms=${(tLoaded - tReq).toFixed(1)} blocks=${total} title="${page.title.replace(/"/g, "'")}"`);
      // Render the rest progressively, a chunk per frame.
      const grow = () => {
        if (limit() >= d.order().length) {
          afterPaint((t2) => blog(`page_rendered ms=${(t2 - props.route.t0).toFixed(1)} blocks=${d.order().length}`));
          return;
        }
        setLimit((l) => l + 150);
        requestAnimationFrame(grow);
      };
      requestAnimationFrame(grow);
      if (!page.title) titleEl.focus();
    });
  });

  const renderAll = () => { const d = doc(); if (d && limit() < d.order().length) setLimit(d.order().length); };

  let renameTimer = 0;
  const onTitleInput = () => {
    const t = (titleEl.textContent ?? "").replace(/\n/g, " ");
    setTitle(t);
    props.onTitle(t);
    clearTimeout(renameTimer);
    renameTimer = window.setTimeout(() => api.renamePage(props.route.id, t).catch((e) => setToast(`${e}`)), 300);
  };

  const leave = async () => {
    clearTimeout(renameTimer);
    const t = (titleEl.textContent ?? "").replace(/\n/g, " ");
    const d = doc();
    const jobs: Promise<unknown>[] = [];
    if (d && t !== d.page.title) jobs.push(api.renamePage(props.route.id, t));
    if (d) jobs.push(d.saver.settle());
    await Promise.allSettled(jobs);
  };
  onCleanup(() => { saves.last = leave(); });

  // ---------- focus & caret ----------
  function focusBlock(id: string, start: number | "end" = 0, end?: number) {
    renderAll();
    const n = node(id);
    if (!n || n.kind === "Divider") return;
    setActive(id);
    const el = els.get(id);
    if (!el) return;
    if (document.activeElement !== el) el.focus({ preventScroll: true });
    const s = start === "end" ? n.text.length : Math.min(start, n.text.length);
    setSel(el, s, end ?? s);
    ensureVisible(el);
  }

  function ensureVisible(el: HTMLElement) {
    const r = selRect() ?? el.getBoundingClientRect();
    const sr = scroller.getBoundingClientRect();
    const bottom = isMobile && keyboard().open ? Math.min(sr.bottom, keyboard().bottom - 44) : sr.bottom;
    if (r.bottom > bottom - 16) scroller.scrollTop += r.bottom - bottom + 48;
    else if (r.top < sr.top + 8) scroller.scrollTop -= sr.top + 48 - r.top;
  }

  function focusNear(id: string, x: number, fromBelow: boolean) {
    renderAll();
    const n = node(id);
    const el = els.get(id);
    if (!n || !el) return;
    setActive(id);
    el.focus({ preventScroll: true });
    const r = el.getBoundingClientRect();
    const y = fromBelow ? r.bottom - 8 : r.top + 10;
    const pos = (document as any).caretRangeFromPoint?.(x, y) as Range | null;
    if (pos && el.contains(pos.startContainer)) {
      const s = window.getSelection()!;
      s.removeAllRanges();
      s.addRange(pos);
    } else setSel(el, fromBelow ? n.text.length : 0);
    ensureVisible(el);
  }

  const blockOf = (t: EventTarget | null) => {
    const row = (t as HTMLElement | null)?.closest?.(".blk") as HTMLElement | null;
    if (!row) return null;
    const n = node(row.dataset.id!);
    const el = els.get(row.dataset.id!);
    return n && el ? { n, el, row } : null;
  };

  // ---------- text edits ----------
  function replaceRange(n: Node, el: HTMLElement, a: number, b: number, ins: string) {
    const d = doc()!;
    const text = n.text.slice(0, a) + ins + n.text.slice(b);
    d.setText(n, text, spliceMarks(n.marks, a, b, ins.length));
    setSel(el, a + ins.length);
  }

  function afterTextInput(n: Node, el: HTMLElement) {
    const d = doc()!;
    const sel = getSel(el);
    if (!sel) return;
    // Markdown shortcuts at the start of a plain block.
    if (n.kind !== "Code") {
      for (const [p, kind] of MD_RULES) {
        if (sel.start === p.length && n.text.startsWith(p) && kind !== n.kind) {
          closeSlash();
          d.setText(n, n.text.slice(p.length), sliceMarks(n.marks, p.length, n.text.length));
          d.setKind(n, kind);
          if (kind === "Divider") {
            const next = d.insertAfter(n, n.parent, "Paragraph", n.text, n.marks);
            d.setText(n, "", []);
            focusBlock(next.id, 0);
          } else focusBlock(n.id, 0);
          return;
        }
      }
    }
    // The `/` menu: `/` typed at the start of a block, filter is what follows.
    const s = slash();
    const upto = n.text.slice(0, sel.start);
    if (n.kind !== "Code" && upto.startsWith("/") && !upto.includes("\n")) {
      const filter = upto.slice(1);
      if (s && s.id === n.id && s.mode === "slash") {
        if (filter.endsWith("  ") || (filter.endsWith(" ") && !matches(filter).length)) closeSlash();
        else setSlash({ ...s, filter, index: 0 });
      } else if (sel.start === 1) openSlash(n, "slash");
    } else if (s && s.mode === "slash") closeSlash();
  }

  // ---------- slash menu ----------
  const matches = (filter: string) => {
    const f = filter.trim().toLowerCase();
    if (!f) return KINDS;
    return KINDS.filter((k) => k.name.toLowerCase().includes(f) || k.keys.split(" ").some((w) => w.startsWith(f)));
  };
  const slashItems = createMemo(() => { const s = slash(); return s ? matches(s.filter) : []; });

  function openSlash(n: Node, mode: "slash" | "turn") {
    const kb = keyboard();
    let x = 8, y = 0, above = false;
    if (isMobile && kb.open) {
      // A popover pinned just above the keyboard toolbar.
      y = kb.bottom - 44 - 8; above = true;
    } else {
      const r = selRect() ?? els.get(n.id)!.getBoundingClientRect();
      const limitY = isMobile && kb.open ? kb.bottom - 44 : window.innerHeight;
      const h = isMobile ? 280 : 400;
      x = Math.max(8, Math.min(r.left - 8, window.innerWidth - (isMobile ? 348 : 332)));
      if (r.bottom + 6 + h > limitY && r.top - 6 - h > 0) { y = r.top - 6; above = true; }
      else y = r.bottom + 6;
    }
    setSlash({ id: n.id, mode, filter: "", index: 0, x, y, above });
  }
  const closeSlash = () => setSlash(null);

  function pickKind(kind: Kind) {
    const s = slash();
    const d = doc()!;
    if (!s) return;
    const n = node(s.id);
    closeSlash();
    if (!n) return;
    if (s.mode === "slash" && n.text.startsWith("/")) {
      const cut = 1 + s.filter.length;
      d.setText(n, n.text.slice(cut), sliceMarks(n.marks, cut, n.text.length));
    }
    if (kind === "Divider") {
      const rest = n.text, restMarks = n.marks;
      d.setKind(n, "Divider");
      const next = d.insertAfter(n, n.parent, "Paragraph", rest, restMarks);
      focusBlock(next.id, 0);
      return;
    }
    d.setKind(n, kind);
    focusBlock(n.id, 0);
  }

  // ---------- formatting ----------
  function format(key: MarkKey) {
    const id = active();
    const d = doc();
    if (!id || !d) return;
    const n = node(id)!, el = els.get(id)!;
    if (n.kind === "Code" || n.kind === "Divider") return;
    const sel = getSel(el);
    if (!sel) return;
    let [a, b] = [sel.start, sel.end];
    if (a === b) [a, b] = wordAt(n.text, a);
    if (a === b) return;
    if (key === "a") { openLink(n, a, b); return; }
    d.setText(n, n.text, toggleMark(n.marks, n.text.length, key, a, b));
    setSel(el, sel.start, sel.end);
    updateSelMenu();
  }

  function openLink(n: Node, a: number, b: number) {
    const r = selRect() ?? els.get(n.id)!.getBoundingClientRect();
    const existing = n.marks.find((m) => m.key === "a" && m.start <= a && m.end >= b);
    const kb = keyboard();
    const y = isMobile && kb.open ? kb.bottom - 44 - 52 : Math.min(r.bottom + 8, window.innerHeight - 60);
    setSelMenu(null);
    setLinkPop({ id: n.id, start: a, end: b, x: isMobile ? 12 : Math.max(8, Math.min(r.left, window.innerWidth - 350)), y, href: existing?.href ?? "" });
  }

  function applyLink(href: string) {
    const p = linkPop();
    setLinkPop(null);
    const d = doc();
    if (!p || !d) return;
    const n = node(p.id);
    if (!n) return;
    href = href.trim();
    if (href && !/^[a-z][a-z0-9+.-]*:/i.test(href)) href = "https://" + href;
    d.setText(n, n.text, toggleMark(n.marks, n.text.length, "a", p.start, p.end, href || undefined));
    focusBlock(n.id, p.end);
  }

  // Desktop: menu over a text selection.
  let mouseDown = false;
  function updateSelMenu() {
    if (isMobile || mouseDown || linkPop()) return;
    const s = window.getSelection();
    if (!s || s.isCollapsed || !s.rangeCount) { setSelMenu(null); return; }
    const b = blockOf(s.anchorNode?.parentElement ?? null);
    if (!b || !b.el.contains(s.focusNode) || b.n.kind === "Code") { setSelMenu(null); return; }
    const sel = getSel(b.el);
    if (!sel || sel.collapsed) { setSelMenu(null); return; }
    const r = s.getRangeAt(0).getBoundingClientRect();
    const w = 236;
    setSelMenu({ id: b.n.id, start: sel.start, end: sel.end, x: Math.max(8, Math.min(r.left + r.width / 2 - w / 2, window.innerWidth - w - 8)), y: Math.max(8, r.top - 36 - 8) });
  }
  let selRaf = 0;
  const onSelChange = () => { cancelAnimationFrame(selRaf); selRaf = requestAnimationFrame(updateSelMenu); };
  const onDocDown = () => { mouseDown = true; };
  const onDocUp = () => { mouseDown = false; onSelChange(); };
  if (!isMobile) {
    document.addEventListener("selectionchange", onSelChange);
    document.addEventListener("mousedown", onDocDown);
    document.addEventListener("mouseup", onDocUp);
    onCleanup(() => {
      document.removeEventListener("selectionchange", onSelChange);
      document.removeEventListener("mousedown", onDocDown);
      document.removeEventListener("mouseup", onDocUp);
    });
  }

  // ---------- structure ----------
  function doSplit(n: Node, el: HTMLElement) {
    const sel = getSel(el);
    const d = doc()!;
    let at = sel ? sel.start : n.text.length;
    if (sel && !sel.collapsed) { replaceRange(n, el, sel.start, sel.end, ""); at = sel.start; }
    const [m, off] = d.split(n, at);
    focusBlock(m.id, off);
  }

  function doBackspace(n: Node) {
    const r = doc()!.backspaceAtStart(n);
    if (r) focusBlock(r[0].id, r[1]);
  }

  function doDeleteForward(n: Node) {
    const d = doc()!;
    const nextId = d.nextVisible(n.id);
    if (!nextId) return;
    const next = node(nextId)!;
    const at = n.text.length;
    if (next.kind === "Divider") d.remove(next);
    else d.mergeInto(n, next);
    focusBlock(n.id, at);
  }

  function keepCaret(fn: (n: Node) => unknown) {
    const id = active();
    if (!id) return;
    const n = node(id)!, el = els.get(id)!;
    const sel = getSel(el);
    fn(n);
    focusBlock(id, sel?.start ?? 0, sel?.end);
  }

  // ---------- events on the blocks container ----------
  function onBeforeInput(e: InputEvent) {
    const b = blockOf(e.target);
    if (!b) return;
    const { n, el } = b;
    const t = e.inputType;
    if (t === "insertText" || t === "insertCompositionText" || t === "deleteContentBackward" || t === "insertReplacementText") {
      inputMeter.record(lastKeyTs && performance.now() - lastKeyTs < 100 ? lastKeyTs : e.timeStamp);
      lastKeyTs = 0;
    }
    switch (t) {
      case "insertParagraph":
        e.preventDefault();
        if (slash()) { const it = slashItems()[slash()!.index]; if (it) pickKind(it.kind); else closeSlash(); return; }
        if (n.kind === "Code") { const s = getSel(el); if (s) replaceRange(n, el, s.start, s.end, "\n"); return; }
        doSplit(n, el);
        return;
      case "insertLineBreak": {
        e.preventDefault();
        const s = getSel(el);
        if (s) replaceRange(n, el, s.start, s.end, "\n");
        return;
      }
      case "deleteContentBackward": {
        const s = getSel(el);
        if (s && s.collapsed && s.start === 0) { e.preventDefault(); doBackspace(n); }
        return;
      }
      case "historyUndo": case "historyRedo": case "formatBold": case "formatItalic": case "formatUnderline":
      case "formatStrikeThrough": case "insertFromDrop": case "insertFromPaste": case "insertFromPasteAsQuotation":
        e.preventDefault();
        return;
    }
  }

  function onInput(e: InputEvent) {
    const b = blockOf(e.target);
    if (!b) return;
    const { n, el } = b;
    const r = readInline(el);
    n.text = r.text;
    n.marks = r.marks;
    doc()!.textChanged(n);
    if (!e.isComposing) afterTextInput(n, el);
  }

  function onCompositionEnd(e: CompositionEvent) {
    const b = blockOf(e.target);
    if (b) afterTextInput(b.n, b.el);
  }

  function onPaste(e: ClipboardEvent) {
    const b = blockOf(e.target);
    if (!b) return;
    e.preventDefault();
    const d = doc()!;
    const { n, el } = b;
    const text = (e.clipboardData?.getData("text/plain") ?? "").replace(/\r\n?/g, "\n");
    if (!text) return;
    const sel = getSel(el) ?? { start: n.text.length, end: n.text.length, collapsed: true };
    const lines = n.kind === "Code" ? [text] : text.split("\n");
    if (lines.length === 1) { replaceRange(n, el, sel.start, sel.end, text); return; }
    const rest = n.text.slice(sel.end), restMarks = sliceMarks(n.marks, sel.end, n.text.length);
    d.setText(n, n.text.slice(0, sel.start) + lines[0], sliceMarks(n.marks, 0, sel.start));
    let prev = n;
    const kind: Kind = n.kind === "Bulleted" || n.kind === "Numbered" || n.kind === "Todo" ? n.kind : "Paragraph";
    for (let i = 1; i < lines.length; i++) {
      const last = i === lines.length - 1;
      prev = d.insertAfter(prev, n.parent, kind, last ? lines[i] + rest : lines[i], last ? shiftMarks(restMarks, lines[i].length) : []);
    }
    focusBlock(prev.id, lines[lines.length - 1].length);
  }

  function lineEdge(el: HTMLElement, up: boolean) {
    const r = selRect();
    if (!r) return true;
    const er = el.getBoundingClientRect();
    const lh = parseFloat(getComputedStyle(el).lineHeight) || 24;
    return up ? r.top < er.top + lh * 0.8 : r.bottom > er.bottom - lh * 0.8;
  }

  function onKeyDown(e: KeyboardEvent) {
    const b = blockOf(e.target);
    if (!b) return;
    const { n, el } = b;
    const d = doc()!;
    lastKeyTs = e.timeStamp;
    const s = slash();
    if (s) {
      const items = slashItems();
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const k = items.length || 1;
        setSlash({ ...s, index: (s.index + (e.key === "ArrowDown" ? 1 : k - 1)) % k });
        document.querySelector(".slash .item.sel")?.scrollIntoView({ block: "nearest" });
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") { e.preventDefault(); const it = items[s.index]; if (it) pickKind(it.kind); else closeSlash(); return; }
      if (e.key === "Escape") { e.preventDefault(); closeSlash(); return; }
    }
    if (e.isComposing || e.keyCode === 229) return;
    const mod = e.ctrlKey || e.metaKey;
    const key = e.key.toLowerCase();
    if (mod && !e.altKey) {
      if (key === "b" && !e.shiftKey) { e.preventDefault(); format("b"); return; }
      if (key === "i" && !e.shiftKey) { e.preventDefault(); format("i"); return; }
      if (key === "e" && !e.shiftKey) { e.preventDefault(); format("c"); return; }
      if (key === "k" && !e.shiftKey) { e.preventDefault(); format("a"); return; }
      if ((key === "s" || key === "x") && e.shiftKey) { e.preventDefault(); format("s"); return; }
      if (e.shiftKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
        e.preventDefault(); keepCaret((x) => (e.key === "ArrowUp" ? d.moveUp(x) : d.moveDown(x))); return;
      }
      if (key === "z" || key === "y") { e.preventDefault(); return; } // no undo in the prototype
    }
    const sel = getSel(el);
    switch (e.key) {
      case "Enter":
        if (e.shiftKey || n.kind === "Code") return; // beforeinput inserts the newline
        e.preventDefault();
        doSplit(n, el);
        return;
      case "Backspace":
        if (sel && sel.collapsed && sel.start === 0) { e.preventDefault(); doBackspace(n); }
        return;
      case "Delete":
        if (sel && sel.collapsed && sel.start === n.text.length) { e.preventDefault(); doDeleteForward(n); }
        return;
      case "Tab":
        e.preventDefault();
        keepCaret((x) => (e.shiftKey ? d.outdent(x) : d.indent(x)));
        return;
      case "Escape":
        el.blur();
        return;
      case "ArrowUp": case "ArrowDown": {
        if (e.shiftKey) return;
        const up = e.key === "ArrowUp";
        if (!lineEdge(el, up)) return;
        let id: string | null = n.id;
        do id = up ? d.prevVisible(id) : d.nextVisible(id); while (id && node(id)!.kind === "Divider");
        if (!id) { if (up) { e.preventDefault(); titleEl.focus(); } return; }
        e.preventDefault();
        const r = selRect();
        focusNear(id, r ? r.left : el.getBoundingClientRect().left, up);
        return;
      }
      case "ArrowLeft": case "ArrowRight": {
        if (e.shiftKey || !sel || !sel.collapsed) return;
        const left = e.key === "ArrowLeft";
        if (left ? sel.start !== 0 : sel.start !== n.text.length) return;
        let id: string | null = n.id;
        do id = left ? d.prevVisible(id) : d.nextVisible(id); while (id && node(id)!.kind === "Divider");
        if (!id) return;
        e.preventDefault();
        focusBlock(id, left ? "end" : 0);
        return;
      }
    }
  }

  function onFocusIn(e: FocusEvent) {
    const b = blockOf(e.target);
    if (b && (e.target as HTMLElement).classList.contains("txt")) setActive(b.n.id);
  }
  function onFocusOut(e: FocusEvent) {
    const to = e.relatedTarget as HTMLElement | null;
    if (to && (to.closest(".blk") || to.closest(".menu") || to.closest(".kbbar"))) return;
    setTimeout(() => {
      const a = document.activeElement as HTMLElement | null;
      if (a && a.closest?.(".blk .txt")) return;
      if (!linkPop()) { setActive(null); closeSlash(); }
    }, 0);
  }

  // Clicks: checkboxes, links, and (mobile) tap-to-edit.
  let suppressClick = false;
  function onClick(e: MouseEvent) {
    const t = e.target as HTMLElement;
    if (t.closest("a")) e.preventDefault();
    if (suppressClick) { suppressClick = false; e.preventDefault(); return; }
    const b = blockOf(t);
    if (!b) return;
    if (t.closest('[data-act="check"]')) { doc()!.toggleChecked(b.n); return; }
    if (isMobile && b.n.kind !== "Divider" && active() !== b.n.id) {
      setActive(b.n.id);
      b.el.focus({ preventScroll: true });
      const pos = (document as any).caretRangeFromPoint?.(e.clientX, e.clientY) as Range | null;
      if (pos && b.el.contains(pos.startContainer)) {
        const s = window.getSelection()!;
        s.removeAllRanges();
        s.addRange(pos);
      } else setSel(b.el, b.n.text.length);
    }
  }

  // Mobile: long-press a block (not the one being edited) for the sheet.
  let lp: { timer: number; x: number; y: number; id: string; fired: boolean } | null = null;
  function onTouchStart(e: TouchEvent) {
    if (!isMobile || e.touches.length !== 1) return;
    const b = blockOf(e.target);
    if (!b || (active() === b.n.id && keyboard().open)) return;
    const t = e.touches[0];
    const id = b.n.id;
    lp = {
      x: t.clientX, y: t.clientY, id, fired: false,
      timer: window.setTimeout(() => {
        if (!lp) return;
        lp.fired = true;
        suppressClick = true;
        navigator.vibrate?.(10);
        (document.activeElement as HTMLElement | null)?.blur?.();
        openSheet(id);
      }, 450),
    };
  }
  function onTouchMove(e: TouchEvent) {
    if (!lp) return;
    const t = e.touches[0];
    if (Math.hypot(t.clientX - lp.x, t.clientY - lp.y) > 10) { clearTimeout(lp.timer); lp = null; }
  }
  function onTouchEnd(e: TouchEvent) {
    if (!lp) return;
    clearTimeout(lp.timer);
    if (lp.fired) e.preventDefault();
    lp = null;
  }

  // ---------- desktop: hover handle, drag to move ----------
  function onMouseMove(e: MouseEvent) {
    if (isMobile || dragging) return;
    const t = e.target as HTMLElement;
    if (t.closest(".handle")) return;
    let row = t.closest(".blk") as HTMLElement | null;
    if (!row) {
      const br = blocksEl.getBoundingClientRect();
      if (e.clientY < br.top || e.clientY > br.bottom) { if (!blockMenu()) setHover(null); return; }
      const hit = document.elementFromPoint(br.left + 80, e.clientY) as HTMLElement | null;
      row = hit?.closest(".blk") as HTMLElement | null;
    }
    if (!row) return;
    const id = row.dataset.id!;
    const n = node(id);
    if (!n) return;
    const h = hover();
    const lh = LINE[n.kind] ?? 24;
    const top = n.kind === "Code" ? row.offsetTop + 4 : n.kind === "Divider" ? row.offsetTop - 5.5 : row.offsetTop + 2 + (lh - 24) / 2;
    const left = n.meta().depth * 24 - 44;
    if (!h || h.id !== id || h.top !== top || h.left !== left) setHover({ id, top, left });
  }

  let dragging = false;
  function onGripDown(e: PointerEvent) {
    const h = hover();
    if (!h || e.button !== 0) return;
    e.preventDefault();
    const grip = e.currentTarget as HTMLElement;
    grip.setPointerCapture(e.pointerId);
    const startY = e.clientY;
    const id = h.id;
    let target: { id: string; before: boolean } | null = null;
    let raf = 0;
    let lastY = e.clientY;
    const autoscroll = () => {
      const sr = scroller.getBoundingClientRect();
      const dy = lastY < sr.top + 60 ? -12 : lastY > sr.bottom - 60 ? 12 : 0;
      if (dy) scroller.scrollTop += dy;
      if (dragging) raf = requestAnimationFrame(autoscroll);
    };
    const move = (ev: PointerEvent) => {
      lastY = ev.clientY;
      if (!dragging && Math.abs(ev.clientY - startY) > 4) {
        dragging = true;
        renderAll();
        els.get(id)?.parentElement?.classList.add("dragging");
        setBlockMenu(null);
        raf = requestAnimationFrame(autoscroll);
      }
      if (!dragging) return;
      const br = blocksEl.getBoundingClientRect();
      const y = Math.max(br.top + 1, Math.min(ev.clientY, br.bottom - 1));
      const row = (document.elementFromPoint(br.left + 80, y) as HTMLElement | null)?.closest(".blk") as HTMLElement | null;
      if (!row) return;
      const rr = row.getBoundingClientRect();
      const before = ev.clientY < rr.top + rr.height / 2;
      const tn = node(row.dataset.id!)!;
      target = { id: tn.id, before };
      const depth = tn.meta().depth + (!before && tn.children.length ? 1 : 0);
      setDrop({ top: before ? row.offsetTop - 1 : row.offsetTop + row.offsetHeight + 1, left: depth * 24 });
    };
    const up = () => {
      grip.removeEventListener("pointermove", move);
      grip.removeEventListener("pointerup", up);
      grip.removeEventListener("pointercancel", up);
      cancelAnimationFrame(raf);
      els.get(id)?.parentElement?.classList.remove("dragging");
      setDrop(null);
      if (!dragging) {
        const r = grip.getBoundingClientRect();
        setMenuFilter("");
        setBlockMenu({ id, x: r.left, y: r.top, turn: false });
        return;
      }
      dragging = false;
      const d = doc()!;
      const n = node(id)!;
      if (!target || target.id === id || d.isInside(target.id, id)) return;
      const t = node(target.id)!;
      if (target.before) {
        const sib = d.siblings(t.parent);
        const i = sib.indexOf(t.id);
        let after = i > 0 ? sib[i - 1] : null;
        if (after === id) return;
        d.move(n, t.parent, after);
      } else if (t.children.length) d.move(n, t.id, null);
      else d.move(n, t.parent, t.id);
      setHover(null);
    };
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", up);
    grip.addEventListener("pointercancel", up);
  }

  function addBelow() {
    const h = hover();
    const d = doc();
    if (!h || !d) return;
    const n = node(h.id)!;
    const m = d.insertAfter(n, n.parent, "Paragraph", "/");
    focusBlock(m.id, 1);
    openSlash(m, "slash");
  }

  // ---------- sheet (mobile) and block menu (desktop) ----------
  function openSheet(id: string) {
    setSheet({ id, shown: false, page: "main" });
    props.onSheet(true);
    requestAnimationFrame(() => requestAnimationFrame(() => setSheet((s) => (s ? { ...s, shown: true } : s))));
  }
  /** Called by the app on back navigation; closes the sheet if open. */
  function closeSheet() {
    const s = sheet();
    if (!s) return false;
    setSheet({ ...s, shown: false });
    setTimeout(() => setSheet((x) => (x && !x.shown ? null : x)), 280);
    return true;
  }
  props.registerBack(() => {
    if (linkPop()) { setLinkPop(null); return true; }
    if (slash()) { closeSlash(); return true; }
    return closeSheet();
  });
  onCleanup(() => props.registerBack(null));
  const dismissSheet = () => { if (sheet()?.shown) history.back(); };

  function blockAction(id: string, act: "up" | "down" | "delete" | Kind, fromSheet: boolean) {
    const d = doc();
    const n = node(id);
    if (!d || !n) return;
    if (act === "up" || act === "down") {
      if (act === "up") d.moveUp(n); else d.moveDown(n);
      if (!fromSheet) setBlockMenu(null);
    }
    else if (act === "delete") {
      const prev = d.prevVisible(id);
      d.remove(n);
      if (!d.top.length) d.insertAfter(null, null, "Paragraph");
      if (fromSheet) dismissSheet(); else setBlockMenu(null);
      if (!isMobile && prev) focusBlock(prev, "end");
      return;
    } else {
      if (act === "Divider") { d.setKind(n, "Divider"); } else d.setKind(n, act);
      if (fromSheet) dismissSheet(); else { setBlockMenu(null); focusBlock(id, "end"); }
      return;
    }
    // Keep the moved block in view.
    const el = els.get(id);
    if (el) requestAnimationFrame(() => el.parentElement?.scrollIntoView({ block: "nearest", behavior: "smooth" }));
  }

  // ---------- keyboard toolbar (mobile) ----------
  const showKbBar = () => isMobile && !!active() && keyboard().open && !sheet();
  createEffect(() => { if (!showKbBar()) setFmtMode(false); });

  const tbDown = (e: Event) => e.preventDefault();

  /** Toolbar kind buttons: turn the block into `kind`, or back into text. */
  function toggleKind(kind: Kind) {
    keepCaret((n) => doc()!.setKind(n, n.kind === kind ? "Paragraph" : kind));
  }
  /** Toolbar "+": a new block below with the `/` menu open. */
  function plusBlock() {
    const id = active();
    const d = doc();
    if (!id || !d) return;
    const n = node(id)!;
    if (n.text === "" && n.kind === "Paragraph") {
      d.setText(n, "/", []);
      focusBlock(n.id, 1);
      openSlash(n, "slash");
      return;
    }
    const m = d.insertAfter(n, n.parent, "Paragraph", "/");
    focusBlock(m.id, 1);
    openSlash(m, "slash");
  }
  const activeKind = () => { const id = active(); return id ? node(id)?.meta().kind : undefined; };
  const [selTick, setSelTick] = createSignal(0);
  if (isMobile) {
    const bump = () => setSelTick((t) => t + 1);
    document.addEventListener("selectionchange", bump);
    onCleanup(() => document.removeEventListener("selectionchange", bump));
  }
  const activeHas = (key: MarkKey) => {
    selTick();
    const id = active();
    const n = id ? node(id) : undefined;
    const el = id ? els.get(id) : undefined;
    if (!n || !el) return false;
    n.meta().ver;
    const s = getSel(el);
    if (!s) return false;
    const [a, b] = s.collapsed ? wordAt(n.text, s.start) : [s.start, s.end];
    return hasMark(n.marks, key, a, b);
  };

  onCleanup(() => { setActive(null); });

  // ---------- views ----------
  function BlockView(p: { id: string }) {
    const n = node(p.id)!;
    let el!: HTMLDivElement;
    const m = n.meta;
    createEffect(on(() => m().ver, () => { if (el) renderInline(el, n.text, n.marks); }, { defer: true }));
    onCleanup(() => { if (els.get(p.id) === el) els.delete(p.id); });
    const cls = () => {
      const x = m();
      let c = "blk " + KCLASS[x.kind] + " d" + (x.depth % 3);
      if (x.checked && x.kind === "Todo") c += " done";
      if (isActive(p.id)) c += " focus";
      if (sheet()?.id === p.id || blockMenu()?.id === p.id) c += " sheet-target";
      return c;
    };
    return (
      <div class={cls()} data-id={p.id} style={{ "--depth": m().depth }}>
        <Switch>
          <Match when={m().kind === "Bulleted"}><div class="pre" /></Match>
          <Match when={m().kind === "Numbered"}><div class="pre">{numLabel(m().num, m().depth)}.</div></Match>
          <Match when={m().kind === "Todo"}><div class="pre" data-act="check" role="checkbox" aria-checked={m().checked}><span class="check"><ICheck /></span></div></Match>
          <Match when={m().kind === "Divider"}><div class="hr" /></Match>
          <Match when={m().kind === "Code"}>
            <span class="lang" style={{ left: `${m().depth * 24 + 16}px` }}>Plain text</span>
            <Show when={!isMobile}>
              <button class="copy" onMouseDown={(e) => e.preventDefault()}
                onClick={() => navigator.clipboard?.writeText(n.text).catch(() => {})}>Copy</button>
            </Show>
          </Match>
        </Switch>
        <div
          class="txt"
          ref={(e) => { el = e; els.set(p.id, e); renderInline(e, n.text, n.marks); }}
          contentEditable={m().kind !== "Divider" && (!isMobile || isActive(p.id))}
          data-ph={PH[m().kind]}
          spellcheck={m().kind !== "Code"}
        />
      </div>
    );
  }

  const sheetNode = () => { const s = sheet(); return s ? node(s.id) : undefined; };
  const selHas = (key: MarkKey) => {
    const s = selMenu();
    if (!s) return false;
    const n = node(s.id);
    return !!n && hasMark(n.marks, key, s.start, s.end);
  };
  const editedLabel = () => { const e = edited(); return e ? `Edited ${relTime(e).replace("Just now", "just now")}` : ""; };

  interface MenuAct { name: string; icon: () => JSX.Element; run: () => void; disabled?: () => boolean; sub?: boolean; kind?: Kind }
  const blockActs = (id: string): MenuAct[] => {
    const n = () => node(id);
    return [
      { name: "Turn into", icon: () => <ITurn size={18} />, sub: true, run: () => setBlockMenu((b) => (b ? { ...b, turn: true } : b)) },
      { name: "Move up", icon: () => <IUp size={18} />, disabled: () => !n() || !doc()!.canMoveUp(n()!), run: () => blockAction(id, "up", false) },
      { name: "Move down", icon: () => <IDown size={18} />, disabled: () => !n() || !doc()!.canMoveDown(n()!), run: () => blockAction(id, "down", false) },
      { name: "Delete", icon: () => <ITrash size={18} />, run: () => blockAction(id, "delete", false) },
    ];
  };
  const menuItems = (id: string, turn: boolean): MenuAct[] => {
    const f = menuFilter().trim().toLowerCase();
    const kinds: MenuAct[] = KINDS.map((k) => ({ name: k.name, icon: k.icon, kind: k.kind, run: () => blockAction(id, k.kind, false) }));
    if (turn) return kinds.filter((k) => !f || k.name.toLowerCase().includes(f));
    if (!f) return blockActs(id);
    return [...blockActs(id).filter((a) => !a.sub), ...kinds].filter((a) => a.name.toLowerCase().includes(f));
  };

  return (
    <div class="screen editor" data-testid="editor" onKeyDown={(e) => { if (e.key === "Escape" && blockMenu()) setBlockMenu(null); }}>
      <div class={"topbar" + (lined() ? " lined" : "")}>
        <Show when={isMobile} fallback={
          <>
            <button class="tb-btn" data-testid="back" onClick={() => props.onBack()} aria-label="Back" title="Back"><IArrowLeft size={18} /></button>
            <span class="tb-btn crumb"><IPage size={16} /><span class="t">{title() || "Untitled"}</span></span>
            <span class="spacer" />
            <span class="tb-meta" data-testid="save-state" data-state={saveState()}>{saveState() === "Saving…" ? "Saving…" : editedLabel()}</span>
          </>
        }>
          <button class="tb-btn m-back" data-testid="back" onClick={() => props.onBack()} aria-label="Back"><IBack size={24} /></button>
          <span class="tb-btn crumb m-crumb"><IPage size={18} /><span class="t">{title() || "Untitled"}</span></span>
          <span class="spacer" />
          <span data-testid="save-state" data-state={saveState()} style={{ display: "none" }} />
        </Show>
      </div>
      <div class="scroller" ref={scroller} style={{ "padding-bottom": showKbBar() ? "44px" : undefined }}
        onScroll={() => { const l = scroller.scrollTop > 0; if (l !== lined()) setLined(l); }}
        onMouseLeave={() => { if (!dragging && !blockMenu()) setHover(null); }}>
        <div class="col page-top" onMouseMove={onMouseMove}>
          <div
            class="page-title"
            ref={titleEl}
            contentEditable="plaintext-only"
            spellcheck={false}
            data-testid="title"
            onInput={onTitleInput}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === "ArrowDown") {
                e.preventDefault();
                const d = doc();
                if (!d) return;
                if (e.key === "Enter" && !d.top.length) d.insertAfter(null, null, "Paragraph");
                const first = d.order().find((id) => node(id)!.kind !== "Divider");
                if (first) focusBlock(first, 0);
              }
            }}
            onBeforeInput={(e) => { if (e.inputType === "insertParagraph" || e.inputType === "insertLineBreak") e.preventDefault(); }}
          />
          <div
            class="blocks"
            ref={blocksEl}
            onBeforeInput={onBeforeInput}
            onInput={onInput}
            onKeyDown={onKeyDown}
            onFocusIn={onFocusIn}
            onFocusOut={onFocusOut}
            onClick={onClick}
            onPaste={onPaste}
            onCompositionEnd={onCompositionEnd}
            onTouchStart={onTouchStart}
            onTouchMove={onTouchMove}
            onTouchEnd={onTouchEnd}
            onTouchCancel={onTouchEnd}
            onContextMenu={(e) => { if (isMobile) e.preventDefault(); }}
            onDragStart={(e) => e.preventDefault()}
          >
            <For each={shown()}>{(id) => <BlockView id={id} />}</For>
            <Show when={!isMobile}>
              <div class={"handle" + (hover() ? " on" : "")} style={{ top: `${hover()?.top ?? 0}px`, left: `${hover()?.left ?? 0}px` }}>
                <button onClick={addBelow} title="Click to add below" aria-label="Add block below"><IPlus size={16} /></button>
                <button class="grip" data-testid="grip" onPointerDown={onGripDown} title="Drag to move, click to open menu" aria-label="Block menu"><IGrip /></button>
              </div>
              <Show when={drop()}>
                <div class="dropline" style={{ top: `${drop()!.top}px`, left: `${drop()!.left}px` }} />
              </Show>
            </Show>
          </div>
          <div class="page-end" onClick={() => {
            const d = doc();
            if (!d) return;
            const o = d.order();
            const last = o.length ? node(o[o.length - 1])! : null;
            if (last && last.kind === "Paragraph" && !last.text) focusBlock(last.id, 0);
            else { const n = d.insertAfter(last && !last.parent ? last : d.top.length ? node(d.top[d.top.length - 1])! : null, null, "Paragraph"); focusBlock(n.id, 0); }
          }} />
        </div>
      </div>

      {/* The `/` menu (and "Turn into" from the toolbars) */}
      <Show when={slash()}>
        {(s) => (
          <div class="menu slash" data-testid="slash-menu" style={{ left: `${s().x}px`, ...(s().above ? { bottom: `${window.innerHeight - s().y}px` } : { top: `${s().y}px` }) }}
            onMouseDown={(e) => e.preventDefault()} onPointerDown={(e) => e.preventDefault()}>
            <div class="label">{s().mode === "turn" ? "Turn into" : "Basic blocks"}</div>
            <For each={slashItems()} fallback={<div class="none">No results</div>}>
              {(it, i) => (
                <button class={"item" + (i() === s().index ? " sel" : "")} onMouseEnter={() => !isMobile && setSlash({ ...s(), index: i() })}
                  onClick={() => pickKind(it.kind)}>
                  <span class="ic">{it.icon()}</span>
                  <span class="nm">{it.name}</span>
                  <Show when={s().mode === "turn" && node(s().id)?.kind === it.kind} fallback={<span class="hint">{it.hint}</span>}>
                    <span class="hint" style={{ color: "var(--accent)" }}><ICheck size={14} /></span>
                  </Show>
                </button>
              )}
            </For>
          </div>
        )}
      </Show>

      {/* Desktop: toolbar over a text selection */}
      <Show when={selMenu()}>
        {(s) => (
          <div class="menu selbar" data-testid="selection-toolbar" style={{ left: `${s().x}px`, top: `${s().y}px` }} onMouseDown={(e) => e.preventDefault()}>
            <button class="txtbtn" title="Turn into" onClick={() => { const n = node(s().id); if (n) { setSelMenu(null); openSlash(n, "turn"); } }}>
              {kindName(node(s().id)?.meta().kind)}<IChevDown size={12} />
            </button>
            <span class="sep" />
            <button class={selHas("a") ? "on" : ""} title="Link (Ctrl+K)" aria-label="Link" onClick={() => format("a")}><ILink size={16} /></button>
            <span class="sep" />
            <button class={selHas("b") ? "on" : ""} title="Bold (Ctrl+B)" aria-label="Bold" onClick={() => format("b")}><span class="glyph-b">B</span></button>
            <button class={selHas("i") ? "on" : ""} title="Italic (Ctrl+I)" aria-label="Italic" onClick={() => format("i")}><span class="glyph-i">i</span></button>
            <button class={selHas("s") ? "on" : ""} title="Strikethrough (Ctrl+Shift+S)" aria-label="Strikethrough" onClick={() => format("s")}><span class="glyph-s">S</span></button>
            <button class={selHas("c") ? "on" : ""} title="Inline code (Ctrl+E)" aria-label="Inline code" onClick={() => format("c")}><ICode size={16} /></button>
          </div>
        )}
      </Show>

      {/* Link editor */}
      <Show when={linkPop()}>
        {(p) => (
          <div class="menu linkpop" data-testid="link-popover" style={{ left: `${p().x}px`, top: `${p().y}px` }}>
            <input
              ref={(el) => setTimeout(() => el.focus())}
              value={p().href}
              placeholder="Paste link"
              onKeyDown={(e) => {
                if (e.key === "Enter") { e.preventDefault(); applyLink(e.currentTarget.value); }
                if (e.key === "Escape") { e.preventDefault(); setLinkPop(null); focusBlock(p().id, p().start, p().end); }
              }}
            />
            <button class="btn primary" onMouseDown={(e) => e.preventDefault()} onClick={(e) => applyLink((e.currentTarget.previousElementSibling as HTMLInputElement).value)}>
              {p().href ? "Update" : "Link"}
            </button>
            <Show when={p().href}>
              <button class="btn ghost" onMouseDown={(e) => e.preventDefault()} onClick={() => applyLink("")}>Remove</button>
            </Show>
          </div>
        )}
      </Show>

      {/* Desktop: block menu from the drag handle */}
      <Show when={blockMenu()}>
        {(bm) => (
          <>
            <div style={{ position: "fixed", inset: "0", "z-index": "19" }} onMouseDown={() => { setBlockMenu(null); setHover(null); }} />
            <div class="menu blockmenu" data-testid="block-menu" style={{ left: `${Math.max(8, bm().x - 265 - 4)}px`, top: `${Math.max(8, Math.min(bm().y - 30, window.innerHeight - (bm().turn ? 420 : 200)))}px` }}>
              <label class="msearch">
                <input ref={(el) => setTimeout(() => el.focus())} placeholder={bm().turn ? "Turn into…" : "Search actions…"} value={menuFilter()}
                  onInput={(e) => setMenuFilter(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") { e.preventDefault(); setBlockMenu(null); }
                    if (e.key === "Enter") { const it = menuItems(bm().id, bm().turn).find((x) => !x.disabled?.()); it?.run(); }
                  }} />
              </label>
              <Show when={bm().turn}><div class="label">Turn into</div></Show>
              <For each={menuItems(bm().id, bm().turn)} fallback={<div class="none">No results</div>}>
                {(a) => (
                  <>
                    <Show when={a.name === "Delete" && !menuFilter()}><div class="sepl" /></Show>
                    <button class="mi" disabled={a.disabled?.()} onClick={() => { setMenuFilter(""); a.run(); }}>
                      <span class="ic">{a.icon()}</span>{a.name}
                      <Show when={a.sub}><span class="chev"><IChevR size={16} /></span></Show>
                      <Show when={a.kind && bm().turn && node(bm().id)?.kind === a.kind}><span class="chev" style={{ color: "var(--accent)" }}><ICheck size={14} /></span></Show>
                    </button>
                  </>
                )}
              </For>
            </div>
          </>
        )}
      </Show>

      {/* Mobile: toolbar pinned above the keyboard */}
      <Show when={showKbBar()}>
        <div class="kbbar" data-testid="keyboard-toolbar" style={{ transform: `translateY(${keyboard().bottom - 44}px)` }} onPointerDown={tbDown} onMouseDown={tbDown}>
          <div class="scroll">
            <Show when={!fmtMode()} fallback={
              <>
                <button class="aa on" aria-label="Text styles" onClick={() => setFmtMode(false)}>Aa</button>
                <button class={activeHas("b") ? "on" : ""} aria-label="Bold" onClick={() => format("b")}><span class="glyph-b" style={{ "font-size": "17px" }}>B</span></button>
                <button class={activeHas("i") ? "on" : ""} aria-label="Italic" onClick={() => format("i")}><span class="glyph-i" style={{ "font-size": "19px" }}>i</span></button>
                <button class={activeHas("s") ? "on" : ""} aria-label="Strikethrough" onClick={() => format("s")}><span class="glyph-s" style={{ "font-size": "17px" }}>S</span></button>
                <button class={activeHas("c") ? "on" : ""} aria-label="Inline code" onClick={() => format("c")}><ICode /></button>
                <button class={activeHas("a") ? "on" : ""} aria-label="Link" onClick={() => format("a")}><ILink /></button>
              </>
            }>
              <button aria-label="Add block" onClick={plusBlock}><IPlus /></button>
              <button class="aa" aria-label="Text styles" onClick={() => setFmtMode(true)}>Aa</button>
              <button aria-label="Turn into" onClick={() => { const n = node(active()!); if (n) (slash()?.mode === "turn" ? closeSlash() : openSlash(n, "turn")); }}><ITurn /></button>
              <button class={activeKind() === "Todo" ? "on" : ""} aria-label="To-do" onClick={() => toggleKind("Todo")}><ITodo /></button>
              <button class={activeKind() === "Bulleted" ? "on" : ""} aria-label="Bulleted list" onClick={() => toggleKind("Bulleted")}><IBullet /></button>
              <button aria-label="Indent" onClick={() => keepCaret((n) => doc()!.indent(n))}><IIndent /></button>
              <button aria-label="Outdent" onClick={() => keepCaret((n) => doc()!.outdent(n))}><IOutdent /></button>
              <button aria-label="Block actions" onClick={() => { const id = active(); if (id) { (document.activeElement as HTMLElement)?.blur(); openSheet(id); } }}><IMore /></button>
            </Show>
          </div>
          <button class="done" aria-label="Hide keyboard" onClick={() => (document.activeElement as HTMLElement)?.blur()}><IKbHide /></button>
        </div>
      </Show>

      {/* Mobile: bottom sheet */}
      <Show when={sheet()}>
        {(s) => (
          <>
            <div class={"scrim" + (s().shown ? " on" : "")} onClick={dismissSheet} />
            <div class={"sheet" + (s().shown ? " on" : "")} role="dialog" aria-label="Block actions" data-testid="block-sheet">
              <div class="grab" />
              <Show when={s().page === "main"} fallback={
                <>
                  <button class="sheet-title" style={{ width: "100%" }} onClick={() => setSheet({ ...s(), page: "main" })}><IBack size={18} />Turn into</button>
                  <For each={KINDS}>{(k) => (
                    <button class="mi" onClick={() => blockAction(s().id, k.kind, true)}>
                      <span class="ic">{k.icon()}</span>{k.name}
                      <Show when={sheetNode()?.kind === k.kind}><span class="tick"><ICheck size={16} /></span></Show>
                    </button>
                  )}</For>
                </>
              }>
                <div class="preview">{sheetNode()?.text || kindName(sheetNode()?.kind)}</div>
                <button class="mi" onClick={() => setSheet({ ...s(), page: "turn" })}><span class="ic"><ITurn size={22} /></span>Turn into<span class="chev"><IChevR size={18} /></span></button>
                <button class="mi" disabled={!sheetNode() || !doc()!.canMoveUp(sheetNode()!)} onClick={() => blockAction(s().id, "up", true)}><span class="ic"><IUp size={22} /></span>Move up</button>
                <button class="mi" disabled={!sheetNode() || !doc()!.canMoveDown(sheetNode()!)} onClick={() => blockAction(s().id, "down", true)}><span class="ic"><IDown size={22} /></span>Move down</button>
                <div class="sepl" />
                <button class="mi" onClick={() => blockAction(s().id, "delete", true)}><span class="ic"><ITrash size={22} /></span>Delete</button>
              </Show>
            </div>
          </>
        )}
      </Show>

      <Show when={toast()}>
        <div class="toast" onClick={() => setToast("")} ref={() => setTimeout(() => setToast(""), 4000)}>{toast()}</div>
      </Show>
    </div>
  );
}
