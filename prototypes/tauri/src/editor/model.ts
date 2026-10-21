// The editor's in-memory page: a block tree mirrored from the core, a flat
// render order, and a save queue that sends every change to `vault.apply`.
import { createSignal, type Accessor, type Setter } from "solid-js";
import { api, type ApiBlock, type ApiPage, type BlockOp, type Kind } from "../api";
import { fromApiMarks, normalize, shiftMarks, sliceMarks, toApiMarks, type Mark } from "./inline";

export interface Meta { kind: Kind; checked: boolean; depth: number; num: number; ver: number }

export interface Node {
  id: string;
  kind: Kind;
  text: string;
  marks: Mark[];
  checked: boolean;
  parent: string | null;
  children: string[];
  meta: Accessor<Meta>;
  setMeta: Setter<Meta>;
}

/** The last page's pending saves; the list waits for them before reloading. */
export const saves = { last: Promise.resolve() as Promise<unknown> };

let seq = 0;
export function newId(): string {
  const r = crypto.getRandomValues(new Uint32Array(2));
  return "b" + Date.now().toString(36) + (seq++).toString(36) + r[0].toString(36) + r[1].toString(36);
}

/** Ordered, serialised calls to `vault.apply`. Text edits are coalesced per
 *  block and sent after a short pause, or before the next structural change
 *  so the core always sees ops in the order they happened. */
export class Saver {
  private pending = new Map<string, () => BlockOp>();
  private chain: Promise<void> = Promise.resolve();
  private timer = 0;
  private inflight = 0;
  constructor(private pageId: string, private onState: (s: "saving" | "saved" | string) => void) {}

  text(n: Node) {
    this.pending.set(n.id, () => ({ op: "SetText", id: n.id, text: n.text, marks: toApiMarks(n.text, n.marks) }));
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => this.flushText(), 250);
  }

  send(ops: BlockOp[]) {
    this.enqueue([...this.drain(), ...ops]);
  }

  flushText() { this.enqueue(this.drain()); }

  private drain(): BlockOp[] {
    clearTimeout(this.timer);
    const ops = [...this.pending.values()].map((f) => f());
    this.pending.clear();
    return ops;
  }

  private enqueue(ops: BlockOp[]) {
    if (!ops.length) return;
    this.inflight++;
    this.onState("saving");
    this.chain = this.chain
      .then(() => api.apply(this.pageId, ops))
      .then(() => { if (--this.inflight === 0) this.onState("saved"); })
      .catch((e) => { this.inflight--; this.onState(`error: ${e}`); console.error("apply failed", e, ops); });
  }

  /** Everything sent and acknowledged (before leaving the page). */
  async settle() { this.flushText(); await this.chain; }
}

export class Doc {
  nodes = new Map<string, Node>();
  top: string[] = [];
  order: Accessor<string[]>;
  private setOrder: Setter<string[]>;
  saver: Saver;

  constructor(public page: ApiPage, onSave: (s: string) => void) {
    [this.order, this.setOrder] = createSignal<string[]>([]);
    this.saver = new Saver(page.id, onSave);
    const add = (b: ApiBlock, parent: string | null) => {
      const n = this.make(b.id, b.kind, b.text, fromApiMarks(b.text, b.marks), b.checked, parent);
      for (const c of b.children) n.children.push(add(c, b.id).id);
      return n;
    };
    for (const b of page.blocks) this.top.push(add(b, null).id);
    this.reflow();
  }

  private make(id: string, kind: Kind, text: string, marks: Mark[], checked: boolean, parent: string | null): Node {
    const [meta, setMeta] = createSignal<Meta>({ kind, checked, depth: 0, num: 0, ver: 0 });
    const n: Node = { id, kind, text, marks, checked, parent, children: [], meta, setMeta };
    this.nodes.set(id, n);
    return n;
  }

  get(id: string) { return this.nodes.get(id)!; }
  siblings(parent: string | null) { return parent === null ? this.top : this.get(parent).children; }

