// BAKEOFF instrumentation (the Bake-off spec in the internal wiki; current state: wiki/prototypes/tauri.md).
// Every line goes through the `bakeoff_log` command, which writes it with the
// `BAKEOFF` prefix to logcat (Android) or stdout and <app data>/bakeoff.log.
import { api } from "./api";

export function blog(line: string) {
  api.log(line).catch(() => {});
}

/** Runs `cb` after the next frame has been produced: rAF runs just before
 *  paint, and a message posted from it is handled right after that frame. */
export function afterPaint(cb: (t: number) => void) {
  requestAnimationFrame(() => {
    const ch = new MessageChannel();
    ch.port1.onmessage = () => cb(performance.now());
    ch.port2.postMessage(0);
  });
}

function pct(sorted: number[], p: number) {
  if (!sorted.length) return 0;
  const i = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1));
  return sorted[i];
}
const f1 = (n: number) => n.toFixed(1);

export function summary(values: number[]) {
  const s = [...values].sort((a, b) => a - b);
  return { n: s.length, p50: pct(s, 50), p95: pct(s, 95), p99: pct(s, 99), max: s[s.length - 1] ?? 0 };
}

/** Frame times from requestAnimationFrame deltas while a scroller moves.
 *  Sampling starts on the first scroll event and stops 250 ms after the last;
 *  each stop logs the cumulative stats for this surface. */
export class FrameMeter {
  private deltas: number[] = [];
  private bursts = 0;
  private running = false;
  private prev = -1;
  private lastScroll = 0;

  constructor(private surface: string) {}

  reset() { this.deltas = []; this.bursts = 0; }

  attach(el: HTMLElement) {
    el.addEventListener("scroll", () => this.onScroll(), { passive: true });
  }

  private onScroll() {
    this.lastScroll = performance.now();
    if (this.running) return;
    this.running = true;
    this.prev = -1;
    requestAnimationFrame(this.tick);
  }

  private tick = (t: number) => {
    if (this.prev >= 0) this.deltas.push(t - this.prev);
    this.prev = t;
    if (performance.now() - this.lastScroll > 250) {
      this.running = false;
      this.bursts++;
      this.report();
      return;
    }
    requestAnimationFrame(this.tick);
  };

  report() {
    const s = summary(this.deltas);
    const over = this.deltas.filter((d) => d > 16.7).length;
    blog(
      `frames surface=${this.surface} bursts=${this.bursts} n=${s.n} p50=${f1(s.p50)} p95=${f1(s.p95)} ` +
        `p99=${f1(s.p99)} max=${f1(s.max)} over16.7=${s.n ? f1((100 * over) / s.n) : "0.0"}%`,
    );
  }
}

/** Input latency: from the input event's timestamp to the next painted frame. */
export class InputMeter {
  private lat: number[] = [];
  reset() { this.lat = []; }
  /** `t0`: the key event's timestamp (performance.now() clock). */
  record(t0: number) {
    afterPaint((t1) => {
      this.lat.push(t1 - t0);
      if (this.lat.length % 20 === 0) this.report();
    });
  }
  report() {
    const s = summary(this.lat);
    blog(`input n=${s.n} p50=${f1(s.p50)} p95=${f1(s.p95)} max=${f1(s.max)}`);
  }
}

export const listFrames = new FrameMeter("list");
export const pageFrames = new FrameMeter("page");
export const inputMeter = new InputMeter();
