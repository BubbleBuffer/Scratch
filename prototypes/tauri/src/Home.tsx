// Home: all pages in one virtualized list (sorted by last edited), a search
// field that filters through `vault.search`, recent-page cards, a new-page button.
import { For, Index, Show, createMemo, createSignal, onMount, type Accessor, type JSX } from "solid-js";
import { api, type PageSummary, type SearchHit } from "./api";
import { afterPaint, blog, listFrames } from "./bakeoff";
import { IClock, ICompose, IHome, IPage, IPlus, ISearch, IX } from "./icons";
import { isMobile } from "./platform";

const CHUNK = 100;
const OVERSCAN = 8;
const SEARCH_LIMIT = 100;
const ROW_H = isMobile ? 44 : 36;
const HIT_H = isMobile ? 56 : 48;

const monthDay = new Intl.DateTimeFormat("en", { month: "short", day: "numeric" });
const monthDayYear = new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric" });
export function relTime(ms: number, now = Date.now()) {
  const s = (now - ms) / 1000;
  if (s < 60) return "Just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  if (s < 2 * 86400) return "Yesterday";
  if (s < 7 * 86400) return `${Math.floor(s / 86400)}d ago`;
  const d = new Date(ms);
  return d.getFullYear() === new Date(now).getFullYear() ? monthDay.format(d) : monthDayYear.format(d);
}

/** A windowed list inside a scroller that may have other content (`top`)
 *  above the rows: a fixed pool of row elements, each showing the index
 *  `start + ((k - start) mod pool)`, so scrolling by one row updates one row. */
function VList(p: {
  count: number; rowH: number; top?: JSX.Element;
  row: (index: Accessor<number>) => JSX.Element;
  onRange: (a: number, b: number) => void;
  ref?: (el: HTMLDivElement) => void;
}) {
  let el!: HTMLDivElement;
  let rowsEl!: HTMLDivElement;
  const [scrollTop, setScrollTop] = createSignal(0);
  const [rowsTop, setRowsTop] = createSignal(0);
  const [viewH, setViewH] = createSignal(window.innerHeight);
  const pool = createMemo(() => Math.min(p.count, Math.ceil(viewH() / p.rowH) + OVERSCAN * 2));
  const start = createMemo(() =>
    Math.max(0, Math.min(p.count - pool(), Math.floor((scrollTop() - rowsTop()) / p.rowH) - OVERSCAN)));
  const slots = createMemo(() => Array.from({ length: pool() }, (_, k) => k));
  createMemo(() => p.onRange(start(), start() + pool()));
  onMount(() => {
    const measure = () => { setViewH(el.clientHeight || window.innerHeight); setRowsTop(rowsEl.offsetTop); };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    if (rowsEl.previousElementSibling) ro.observe(rowsEl.previousElementSibling);
    p.ref?.(el);
  });
  return (
    <div class="vlist" ref={el} onScroll={() => setScrollTop(el.scrollTop)}>
      <div class="col">
        <div>{p.top}</div>
        <div class="vrows" ref={rowsEl} style={{ height: `${p.count * p.rowH + 24}px` }}>
          <Index each={slots()}>
            {(k) => {
              const idx = createMemo(() => { const s = start(), n = pool(); return s + ((((k() - s) % n) + n) % n); });
              return p.row(idx);
            }}
          </Index>
        </div>
      </div>
    </div>
  );
}