  /** Recomputes the flat order, depths and list numbers; updates only the
   *  blocks whose presentation changed. */
  reflow() {
    const out: string[] = [];
    const visit = (ids: string[], depth: number) => {
      let num = 0;
      for (const id of ids) {
        const n = this.get(id);
        num = n.kind === "Numbered" ? num + 1 : 0;
        const m = n.meta();
        if (m.depth !== depth || m.num !== num || m.kind !== n.kind || m.checked !== n.checked) {
          n.setMeta({ ...m, depth, num, kind: n.kind, checked: n.checked });
        }
        out.push(id);
        visit(n.children, depth + 1);
      }
    };
    visit(this.top, 0);
    this.setOrder(out);
  }

  /** Forces the block's inline DOM to be rebuilt from the model. */
  rerender(n: Node) { n.setMeta((m) => ({ ...m, kind: n.kind, checked: n.checked, ver: m.ver + 1 })); }

  prevVisible(id: string): string | null {
    const o = this.order();
    const i = o.indexOf(id);
    return i > 0 ? o[i - 1] : null;
  }
  nextVisible(id: string): string | null {
    const o = this.order();
    const i = o.indexOf(id);
    return i >= 0 && i + 1 < o.length ? o[i + 1] : null;
  }

  isInside(id: string, ancestor: string) {
    for (let p: string | null = id; p; p = this.get(p).parent) if (p === ancestor) return true;
    return false;
  }

  // ---------- edits (each mutates locally, then queues the matching ops) ----------

  textChanged(n: Node) { this.saver.text(n); }

  setText(n: Node, text: string, marks: Mark[]) {
    n.text = text;
    n.marks = normalize(marks, text.length);
    this.rerender(n);
    this.saver.text(n);
  }

  insertAfter(ref: Node | null, parent: string | null, kind: Kind, text = "", marks: Mark[] = []): Node {
    const n = this.make(newId(), kind, text, marks, false, parent);
    const sib = this.siblings(parent);
    const at = ref ? sib.indexOf(ref.id) + 1 : 0;
    sib.splice(at, 0, n.id);
    this.saver.send([{ op: "Insert", parent, after: ref?.id ?? null,
      block: { id: n.id, kind, text, marks: toApiMarks(text, marks), checked: false } }]);
    this.reflow();
    return n;
  }

  /** Enter at `at`: returns the block and offset that should get the caret. */
  split(n: Node, at: number): [Node, number] {
    const list = n.kind === "Bulleted" || n.kind === "Numbered" || n.kind === "Todo";
    if (n.text === "" && (list || n.kind === "Quote")) {
      if (n.parent) this.outdent(n);
      else this.setKind(n, "Paragraph");
      return [n, 0];
    }
    const nextKind: Kind = list ? n.kind : n.kind === "Quote" ? "Paragraph" : "Paragraph";
    if (at === 0 && n.text !== "") {
      // Enter at the start: an empty block above, caret stays.
      const sib = this.siblings(n.parent);
      const i = sib.indexOf(n.id);
      const prev = i > 0 ? this.get(sib[i - 1]) : null;
      this.insertAfter(prev, n.parent, list ? n.kind : "Paragraph");
      return [n, 0];
    }
    const right = n.text.slice(at), rightMarks = sliceMarks(n.marks, at, n.text.length);
    n.text = n.text.slice(0, at);
    n.marks = sliceMarks(n.marks, 0, at);
    this.rerender(n);
    this.saver.text(n);
    const m = n.children.length
      ? this.insertAfter(null, n.id, nextKind, right, rightMarks)
      : this.insertAfter(n, n.parent, nextKind, right, rightMarks);
    return [m, 0];
  }

  /** Backspace at offset 0. Returns where the caret goes, or null for no-op. */
  backspaceAtStart(n: Node): [Node, number] | null {
    if (n.kind !== "Paragraph" && n.kind !== "Divider") {
      this.setKind(n, "Paragraph");
      return [n, 0];
    }
    if (n.parent) {
      const sib = this.siblings(n.parent);
      // Only the last child can outdent cleanly; others merge upward.
      if (sib[sib.length - 1] === n.id) { this.outdent(n); return [n, 0]; }
    }
    const prevId = this.prevVisible(n.id);
    if (!prevId) return null;
    const prev = this.get(prevId);
    if (prev.kind === "Divider") { this.remove(prev); return [n, 0]; }
    return this.mergeInto(prev, n);
  }

