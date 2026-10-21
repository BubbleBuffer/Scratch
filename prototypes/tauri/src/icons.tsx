// Thin line icons (1.5px strokes on a 20px grid), inline
// SVG so nothing is loaded at runtime.
import type { JSX } from "solid-js";

type P = { size?: number };
const S = (p: { size?: number; children: JSX.Element; fill?: boolean }) => (
  <svg width={p.size ?? 20} height={p.size ?? 20} viewBox="0 0 20 20" fill={p.fill ? "currentColor" : "none"}
    stroke={p.fill ? "none" : "currentColor"} stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    {p.children}
  </svg>
);
const T = (p: { size?: number; text: string; fs?: number; weight?: number; italic?: boolean }) => (
  <svg width={p.size ?? 20} height={p.size ?? 20} viewBox="0 0 20 20" aria-hidden="true">
    <text x="10" y="10.5" text-anchor="middle" dominant-baseline="middle" fill="currentColor"
      font-size={String(p.fs ?? 12)} font-weight={String(p.weight ?? 600)} font-style={p.italic ? "italic" : "normal"}
      style={{ "font-family": "var(--font)" }}>{p.text}</text>
  </svg>
);

export const IPage = (p: P) => (
  <S size={p.size}><path d="M5.5 3h6.2l3.3 3.3V16a1 1 0 0 1-1 1h-8.5a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z" /><path d="M11.5 3.2V6.5h3.3" /><path d="M7.5 10h5M7.5 13h4" /></S>
);
export const ISearch = (p: P) => (<S size={p.size}><circle cx="8.8" cy="8.8" r="5" /><path d="m12.6 12.6 4 4" /></S>);
export const IHome = (p: P) => (<S size={p.size}><path d="M3.5 9 10 3.5 16.5 9" /><path d="M5.5 7.5V16h3v-4.5h3V16h3V7.5" /></S>);
export const IInbox = (p: P) => (<S size={p.size}><path d="M3.5 11 5.5 4h9l2 7v5h-13z" /><path d="M3.5 11h4a2.5 2.5 0 0 0 5 0h4" /></S>);
export const ICompose = (p: P) => (<S size={p.size}><path d="M15 3.5 16.5 5 9.5 12l-2.3.8.8-2.3z" /><path d="M16 11v4.5a1 1 0 0 1-1 1H4.5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1H9" /></S>);
export const IPlus = (p: P) => (<S size={p.size}><path d="M10 4v12M4 10h12" /></S>);
export const IX = (p: P) => (<S size={p.size}><path d="M5.5 5.5l9 9M14.5 5.5l-9 9" /></S>);
export const IBack = (p: P) => (<S size={p.size ?? 24}><path d="M12.5 4.5 7 10l5.5 5.5" /></S>);
export const IArrowLeft = (p: P) => (<S size={p.size}><path d="M16 10H4.5M9 5.5 4.5 10 9 14.5" /></S>);
export const IChevR = (p: P) => (<S size={p.size ?? 16}><path d="M8 5l5 5-5 5" /></S>);
export const IChevDown = (p: P) => (<S size={p.size ?? 12}><path d="M5 8l5 5 5-5" /></S>);
export const IStar = (p: P) => (<S size={p.size}><path d="m10 3.3 2 4.3 4.6.5-3.4 3.1 1 4.6L10 13.5l-4.2 2.3 1-4.6-3.4-3.1 4.6-.5z" /></S>);
export const IClock = (p: P) => (<S size={p.size ?? 14}><circle cx="10" cy="10" r="6.5" /><path d="M10 6.5V10l2.5 1.5" /></S>);
export const IMore = (p: P) => (
  <S size={p.size} fill><circle cx="4.5" cy="10" r="1.4" /><circle cx="10" cy="10" r="1.4" /><circle cx="15.5" cy="10" r="1.4" /></S>
);
export const IGrip = () => (
  <svg width="10" height="14" viewBox="0 0 10 14" fill="currentColor" aria-hidden="true">
    <circle cx="2.5" cy="2.5" r="1.25" /><circle cx="7.5" cy="2.5" r="1.25" />
    <circle cx="2.5" cy="7" r="1.25" /><circle cx="7.5" cy="7" r="1.25" />
    <circle cx="2.5" cy="11.5" r="1.25" /><circle cx="7.5" cy="11.5" r="1.25" />
  </svg>
);
export const ICheck = (p: P) => (
  <svg width={p.size ?? 12} height={p.size ?? 12} viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.8"
    stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M2.5 6.3 4.9 8.6 9.5 3.6" /></svg>
);
export const ILink = (p: P) => (
  <S size={p.size}><path d="M8.5 11.5a3 3 0 0 0 4.2 0l2.6-2.6a3 3 0 0 0-4.2-4.2l-.9.9" /><path d="M11.5 8.5a3 3 0 0 0-4.2 0l-2.6 2.6a3 3 0 0 0 4.2 4.2l.9-.9" /></S>
);
export const IIndent = (p: P) => (<S size={p.size}><path d="M3.5 4.5h13M9.5 8.5h7M9.5 12h7M3.5 15.5h13" /><path d="M3.5 7.8 6.2 10.2 3.5 12.6" /></S>);
export const IOutdent = (p: P) => (<S size={p.size}><path d="M3.5 4.5h13M9.5 8.5h7M9.5 12h7M3.5 15.5h13" /><path d="M6.2 7.8 3.5 10.2 6.2 12.6" /></S>);
export const IKbHide = (p: P) => (
  <S size={p.size}><rect x="2.5" y="3.5" width="15" height="9" rx="1.5" /><path d="M5.5 6.5h1M9.5 6.5h1M13.5 6.5h1M6.5 9.5h7M7.5 15l2.5 2 2.5-2" /></S>
);
export const IUp = (p: P) => (<S size={p.size}><path d="M10 16V4.5M5 9.5l5-5 5 5" /></S>);
export const IDown = (p: P) => (<S size={p.size}><path d="M10 4v11.5M5 10.5l5 5 5-5" /></S>);
export const ITrash = (p: P) => (<S size={p.size}><path d="M3.5 5.5h13M8 5.5V4h4v1.5M5.2 5.5l.8 10.5a1 1 0 0 0 1 .9h6a1 1 0 0 0 1-.9l.8-10.5M8.5 8.5v5.5M11.5 8.5v5.5" /></S>);
export const ITurn = (p: P) => (
  <S size={p.size}><path d="M4.5 8a5.5 5.5 0 0 1 10-2" /><path d="M15.5 12a5.5 5.5 0 0 1-10 2" /><path d="M14.8 3v3.2h-3.2M5.2 17v-3.2h3.2" /></S>
);