export function Home(props: {
  count: number;
  version: number;
  hidden: boolean;
  onOpen: (id: string, t0: number, s?: PageSummary) => void;
  onNew: (t0: number) => void;
  registerSearch?: (focus: () => void) => void;
}) {
  const cache = new Map<number, PageSummary>();
  const loading = new Set<number>();
  const [ver, setVer] = createSignal(0);
  const [q, setQ] = createSignal("");
  const [hits, setHits] = createSignal<SearchHit[]>([]);
  let range: [number, number] = [0, 40];
  let loadedVersion = props.version;
  let firstShown = false;
  let now = Date.now();
  let inputEl!: HTMLInputElement;
  let listEl: HTMLDivElement | undefined;

  function ensure(a: number, b: number) {
    if (loadedVersion !== props.version) {
      loadedVersion = props.version;
      cache.clear();
      loading.clear();
      now = Date.now();
    }
    for (let c = Math.floor(a / CHUNK); c <= Math.floor(Math.max(a, b - 1) / CHUNK); c++) {
      if (loading.has(c)) continue;
      loading.add(c);
      const v = loadedVersion;
      api.listPages(c * CHUNK, CHUNK).then((rows) => {
        if (v !== loadedVersion) return;
        rows.forEach((r, i) => cache.set(c * CHUNK + i, r));
        setVer((x) => x + 1);
        if (!firstShown) {
          firstShown = true;
          afterPaint((t) => blog(`list_visible ms=${t.toFixed(1)} count=${props.count}`));
        }
      }).catch(() => loading.delete(c));
    }
  }

  // Reload the visible rows (and the recent cards) when the data changed.
  createMemo(() => { props.version; ensure(0, 8); ensure(range[0], range[1]); });

  // One search in flight at a time; typing faster than the index answers
  // skips the stale queries instead of queueing them.
  let running = false;
  let wanted = "";
  function onSearch(v: string) {
    setQ(v);
    wanted = v;
    if (!v.trim()) { setHits([]); return; }
    if (!running) runSearch();
  }
  function runSearch() {
    const v = wanted;
    running = true;
    api.search(v, SEARCH_LIMIT).then((h) => { if (v === wanted) setHits(h); }).finally(() => {
      running = false;
      if (wanted !== v && wanted.trim()) runSearch();
    });
  }
  props.registerSearch?.(() => { listEl?.scrollTo({ top: 0 }); inputEl.focus(); });

  const Row = (idx: Accessor<number>) => {
    const data = () => { ver(); return cache.get(idx()); };
    return (
      <div class="row" data-testid="page-row" style={{ transform: `translateY(${idx() * ROW_H}px)` }}
        onClick={(e) => { const d = data(); if (d) props.onOpen(d.id, e.timeStamp, d); }}>
        <span class="ic"><IPage size={isMobile ? 22 : 20} /></span>
        <Show when={data()} fallback={<span class="skel" />}>
          {(d) => (
            <>
              <span class={"t" + (d().title ? "" : " untitled")}>{d().title || "Untitled"}</span>
              <span class="time">{relTime(d().updated_ms, now)}</span>
            </>
          )}
        </Show>
      </div>
    );
  };

  const HitRow = (idx: Accessor<number>) => {
    const h = () => hits()[idx()];
    return (
      <div class="row hit" data-testid="search-row" style={{ transform: `translateY(${idx() * HIT_H}px)`, height: `${HIT_H}px` }}
        onClick={(e) => { const x = h(); if (x) props.onOpen(x.id, e.timeStamp); }}>
        <span class="ic"><IPage size={isMobile ? 22 : 20} /></span>
        <div class="col2">
          <span class={"t" + (h()?.title ? "" : " untitled")}>{h()?.title || "Untitled"}</span>
          <span class="snip">{h()?.snippet}</span>
        </div>
      </div>
    );
  };

  const searchBox = (
    <label class="searchbox">
      <ISearch size={isMobile ? 20 : 16} />
      <input ref={inputEl} type="text" placeholder={isMobile ? "Search" : "Search pages…"} value={q()} spellcheck={false}
        autocomplete="off" enterkeyhint="search" data-testid="search" onInput={(e) => onSearch(e.currentTarget.value)}
        onKeyDown={(e) => { if (e.key === "Escape") { onSearch(""); inputEl.blur(); } }} />
      <Show when={q()}>
        <button class="clear" aria-label="Clear search" onMouseDown={(e) => e.preventDefault()} onClick={() => onSearch("")}><IX size={14} /></button>
      </Show>
    </label>
  );

  const recent = () => { ver(); return Array.from({ length: 8 }, (_, i) => cache.get(i)).filter(Boolean) as PageSummary[]; };
  const searching = () => !!q().trim();

  const cards = (
    <Show when={!searching()}>
      <div class="sec-label">{isMobile ? "Recents" : <><IClock />Recently edited</>}</div>
      <div class="cards">
        <For each={recent()}>
          {(s) => (
            <button class="card" onClick={(e) => props.onOpen(s.id, e.timeStamp, s)}>
              <div class="cover" />
              <span class="cic"><IPage size={isMobile ? 26 : 28} /></span>
              <div class="ct">{s.title || "Untitled"}</div>
              <div class="cm">{relTime(s.updated_ms, now)}</div>
            </button>
          )}
        </For>
      </div>
    </Show>
  );

  const listLabel = () => searching()
    ? <div class="sec-label">{hits().length === SEARCH_LIMIT ? `${SEARCH_LIMIT}+` : hits().length} results</div>
    : <div class="sec-label">{isMobile ? "Private" : "All pages"}</div>;

  const header = isMobile ? (
    <div class="m-head">
      <h1 class="m-title">Notes</h1>
      {cards}
      {listLabel()}
    </div>
  ) : (
    <div class="home-head">
      <h1 class="home-title">Pages</h1>
      <div class="home-sub">{props.count.toLocaleString("en")} pages, sorted by last edited</div>
      <div class="home-tools">
        {searchBox}
        <button class="btn-primary" data-testid="new-page" onClick={(e) => props.onNew(e.timeStamp)}><IPlus size={16} />New page</button>
      </div>
      {cards}
      {listLabel()}
    </div>
  );

  return (
    <div class={"screen home" + (props.hidden ? " hidden" : "")} data-testid="home">
      <Show when={!isMobile}>
        <div class="topbar">
          <span class="tb-btn crumb"><IHome size={16} /><span class="t">Pages</span></span>
        </div>
      </Show>
      <VList count={searching() ? hits().length : props.count} rowH={searching() ? HIT_H : ROW_H}
        top={<>{header}<Show when={searching() && !hits().length}><div class="empty-note">No pages match “{q()}”</div></Show></>}
        row={(idx) => <Show when={searching()} fallback={Row(idx)}>{HitRow(idx)}</Show>}
        onRange={(a, b) => { if (!searching()) { range = [a, b]; ensure(a, b); } }}
        ref={(el) => { listEl = el; listFrames.attach(el); }} />
      <Show when={isMobile}>
        <div class="bottombar">
          {searchBox}
          <button class="iconbtn" data-testid="new-page" aria-label="New page" onClick={(e) => props.onNew(e.timeStamp)}><ICompose size={24} /></button>
        </div>
      </Show>
    </div>
  );
}