  /** Appends `n`'s text to `prev`, lifts `n`'s children, deletes `n`. */
  mergeInto(prev: Node, n: Node): [Node, number] {
    const at = prev.text.length;
    prev.text += n.text;
    prev.marks = normalize([...prev.marks, ...shiftMarks(n.marks, at)], prev.text.length);
    this.rerender(prev);
    this.saver.text(prev);
    const ops: BlockOp[] = [];
    const sib = this.siblings(n.parent);
    let after = n.id;
    let idx = sib.indexOf(n.id);
    for (const c of [...n.children]) {
      ops.push({ op: "Move", id: c, parent: n.parent, after });
      this.get(c).parent = n.parent;
      sib.splice(++idx, 0, c);
      after = c;
    }
    n.children = [];
    ops.push({ op: "Delete", id: n.id });
    sib.splice(sib.indexOf(n.id), 1);
    this.nodes.delete(n.id);
    this.saver.send(ops);
    this.reflow();
    return [prev, at];
  }

  remove(n: Node) {
    const sib = this.siblings(n.parent);
    sib.splice(sib.indexOf(n.id), 1);
    const drop = (id: string) => { const x = this.get(id); x.children.forEach(drop); this.nodes.delete(id); };
    drop(n.id);
    this.saver.send([{ op: "Delete", id: n.id }]);
    this.reflow();
  }

  setKind(n: Node, kind: Kind) {
    if (n.kind === kind) return;
    const ops: BlockOp[] = [{ op: "SetKind", id: n.id, kind }];
    n.kind = kind;
    if (kind === "Divider" || kind === "Code") {
      if (kind === "Divider") n.text = "";
      n.marks = [];
    }
    if (kind !== "Todo" && n.checked) { n.checked = false; ops.push({ op: "SetChecked", id: n.id, checked: false }); }
    this.saver.text(n);
    this.saver.send(ops);
    this.rerender(n);
    this.reflow();
  }

  toggleChecked(n: Node) {
    n.checked = !n.checked;
    this.saver.send([{ op: "SetChecked", id: n.id, checked: n.checked }]);
    this.rerender(n);
  }

  move(n: Node, parent: string | null, after: string | null) {
    if (parent && this.isInside(parent, n.id)) return false;
    if (after === n.id) return false;
    const from = this.siblings(n.parent);
    from.splice(from.indexOf(n.id), 1);
    const to = this.siblings(parent);
    to.splice(after ? to.indexOf(after) + 1 : 0, 0, n.id);
    n.parent = parent;
    this.saver.send([{ op: "Move", id: n.id, parent, after }]);
    this.reflow();
    return true;
  }

  indent(n: Node) {
    const sib = this.siblings(n.parent);
    const i = sib.indexOf(n.id);
    if (i <= 0) return false;
    const np = this.get(sib[i - 1]);
    if (np.kind === "Divider") return false;
    return this.move(n, np.id, np.children[np.children.length - 1] ?? null);
  }

  outdent(n: Node) {
    if (!n.parent) return false;
    const p = this.get(n.parent);
    return this.move(n, p.parent, p.id);
  }

  moveUp(n: Node) {
    const sib = this.siblings(n.parent);
    const i = sib.indexOf(n.id);
    if (i > 0) return this.move(n, n.parent, i >= 2 ? sib[i - 2] : null);
    // First child: move out, above the parent.
    if (n.parent) {
      const p = this.get(n.parent);
      const ps = this.siblings(p.parent);
      const pi = ps.indexOf(p.id);
      return this.move(n, p.parent, pi > 0 ? ps[pi - 1] : null);
    }
    return false;
  }

  moveDown(n: Node) {
    const sib = this.siblings(n.parent);
    const i = sib.indexOf(n.id);
    if (i < sib.length - 1) return this.move(n, n.parent, sib[i + 1]);
    if (n.parent) return this.move(n, this.get(n.parent).parent, n.parent);
    return false;
  }

  canMoveUp(n: Node) { return !!n.parent || this.top.indexOf(n.id) > 0; }
  canMoveDown(n: Node) { return !!n.parent || this.top.indexOf(n.id) < this.top.length - 1; }
}
