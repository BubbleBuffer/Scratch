// Typed wrappers around the Tauri commands in src-tauri/src/lib.rs. Outside
// Tauri (headless tests in Chromium) the same commands go over HTTP to the
// test bridge (src-tauri/examples/bridge.rs), which serves this build.
import { invoke as tauriInvoke } from "@tauri-apps/api/core";

const inTauri = "__TAURI_INTERNALS__" in window;

async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (inTauri) return tauriInvoke<T>(cmd, args);
  const r = await fetch(`/invoke/${cmd}`, { method: "POST", body: JSON.stringify(args) });
  const v = await r.json();
  if (!r.ok) throw v;
  return v as T;
}

export type Kind =
  | "Paragraph" | "Heading1" | "Heading2" | "Heading3"
  | "Bulleted" | "Numbered" | "Todo" | "Quote" | "Code" | "Divider";

/** Serde's external tagging: "Bold" or {"Link": "https://…"}. */
export type ApiMarkKind = "Bold" | "Italic" | "Strike" | "Code" | { Link: string };
/** Offsets in Unicode scalar values (code points), as the core wants them. */
export interface ApiMark { start: number; end: number; kind: ApiMarkKind }

export interface ApiBlock {
  id: string; kind: Kind; text: string; marks: ApiMark[]; checked: boolean; children: ApiBlock[];
}
export interface ApiPage { id: string; title: string; blocks: ApiBlock[] }
export interface PageSummary { id: string; title: string; parent: string | null; updated_ms: number; child_count: number }
export interface SearchHit { id: string; title: string; snippet: string }
export interface NewBlock { id: string; kind: Kind; text: string; marks: ApiMark[]; checked: boolean }

export type BlockOp =
  | { op: "Insert"; parent: string | null; after: string | null; block: NewBlock }
  | { op: "Delete"; id: string }
  | { op: "Move"; id: string; parent: string | null; after: string | null }
  | { op: "SetText"; id: string; text: string; marks: ApiMark[] }
  | { op: "SetKind"; id: string; kind: Kind }
  | { op: "SetChecked"; id: string; checked: boolean };

export const api = {
  vaultStatus: () => invoke<{ needs_seed: boolean; path: string }>("vault_status"),
  openVault: () => invoke<{ count: number; seeded: boolean; seed_ms: number; open_ms: number }>("open_vault"),
  pageCount: () => invoke<number>("page_count"),
  listPages: (offset: number, limit: number) => invoke<PageSummary[]>("list_pages", { offset, limit }),
  search: (query: string, limit: number) => invoke<SearchHit[]>("search", { query, limit }),
  createPage: (title: string) => invoke<string>("create_page", { title }),
  renamePage: (id: string, title: string) => invoke<void>("rename_page", { id, title }),
  loadPage: (id: string) => invoke<ApiPage>("load_page", { id }),
  apply: (id: string, ops: BlockOp[]) => invoke<void>("apply", { id, ops }),
  flush: () => invoke<void>("flush"),
  log: (line: string) => invoke<void>("bakeoff_log", { line }),
};
