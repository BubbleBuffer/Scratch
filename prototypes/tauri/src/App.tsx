import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { api, type PageSummary } from "./api";
import { blog } from "./bakeoff";
import { Editor } from "./editor/Editor";
import { saves } from "./editor/model";
import { Home } from "./Home";
import { IHome, IPage, IPlus, ISearch } from "./icons";
import { isMobile, keyboard, keyboardSim } from "./platform";

type Phase = { name: "boot" } | { name: "seeding"; started: number } | { name: "ready" } | { name: "error"; msg: string };
export interface Route { id: string; t0: number; key: number; updated?: number }

export function App() {
  const [phase, setPhase] = createSignal<Phase>({ name: "boot" });
  const [count, setCount] = createSignal(0);
  const [version, setVersion] = createSignal(0);
  const [route, setRoute] = createSignal<Route | null>(null);
  const [tick, setTick] = createSignal(0);
  const [sidebarPages, setSidebarPages] = createSignal<PageSummary[]>([]);
  const [liveTitle, setLiveTitle] = createSignal<{ id: string; title: string } | null>(null);
  let backHandler: (() => boolean) | null = null;
  let sheetOpen = false;
  let focusSearch: (() => void) | null = null;

  const loadSidebar = () => { if (!isMobile) api.listPages(0, 30).then(setSidebarPages).catch(() => {}); };

  onMount(async () => {
    try {
      const status = await api.vaultStatus();
      if (status.needs_seed) setPhase({ name: "seeding", started: performance.now() });
      const timer = setInterval(() => setTick((t) => t + 1), 250);
      const info = await api.openVault();
      clearInterval(timer);
      if (info.seeded) blog(`seed ms=${info.seed_ms} pages=${info.count}`);
      blog(`vault_open ms=${info.open_ms.toFixed(1)} pages=${info.count} platform=${isMobile ? "mobile" : "desktop"}`);
      setCount(info.count);
      history.replaceState({ list: 1 }, "");
      setPhase({ name: "ready" });
      loadSidebar();
    } catch (e) {
      setPhase({ name: "error", msg: String(e) });
    }
  });

  const refreshList = async () => {
    await saves.last.catch(() => {});
    await api.flush().catch(() => {});
    setCount(await api.pageCount());
    setVersion((v) => v + 1);
    loadSidebar();
  };

  const openPage = (id: string, t0: number, s?: PageSummary) => {
    if (route()?.id === id) return;
    if (route()) history.replaceState({ page: id }, "");
    else history.pushState({ page: id }, "");
    setRoute({ id, t0, key: Date.now(), updated: s?.updated_ms });
  };

  const newPage = async (t0: number) => {
    const id = await api.createPage("");
    openPage(id, t0, { id, title: "", parent: null, updated_ms: Date.now(), child_count: 0 });
    setCount((c) => c + 1);
    loadSidebar();
  };

  const goHome = () => { if (route()) history.back(); };

  // Android back gesture, the page's back button and Alt+Left all go through
  // history, so one popstate handler returns to the list.
  const onPop = () => {
    if (sheetOpen) { sheetOpen = false; backHandler?.(); return; }
    if (route() && backHandler?.()) { history.pushState({ page: route()!.id }, ""); return; }
    if (route()) { setRoute(null); setLiveTitle(null); refreshList(); }
  };
  const onKey = (e: KeyboardEvent) => { if (e.altKey && e.key === "ArrowLeft" && route()) history.back(); };
  const onVis = () => { if (document.visibilityState === "hidden") api.flush().catch(() => {}); };
  window.addEventListener("popstate", onPop);
  window.addEventListener("keydown", onKey);
  document.addEventListener("visibilitychange", onVis);
  onCleanup(() => {
    window.removeEventListener("popstate", onPop);
    window.removeEventListener("keydown", onKey);
    document.removeEventListener("visibilitychange", onVis);
  });

  const progress = () => {
    tick();
    const p = phase();
    if (p.name !== "seeding") return 0;
    // Seeding reports no progress; ease toward the expected duration.
    const expected = isMobile ? 45000 : 18000;
    const t = (performance.now() - p.started) / expected;
    return 1 - Math.exp(-2.2 * t);
  };

  const sbTitle = (p: PageSummary) => { const l = liveTitle(); return (l && l.id === p.id ? l.title : p.title) || "Untitled"; };

  const Sidebar = () => (
    <nav class="sidebar" aria-label="Sidebar">
      <div class="sb-top">
        <div class="sb-row ws"><span class="avatar">N</span><span class="t">Notes</span></div>
        <button class="sb-row" onClick={() => { goHome(); setTimeout(() => focusSearch?.(), 0); }}>
          <span class="ic"><ISearch size={18} /></span><span class="t">Search</span>
        </button>
        <button class={"sb-row" + (route() ? "" : " cur")} onClick={goHome}>
          <span class="ic"><IHome size={18} /></span><span class="t">Home</span>
        </button>
        <button class="sb-row" onClick={(e) => newPage(e.timeStamp)}>
          <span class="ic"><IPlus size={18} /></span><span class="t">New page</span>
        </button>
      </div>
      <div class="sb-scroll">
        <div class="sb-label">Private</div>
        <For each={sidebarPages()}>
          {(p) => (
            <button class={"sb-row" + (route()?.id === p.id ? " cur" : "")} onClick={(e) => openPage(p.id, e.timeStamp, p)}>
              <span class="ic"><IPage size={18} /></span><span class="t">{sbTitle(p)}</span>
            </button>
          )}
        </For>
      </div>
    </nav>
  );

  return (
    <Show when={phase().name === "ready"} fallback={
      <div class="screen seed">
        <Show when={phase().name === "seeding"}>
          <h1>Preparing your test vault</h1>
          <p>Creating 10,000 pages and one 2,000-block page. This happens once.</p>
          <div class="bar"><div style={{ transform: `scaleX(${progress()})` }} /></div>
          <p>{(tick(), Math.round((performance.now() - ((phase() as any).started ?? 0)) / 1000))} s</p>
        </Show>
        <Show when={phase().name === "error"}>
          <h1>Could not open the vault</h1>
          <p class="err">{(phase() as any).msg}</p>
        </Show>
      </div>
    }>
      <div class="app">
        <Show when={!isMobile}><Sidebar /></Show>
        <main class="main">
          <Home count={count()} version={version()} hidden={!!route()} onOpen={openPage} onNew={newPage}
            registerSearch={(f) => { focusSearch = f; }} />
          <Show when={route()} keyed>
            {(r) => (
              <Editor
                route={r}
                onBack={() => history.back()}
                onTitle={(title) => setLiveTitle({ id: r.id, title })}
                onSheet={(open) => { if (open) { sheetOpen = true; history.pushState({ sheet: 1 }, ""); } }}
                registerBack={(fn) => { backHandler = fn; }}
              />
            )}
          </Show>
        </main>
      </div>
      <Show when={keyboardSim() > 0 && keyboard().open}>
        <div class="kb-sim" style={{ height: `${keyboardSim()}px` }} />
      </Show>
    </Show>
  );
}