// Block kind icons (slash menu, turn-into lists, keyboard toolbar).
export const IText = (p: P) => (<S size={p.size}><path d="M5 5.5V4h10v1.5M10 4v12M8 16h4" /></S>);
export const IH1 = (p: P) => <T size={p.size} text="H1" />;
export const IH2 = (p: P) => <T size={p.size} text="H2" />;
export const IH3 = (p: P) => <T size={p.size} text="H3" />;
export const IBullet = (p: P) => (
  <S size={p.size}><path d="M8 5.5h9M8 10h9M8 14.5h9" /><circle cx="4" cy="5.5" r=".9" fill="currentColor" /><circle cx="4" cy="10" r=".9" fill="currentColor" /><circle cx="4" cy="14.5" r=".9" fill="currentColor" /></S>
);
export const INumbered = (p: P) => (
  <S size={p.size}><path d="M8.5 5.5h8.5M8.5 10h8.5M8.5 14.5h8.5" /><path d="M3.5 4.2 4.6 3.6v4" stroke-width="1.2" /><path d="M3.3 12.2c.2-.6.8-.9 1.3-.8.6.1.9.6.7 1.2-.2.5-2 1.9-2 1.9h2.2" stroke-width="1.2" /></S>
);
export const ITodo = (p: P) => (<S size={p.size}><rect x="3.5" y="3.5" width="13" height="13" rx="2.5" /><path d="m6.8 10.2 2.2 2.2 4.3-4.8" /></S>);
export const IQuote = (p: P) => (<S size={p.size}><path d="M4.5 4v12" stroke-width="2" /><path d="M8.5 6h8M8.5 10h8M8.5 14h5" /></S>);
export const ICode = (p: P) => (<S size={p.size}><path d="M7 6 3 10l4 4M13 6l4 4-4 4" /></S>);
export const IDivider = (p: P) => (<S size={p.size}><path d="M3 10h14" /><path d="M6 6h8M6 14h8" stroke-width="1" opacity=".45" /></S>);
export const ICopy = (p: P) => (<S size={p.size ?? 14}><rect x="7" y="7" width="9" height="9" rx="1.5" /><path d="M13 7V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v7a1 1 0 0 0 1 1h2" /></S>);
