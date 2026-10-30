// Settings redesign mockup. Every option here exists in Scratch today
// (audit.md); data is illustrative. State lives in the URL hash so each
// screen and state can be linked and captured.

const S = {
  page: "general", theme: "light", data: "filled", sync: "connected", runner: "computer", tools: "everything",
  agent: true, access: true, help: false, hoverHelp: "", open: "", chrome: true, phoneList: false, tall: false,
  hubShare: true, prefetch: false, reminders: true, autoResolve: true, relay: false, shake: true, schedules: false,
  edit: "", // "project" | "server" | "token" | "device"
};
const BOOL = ["agent", "access", "help", "chrome", "phoneList", "tall", "hubShare", "prefetch", "reminders", "autoResolve", "relay", "shake", "schedules"];

function readHash() {
  const p = new URLSearchParams(location.hash.slice(1));
  for (const [k, v] of p) if (k in S) S[k] = BOOL.includes(k) ? v === "1" : v;
}
function writeHash() {
  const d = new URLSearchParams();
  for (const k of ["page", "theme", "data", "sync", "runner", "tools", "help", "edit"]) d.set(k, BOOL.includes(k) ? (S[k] ? "1" : "0") : S[k]);
  if (!S.access) d.set("access", "0");
  if (!S.chrome) d.set("chrome", "0");
  if (S.phoneList) d.set("phoneList", "1");
  if (S.tall) d.set("tall", "1");
  if (S.schedules) d.set("schedules", "1");
  if (S.open) d.set("open", S.open);
  if (S.hoverHelp) d.set("hoverHelp", S.hoverHelp);
  history.replaceState(null, "", "#" + d);
}

const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const ic = (n) => `<i data-lucide="${n}"></i>`;
const filled = () => S.data === "filled";
const signedIn = () => S.sync === "connected";
const everything = () => S.tools === "everything";
// Help notes are keyed by the row label (a slug), so a hash state can name one: hoverHelp=model.
const slug = (s) => String(s).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");

// ---- building blocks ----
function row({ label, desc, help, ctl = "", stack = false, stackPhone = false, disabled = false, reason = "", scope = "", id = "" }) {
  const hz = help ? slug(label) : "";
  const helpBtn = help ? `<button class="help-btn" aria-expanded="false" aria-controls="help-${hz}" data-hz="${hz}" title="When to change this" aria-label="When to change ${esc(label)}">${ic("circle-help")}</button>` : "";
  return `<div class="row${stack ? " stack" : ""}${stackPhone ? " stack-phone" : ""}${disabled ? " is-disabled" : ""}"${id ? ` id="${id}"` : ""}>
    <div><div class="label">${label}${scope ? ` <span class="scope">${scope}</span>` : ""}${helpBtn}</div>
      ${desc ? `<div class="desc">${desc}</div>` : ""}
      ${reason ? `<div class="reason">${ic("lock")}${reason}</div>` : ""}
      ${help ? `<div class="help" id="help-${hz}" data-hz="${hz}">${help}</div>` : ""}</div>
    ${ctl ? `<div class="ctl">${ctl}</div>` : ""}</div>`;
}
const sw = (key, label, disabled = false) => `<button class="switch" role="switch" aria-checked="${!!S[key]}" aria-label="${esc(label)}" data-toggle="${key}"${disabled ? " disabled" : ""}></button>`;
const swv = (on, label) => `<button class="switch" role="switch" aria-checked="${on}" aria-label="${esc(label)}"></button>`;
const seg = (opts, value, key) => `<div class="seg" role="group">${opts.map(([v, l]) => `<button aria-pressed="${v === value}" ${key ? `data-set="${key}=${v}"` : ""}>${l}</button>`).join("")}</div>`;
const select = (opts, value, label, disabled = false) => `<select class="select" aria-label="${esc(label)}"${disabled ? " disabled" : ""}>${opts.map((o) => `<option${o === value ? " selected" : ""}>${o}</option>`).join("")}</select>`;
const btn = (text, { kind = "", icon = "", act = "", disabled = false } = {}) => `<button class="btn ${kind}"${act ? ` data-act="${act}"` : ""}${disabled ? " disabled" : ""}>${icon ? ic(icon) : ""}${text}</button>`;
const iconBtn = (icon, label, { danger = false, act = "" } = {}) => `<button class="icon-btn${danger ? " danger" : ""}" title="${esc(label)}" aria-label="${esc(label)}"${act ? ` data-act="${act}"` : ""}>${ic(icon)}</button>`;
const section = (title, intro, body, action = "") => `<section class="section"><div class="section-head"><div class="grow"><h3>${title}</h3>${intro ? `<p>${intro}</p>` : ""}</div>${action}</div>${body}</section>`;
// Advanced disclosures keep the state the user left them in, because draw() rebuilds the page.
const ADV_OPEN = new Set();
const adv = (title, note, body, open = false) => {
  const key = S.page + "|" + title;
  return `<details class="adv" data-adv="${esc(key)}"${open || ADV_OPEN.has(key) ? " open" : ""}><summary>${ic("chevron-right")}${title}<small>${note}</small></summary>${body}</details>`;
};
const copyline = (text, label = "Copy") => `<div class="copyline"><code>${esc(text)}</code>${btn(label, { kind: "quiet", icon: "copy" })}</div>`;
const item = ({ glyph, name, sub, mono = false, status = "", err = "", acts = "", tags = "" }) => `<div class="item"><div class="glyph">${ic(glyph)}</div>
  <div style="min-width:0"><div class="name">${name}${tags}${status}</div>${sub ? `<div class="sub${mono ? " mono" : ""}" title="${esc(sub)}">${esc(sub)}</div>` : ""}${err ? `<div class="err">${err}</div>` : ""}</div>
  <div class="acts">${acts}</div></div>`;
const status = (kind, text) => `<span class="status ${kind}">${text}</span>`;
const empty = (glyph, title, text, action = "") => `<div class="empty">${ic(glyph)}<b>${title}</b><p>${text}</p>${action}</div>`;
const notice = (glyph, html, kind = "") => `<div class="notice ${kind}">${ic(glyph)}<div>${html}</div></div>`;
const help = (when, extra = "") => `<b>When to change it:</b> ${when}${extra ? `<br>${extra}` : ""}`;

// ---- pages ----
const PAGES = [
  { group: "", items: [["general", "General", "sliders-horizontal"], ["reminders", "Reminders", "bell"]] },
  { group: "Your vault", items: [["sync", "Sync and devices", "refresh-cw"], ["conflicts", "Conflicts", "git-compare"], ["storage", "Storage", "hard-drive"]] },
  { group: "AI", items: [["agent", "Agent", "bot"], ["connectors", "Connectors", "plug"]] },
  { group: "", items: [["about", "About", "info"]] },
];
const navValue = {
  general: () => ({ light: "Light", dark: "Dark" })[S.theme],
  reminders: () => (S.reminders ? "On" : "Off"),
  sync: () => (signedIn() ? "Synced" : "Off"),
  conflicts: () => "",
  agent: () => (S.agent ? (S.runner === "hub" ? "Hub" : "This computer") : "Off"),
};

const render = {
  general: () => `
    <h2 class="page-title">General</h2>
    ${section("Appearance", "", row({ label: "Theme", desc: "Light or dark colours, on this device only.",
      help: help("Leave it on System to follow Windows or Android, including their night schedule."),
      ctl: seg([["system", "System"], ["light", "Light"], ["dark", "Dark"]], S.theme === "dark" ? "dark" : "light", "theme"), stackPhone: true }))}
    ${section("Feedback", "", row({ label: "Send feedback", desc: "Write what happened. A screenshot and the app’s diagnostics go with it.",
      help: help("Whenever something breaks, looks wrong or confuses you."), ctl: btn("Write feedback", { icon: "message-square-warning" }), stackPhone: true })
      + (phone() ? row({ label: "Shake to send feedback", desc: "Shake the phone to open the feedback sheet.", scope: "Phone",
      help: help("Turn off if it opens by accident, for example while walking."), ctl: sw("shake", "Shake to send feedback") }) : ""))}`,

  reminders: () => `
    <h2 class="page-title">Reminders</h2>
    <p class="page-intro">Notifications on this device for upcoming events, and one summary of your day each morning.</p>
    ${phone() ? notice("bell-off", "<b>Android is blocking Scratch’s notifications.</b> Your reminder settings are kept; turn notifications on for Scratch in Android’s settings to get them.", "warn") : ""}
    ${section("", "", row({ label: "Reminders", desc: "Notify me about events and send a morning summary.", help: help("Turn off on a device where you don’t want to be notified, such as a shared computer."), ctl: sw("reminders", "Reminders") })
      + row({ label: "Remind me before events", desc: "How early the notification comes.", disabled: !S.reminders, reason: S.reminders ? "" : "Turn on Reminders to change this.",
        ctl: select(["At start", "5 min before", "10 min before", "30 min before"], "10 min before", "Remind me before events", !S.reminders) })
      + row({ label: "Morning summary", desc: "When the day’s summary arrives.", disabled: !S.reminders, reason: S.reminders ? "" : "Turn on Reminders to change this.",
        ctl: select(["05:00", "06:00", "07:00", "07:30", "08:00", "09:00", "10:00", "12:00"], "08:00", "Morning summary time", !S.reminders) }))}`,

  sync: () => {
    const conn = signedIn();
    const card = conn
      ? `<div class="status-card"><div class="big">${ic("check")}</div><div><div class="t">Synced</div><div class="s">With <span class="mono">notes.example.com</span> · last synced 2 minutes ago</div></div>${btn("Sync now", { icon: "refresh-cw" })}</div>`
      : `<div class="status-card"><div class="big off">${ic("cloud-off")}</div><div><div class="t">Not syncing</div><div class="s">This vault lives only on this device. Connect a hub to use it on your other devices.</div></div></div>`;
    const account = conn
      ? row({ label: "Hub account", desc: "Signed in to <span class=\"mono\">notes.example.com</span> as <b>sam</b>.", help: help("Sign out to switch hubs or accounts. Your pages stay on this device."), ctl: btn("Sign out") })
      : row({ label: "Sign in to your hub", desc: "Your hub’s web address. Scratch opens your browser to sign in.", stack: true,
          help: help("Use this if your hub runs on a server, such as one you host or one you were invited to."),
          ctl: `<div class="inline"><input class="field" placeholder="https://notes.example.com" aria-label="Hub address">${btn("Sign in", { kind: "primary" })}</div>` })
        + adv("Connect with a pairing code", "for a hub on another of your computers", row({ label: "Pairing code or hub address", stack: true,
          desc: "On the computer that runs the hub, open Settings › Sync and devices › Add a device, then paste the code here.",
          ctl: `<div style="width:100%"><input class="field mono" placeholder="notes://pair?… or http://203.0.113.20:47100" aria-label="Pairing code or hub address">
            <label class="field-label">Token</label><input class="field mono" placeholder="Filled in by the pairing code"><div class="field-hint">Pasting a pairing code fills both fields.</div>
            <div style="margin-top:10px">${btn("Connect")}</div></div>` }));
    const devices = filled()
      ? `<div class="list">${item({ glyph: "smartphone", name: "Phone", sub: "Device 3f9a12c0", status: status("ok", "Synced today"), acts: btn("Unpair", { kind: "quiet danger" }) })}
         ${item({ glyph: "tablet", name: "Tablet", sub: "Paired, not connected yet", status: status("off", "Waiting"), acts: btn("Unpair", { kind: "quiet danger" }) })}
         <div class="list-foot">${btn("Add a device", { icon: "plus", act: "edit=device" })}</div></div>`
      : empty("smartphone", "No devices paired yet", "Pair your phone or another computer to sync with this one. You’ll get a code to scan or paste.", btn("Add a device", { kind: "primary", icon: "plus", act: "edit=device" }));
    const pair = S.edit === "device" ? `<div class="editor"><h4>Add a device</h4><p class="desc" style="color:var(--ink2);margin:4px 0 0">Name the device, then scan the code with it or copy the pairing code to it.</p>
      <label class="field-label">Device name</label><input class="field" value="Phone">
      <div class="foot">${btn("Cancel", { kind: "quiet", act: "edit=" })}${btn("Create pairing code", { kind: "primary" })}</div></div>` : "";
    return `<h2 class="page-title">Sync and devices</h2>
      <p class="page-intro">Keep this vault the same on all your devices. A <b>hub</b> is the computer or server your devices sync through.</p>
      ${card}
      ${section("Your hub", "", account)}
      ${section("Attachments", "", row({ label: "Download attachments in the background", desc: "Otherwise files and images download when a page shows them.",
        help: help("Turn on to have every file available offline. Leave off on a phone with little storage or a metered connection."), ctl: sw("prefetch", "Download attachments in the background") })
        + row({ label: "Download all attachments now", desc: conn ? "Get every file once, to have them offline." : "Connect to a hub first.", ctl: btn("Download", { disabled: !conn }) }))}
      ${phone() ? "" : section("This computer as a hub", "Let your other devices sync with this computer directly, without a server.",
        row({ label: "Share this vault with your devices", desc: "This computer runs a hub while Scratch is open. Your phone syncs with it over your network or Tailscale.",
          help: help("Turn on if you have no hub on a server. Turn off if you sync through a hosted hub and don’t need this computer to serve."), ctl: sw("hubShare", "Share this vault with your devices") })
        + (S.hubShare ? row({ label: "Reachable at", desc: "Devices can use any of these addresses.", stack: true,
          ctl: `<div style="width:100%;display:grid;gap:6px">${copyline("http://203.0.113.20:47100")}${copyline("http://198.51.100.12:47100")}</div>` })
          + row({ label: "Paired devices", desc: "Unpairing revokes a device’s access; it stops syncing.", stack: true, ctl: `<div style="width:100%">${devices}${pair}</div>` })
          + adv("Advanced", "network address", row({ label: "Listen address", desc: "Which network connections the hub accepts, and on which port.",
            help: help("Only if a firewall or another program needs a different port.", "<b>0.0.0.0</b> accepts connections from every network this computer is on. A port of 0 picks a free one."),
            ctl: `<div class="inline"><input class="field mono" value="0.0.0.0:47100" aria-label="Listen address">${btn("Apply")}</div>`, stack: true })) : "")
        + row({ label: "Run schedules on this device", desc: "While Scratch is open, this computer runs the vault’s schedules. Each step creates a note from a template; no AI model is involved.", scope: "Desktop",
          help: help("Turn it on for the one computer that’s usually on, because otherwise schedules don’t run."), ctl: sw("schedules", "Run schedules on this device") }))}
      ${conn ? adv("Trusted certificates", "1 certificate", `<div class="list">${item({ glyph: "shield-check", name: "notes.example.com", sub: "Issued by a private certificate authority · trusted on this device only", acts: btn("Revoke", { kind: "quiet danger" }) })}</div>`) : ""}`;
  },

  conflicts: () => `
    <h2 class="page-title">Conflicts</h2>
    <p class="page-intro">When the same page changes on two devices before they sync, Scratch keeps both versions and lists the page here.</p>
    ${section("Needs a look", "", filled()
      ? `<div class="list">${item({ glyph: "file-text", name: "Weekly plan", sub: "Changed on Phone and this computer · today 09:14", acts: btn("Review") })}${item({ glyph: "file-text", name: "Reading list", sub: "Deleted on Phone, edited here · yesterday", acts: btn("Review") })}</div>`
      : empty("circle-check", "Nothing to review", "No page has changed on two devices at once."))}
    ${section("Handling", "", row({ label: "Resolve simple conflicts on their own", desc: "On: a change that can settle itself leaves this list. Off: it waits here until you confirm it.",
      help: help("Turn off if you want to see every conflict, even ones Scratch can settle."), scope: "All devices", ctl: sw("autoResolve", "Resolve simple conflicts on their own") }))}
    ${section("Resolved in the last 30 days", "", filled() ? `<div class="list">${item({ glyph: "history", name: "Groceries", sub: "Kept the phone’s version · Oct 21", acts: btn("Undo", { kind: "quiet", icon: "undo-2" }) })}</div>` : `<p class="desc" style="color:var(--ink2)">Nothing resolved in the last 30 days.</p>`)}`,

  storage: () => `
    <h2 class="page-title">Storage</h2>
    <p class="page-intro">Where this vault’s pages live on this device. Pages are Markdown files you can open with other apps.</p>
    ${section("Vault folder", "", row({ label: "Location", desc: "The folder that holds your pages and their history.", stack: true, ctl: copyline("C:\\Users\\sam\\Documents\\Scratch") })
      + (phone() ? "" : adv("Open another folder", "starts or opens a different vault", row({ label: "Use a different folder", stack: true,
        desc: "An empty folder starts a new vault; a vault folder opens it. Scratch switches after a restart.",
        help: help("To keep separate vaults (for example work and personal), or to move the vault to another drive."),
        ctl: `<div class="inline"><input class="field mono" placeholder="D:\\Notes" aria-label="Folder">${btn("Choose…")}${btn("Use folder")}</div>` }))))}
    ${phone() ? "" : section("Mounted folders", "Show Markdown folders from elsewhere as pages. Edits save back to the files; outside changes appear within seconds.",
      filled() ? `<div class="list">${item({ glyph: "folder-sync", name: "Recipes", sub: "D:\\Recipes", mono: true, acts: btn("Remove", { kind: "quiet danger" }) })}<div class="list-foot">${btn("Mount a folder", { icon: "plus" })}</div></div>`
        : empty("folder-sync", "No mounted folders", "Mount a folder of Markdown files, such as a project’s docs, to read and edit it in Scratch.", btn("Mount a folder", { icon: "plus" })))}`,

  agent: () => {
    const comp = S.runner === "computer" && !phone();
    const toolsChoice = (v, t, d) => `<button class="choice" role="radio" aria-checked="${S.tools === v}" data-set="tools=${v}"><span class="t">${t}<span class="radio"></span></span><span class="d">${d}</span></button>`;
    const gated = !everything() ? "Needs Tools set to Everything." : !S.access ? "" : "";
    return `<h2 class="page-title">Agent</h2>
      <p class="page-intro">The agent is Scratch’s built-in AI. It answers in the Agent chat and when you choose <b>Ask AI</b> from the / menu.</p>
      ${section("", "", row({ label: "Use the agent", desc: "Show the Agent chat and Ask AI.", ctl: sw("agent", "Use the agent") }))}
      ${S.agent ? `
      ${section("Where it runs", "One choice for all your devices.", `<div class="choices" role="radiogroup" aria-label="Run AI on">
          <button class="choice" role="radio" aria-checked="${S.runner === "hub"}" data-set="runner=hub"><span class="t">${ic("server")}Your hub<span class="radio"></span></span><span class="d">Works the same from every device, including your phone. Uses the models signed in on the hub.</span></button>
          <button class="choice" role="radio" aria-checked="${S.runner === "computer"}" data-set="runner=computer"${phone() ? " disabled" : ""}><span class="t">${ic("monitor")}This computer<span class="radio"></span></span><span class="d">${phone() ? "Set this from Scratch on a computer. Phones always use the hub." : "Uses this computer’s models, and can work in its folders and programs."}</span></button></div>`
        + row({ label: "Model", desc: "New chats start with this model. Each chat can switch.",
          help: help("Pick a faster model for quick questions or a stronger one for long work. The list shows the models " + (S.runner === "hub" ? "the hub" : "this computer") + " can use."),
          ctl: select(S.runner === "hub" ? ["Hub default", "Fable 5.1", "Opus 5.5"] : ["Fable 5.1", "Opus 5.5", "Sonnet 5.5", "GPT-6.1 Sol"], "Fable 5.1", "Model") }))}
      ${section("What it may do", "Applies to " + (S.runner === "hub" ? "the agent on your hub." : "the agent on this computer."), `<div class="choices three" role="radiogroup" aria-label="Tools">
          ${toolsChoice("read", "Read", "Search and read your pages.")}
          ${toolsChoice("write", "Write", "Read, plus create and edit pages.")}
          ${toolsChoice("everything", "Everything", comp ? "Every vault tool, plus computer access below and MCP servers on Connectors." : "Every vault tool.")}</div>`)}
      ${comp ? section("On this computer", "Extra reach for the agent when it runs here.",
        row({ label: "Computer access", desc: "Lets the agent open project folders and run commands. <b>Commands are not sandboxed:</b> they can do anything you can.",
          help: help("Turn off to keep the agent inside your notes."), disabled: !everything(), reason: everything() ? "" : "Needs “Everything” above. With Read or Write this switch does nothing.",
          ctl: sw("access", "Computer access", !everything()) })
        + projectsBlock()
        + `<p class="desc" style="color:var(--ink2);margin:4px 0 0">MCP servers are on <button class="linkish" data-page="connectors">Connectors</button>.</p>`) : ""}
      ${comp ? adv("Advanced", "Claude Code provider", `<p class="desc" style="color:var(--ink2);max-width:60ch;margin:10px 0 0">The agent can use <b>Claude Code</b> as a model provider, with your Claude Code sign-in. It needs the <span class="mono">bun</span> program and BubblePaw’s Claude Code extension. Scratch finds both on its own; leave these empty unless Claude Code is missing from Model.</p>`
          + row({ label: "bun program", stack: true, desc: "Path to bun.exe.", help: help("Only if bun is installed somewhere Scratch doesn’t look.", "A wrong path hides Claude Code entirely. Clear the field to go back to automatic."),
            ctl: `<input class="field mono" placeholder="Found: C:\\Users\\sam\\.bun\\bin\\bun.exe" aria-label="bun program">` })
          + row({ label: "Claude Code extension folder", stack: true, desc: "The folder that holds the extension’s src/main.ts.", help: help("Only if you keep BubblePaw’s extension in your own checkout."),
            ctl: `<input class="field mono" placeholder="Found: C:\\Users\\sam\\BubblePaw\\extensions\\claude-code" aria-label="Claude Code extension folder">` })) : ""}
      ${section("Recent tasks", "Ask AI requests from all your devices.", filled()
        ? `<div class="list">${item({ glyph: "sparkles", name: "Summarise this meeting", sub: "Weekly sync · 10:42 · Phone", status: status("ok", "Done") })}
            ${item({ glyph: "sparkles", name: "Draft next week’s plan", sub: "Weekly plan · 09:15 · this computer", status: status("bad", "Failed"), err: "The model did not answer in time.", acts: btn("Retry", { kind: "quiet", icon: "rotate-ccw" }) })}</div>`
        : empty("sparkles", "No AI tasks yet", "Select text, or type / on any page and choose Ask AI."))}` : ""}`;
  },

  connectors: () => {
    const tokens = !signedIn() ? empty("key-round", "Sign in to a hub first", "Tokens are issued by your hub. Sign in under Sync and devices.", btn("Open Sync and devices", { act: "page=sync" }))
      : filled() ? `<div class="list">${item({ glyph: "key-round", name: "laptop-claude", tags: ` <span class="tag">Write</span>`, sub: "Created Oct 12 · last used 2 hours ago · expires in 80 days", acts: btn("Revoke", { kind: "quiet danger" }) })}
          ${item({ glyph: "key-round", name: "codex-work", tags: ` <span class="tag">Everything</span>`, sub: "Created Oct 20 · never used · never expires", acts: btn("Revoke", { kind: "quiet danger" }) })}
          <div class="list-foot">${btn("New token", { icon: "plus", act: "edit=token" })}</div></div>`
      : empty("key-round", "No tokens yet", "Make one token per AI app, so you can cut off one app without touching the others.", btn("New token", { kind: "primary", icon: "plus", act: "edit=token" }));
    const tokenForm = S.edit === "token" ? `<div class="editor"><h4>New token</h4>
      <div class="two"><div><label class="field-label">Name</label><input class="field mono" placeholder="laptop-claude"><div class="field-hint">Lowercase letters, digits and dashes.</div></div>
      <div><label class="field-label">Expires</label>${select(["In 90 days", "In 1 year", "Never"], "In 90 days", "Expires")}</div></div>
      <label class="field-label">What the app may do</label>${seg([["read", "Read"], ["write", "Write"], ["everything", "Everything"]], "write")}
      <div class="field-hint">The secret is shown once, right after you create the token.</div>
      <div class="foot">${btn("Cancel", { kind: "quiet", act: "edit=" })}${btn("Create token", { kind: "primary" })}</div></div>` : "";
    const appsBody = `${phone() ? "" : section("Quick setup", "Run one command in a terminal on this computer. It adds Scratch to the app and gives it its own token.",
        row({ label: "Claude Code", stack: true, ctl: copyline("notes setup claude") }) + row({ label: "Codex", stack: true, ctl: copyline("notes setup codex") }))}
      ${section("Access tokens", "Each token lets one app reach your vault through your hub. Revoke a token to cut that app off.", tokens + tokenForm)}`;
    const hubBody = phone() ? `<p class="desc" style="color:var(--ink2);margin:4px 0 0">Set this from Scratch on a computer. Phones always use the hub.</p>`
      : row({ label: "Let the hub use this computer", desc: "AI apps connected to your hub can read and edit files and run commands on this computer while Scratch is open.",
          help: help("Turn on to let an AI app on your phone or another machine work on this computer’s files. Leave off otherwise."),
          disabled: !signedIn(), reason: signedIn() ? "" : "Sign in to a hub first.", ctl: sw("relay", "Let the hub use this computer", !signedIn()) });
    return `<h2 class="page-title">Connectors</h2>
      <p class="page-intro">Connectors link Scratch with other tools, both ways. Scratch’s agent can use outside tools, and outside AI apps can use your vault.</p>
      ${cpanel("tools", "wrench", "Tools for the agent", "Extra tools the agent can use, from programs on this computer or web addresses.", toolsStatus(), toolsBody())}
      ${cpanel("apps", "app-window", "Apps that use your vault", "Let AI apps such as Claude Code or Codex read and edit your vault.", appsStatus(), appsBody)}
      ${cpanel("hub", "monitor", "This computer for hub apps", "Let AI apps on your other devices work on this computer’s files.", hubStatus(), hubBody)}`;
  },

  about: () => `
    <h2 class="page-title">About</h2>
    ${section("", "", row({ label: "Version", ctl: `<span class="mono">Scratch 0.1.0</span>` })
      + row({ label: "Device ID", desc: "Identifies this device to the hub and in conflicts. Support may ask for it.", stack: true, ctl: copyline("dev_7c41e09b2f6a4d18") }))}`,
};

// Collapsible panels on the Connectors page. Open ids are kept in the hash (open=tools,apps), so captures can open them.
const isOpen = (id) => S.open.split(",").includes(id);
function cpanel(id, glyph, title, desc, status, body) {
  const open = isOpen(id);
  return `<section class="cpanel${open ? " open" : ""}">
    <button class="cpanel-head" aria-expanded="${open}" aria-controls="cp-${id}" data-cpanel="${id}">
      <span class="cp-glyph">${ic(glyph)}</span>
      <span class="cp-text"><span class="cp-title">${title}</span><span class="cp-desc">${desc}</span></span>
      <span class="cp-status">${status}</span>${ic("chevron-down")}</button>
    <div class="cpanel-body" id="cp-${id}"${open ? "" : " hidden"}>${body}</div></section>`;
}
const toolsStatus = () => (phone() ? "On a computer" : !everything() ? "Off" : filled() ? "3 servers · 1 off" : "No servers");
const appsStatus = () => (!signedIn() ? "Needs a hub" : filled() ? "2 tokens" : "No tokens");
const hubStatus = () => (phone() ? "On a computer" : !signedIn() ? "Needs a hub" : S.relay ? "On" : "Off");

function projectsBlock() {
  const off = !everything() || !S.access;
  const why = !everything() ? "Needs “Everything” above." : !S.access ? "Turn on Computer access to use projects." : "";
  const rows = [item({ glyph: "house", name: "Home", sub: "C:\\Users\\sam", mono: true, tags: ` <span class="tag">Default</span>`, acts: iconBtn("pencil", "Rename Home") })];
  if (filled()) rows.push(item({ glyph: "folder", name: "scratch", sub: "C:\\Users\\sam\\Projects\\scratch", mono: true, acts: iconBtn("pencil", "Rename scratch") + iconBtn("trash-2", "Remove scratch", { danger: true }) }),
    item({ glyph: "folder", name: "thesis", sub: "D:\\Writing\\thesis", mono: true, acts: iconBtn("pencil", "Rename thesis") + iconBtn("trash-2", "Remove thesis", { danger: true }) }));
  const form = S.edit === "project" ? `<div class="editor"><h4>Add a project</h4>
    <label class="field-label">Folder</label><div class="inline"><input class="field mono" placeholder="C:\\Users\\sam\\Projects\\website">${btn("Browse…", { icon: "folder-open" })}</div>
    <label class="field-label">Name</label><input class="field" placeholder="website"><div class="field-hint">Filled in from the folder name when you browse.</div>
    <div class="foot">${btn("Cancel", { kind: "quiet", act: "edit=" })}${btn("Add project", { kind: "primary" })}</div></div>` : "";
  const list = off ? "" : `<div class="list">${rows.join("")}${S.edit === "project" ? "" : `<div class="list-foot">${btn("Add project", { icon: "plus", act: "edit=project" })}${filled() ? "" : `<span class="desc" style="color:var(--ink3);font-size:12.5px">Only Home so far. Add a folder to let the agent work there.</span>`}</div>`}</div>${form}`;
  return row({ label: "Projects", desc: "Folders the agent can work in. In a chat, the agent starts in one of them and runs its commands there.",
    help: help("Add a folder when you want the agent to work on its files, such as a code repository or a writing project."), stack: true, disabled: off, reason: why, ctl: list ? `<div style="width:100%">${list}</div>` : "" });
}

function toolsBody() {
  if (phone()) return `<p class="desc" style="color:var(--ink2);margin:4px 0 0">Set these from Scratch on a computer. Phones always use the hub.</p>`;
  const off = !everything();
  const servers = [
    item({ glyph: "terminal", name: "filesystem", tags: ` <span class="tag">Program</span>`, status: status("ok", "Running"), sub: "npx -y @modelcontextprotocol/server-filesystem D:\\Writing", mono: true,
      acts: swv(true, "filesystem on") + iconBtn("pencil", "Edit filesystem") + iconBtn("trash-2", "Remove filesystem", { danger: true }) }),
    item({ glyph: "globe", name: "docs-search", tags: ` <span class="tag">URL</span>`, status: status("bad", "Failed"), sub: "https://example.com/mcp", mono: true,
      err: "The server refused the connection (401). Check the API key header.", acts: `<button class="switch" role="switch" aria-checked="true" aria-label="docs-search on"></button>` + iconBtn("pencil", "Edit docs-search") + iconBtn("trash-2", "Remove docs-search", { danger: true }) }),
    item({ glyph: "terminal", name: "sqlite", tags: ` <span class="tag">Program</span>`, status: status("off", "Off"), sub: "uvx mcp-server-sqlite --db-path D:\\data\\shop.db", mono: true,
      acts: `<button class="switch" role="switch" aria-checked="false" aria-label="sqlite on"></button>` + iconBtn("pencil", "Edit sqlite") + iconBtn("trash-2", "Remove sqlite", { danger: true }) }),
  ];
  const form = S.edit === "server" ? `<div class="editor"><h4>Add an MCP server</h4>
    <label class="field-label">How it runs</label>${seg([["cmd", "A program on this computer"], ["url", "A web address"]], "cmd")}
    <div class="two"><div><label class="field-label">Name</label><input class="field" placeholder="filesystem"></div>
      <div><label class="field-label">Command</label><input class="field mono" placeholder="npx"></div></div>
    <label class="field-label">Arguments, one per line</label><textarea class="field mono" placeholder="-y&#10;@example/mcp-server"></textarea>
    <label class="field-label">Environment variables, KEY=value per line <span style="color:var(--ink3);font-weight:500">(optional)</span></label><textarea class="field mono" placeholder="EXAMPLE_ROOT=/path"></textarea>
    <div class="field-hint">Copy these from the server’s instructions; they usually give a command like <span class="mono">npx -y @org/server</span>.</div>
    <div class="foot">${btn("Cancel", { kind: "quiet", act: "edit=" })}${btn("Add server", { kind: "primary" })}</div></div>` : "";
  const body = off ? "" : filled()
    ? `<div class="list">${servers.join("")}${S.edit === "server" ? "" : `<div class="list-foot">${btn("Add server", { icon: "plus", act: "edit=server" })}</div>`}</div>${form}`
    : (S.edit === "server" ? form : empty("blocks", "No MCP servers", "MCP servers are small programs or web services that give the agent extra tools, such as reading a database or searching documentation. Their setup instructions give you a command or a URL to add here.", btn("Add server", { icon: "plus", act: "edit=server" })));
  return row({ label: "MCP servers", desc: "Extra tools for the agent, from programs on this computer or from web addresses. Changes apply at once.",
    help: help("Add one when a tool or service offers an MCP server and you want the agent to use it."), stack: true, disabled: off,
    reason: off ? 'Needs “Everything” on the Agent page. Servers don’t start with Read or Write. <button class="linkish" data-page="agent">Change on the Agent page</button>' : "", ctl: body ? `<div style="width:100%">${body}</div>` : "" });
}

// ---- shell ----
const phone = () => document.documentElement.classList.contains("phone");

function draw() {
  const root = document.documentElement;
  root.dataset.theme = S.theme;
  root.classList.toggle("phone", innerWidth < 720);
  root.classList.toggle("at-list", phone() && S.phoneList);
  root.classList.toggle("review-hidden", !S.chrome);
  root.classList.toggle("tall", S.tall);
  const nav = document.querySelector(".nav");
  nav.innerHTML = `<h1>Settings</h1>` + PAGES.map((g) => (g.group ? `<h2>${g.group}</h2>` : "") + g.items.map(([id, label, icon]) => {
    const v = navValue[id]?.();
    const count = id === "conflicts" && filled() ? `<span class="count" aria-label="2 conflicts">2</span>` : "";
    return `<button data-page="${id}" ${id === S.page ? 'aria-current="page"' : ""}>${ic(icon)}<span>${label}</span>${count}${phone() && v ? `<span class="value">${v}</span>` : ""}</button>`;
  }).join("")).join("");
  const main = document.querySelector(".main");
  const top = main.scrollTop;
  main.innerHTML = `<div class="page"><div class="crumb"><button data-act="list">${ic("chevron-left")}Settings</button></div>${render[S.page]()}</div>`;
  applyHelp();
  main.scrollTop = top;
  for (const el of document.querySelectorAll(".review select")) el.value = el.name in S ? (BOOL.includes(el.name) ? (S[el.name] ? "1" : "0") : S[el.name]) : el.value;
  window.lucide?.createIcons({ attrs: { "stroke-width": 1.75 } });
  writeHash();
}

// ---- help notes: hover or keyboard focus peeks one open; a click pins it ----
// Pointer peeks close 250 ms after the pointer leaves the ⓘ button and its note, so moving onto the note does not flicker.
// Touch has no hover: a tap is a click, which toggles the pin.
const PINNED = new Set();              // notes pinned open by a click
const PEEK = { hover: "", focus: "" }; // the note under the pointer, or the one whose ⓘ button has focus
let hoverTimer = 0;
function applyHelp() {
  const shown = (id) => S.help || PINNED.has(id) || PEEK.hover === id || PEEK.focus === id || S.hoverHelp === id;
  for (const b of document.querySelectorAll(".help-btn")) {
    const on = shown(b.dataset.hz);
    b.setAttribute("aria-expanded", on);
    document.getElementById(b.getAttribute("aria-controls"))?.classList.toggle("open", on);
  }
  const all = document.querySelector('.review select[name="help"]');
  if (all) all.value = S.help ? "1" : "0";
}
const zoneOf = (e) => e.target.closest?.("[data-hz]");
document.addEventListener("pointerover", (e) => {
  if (e.pointerType === "touch") return;
  const z = zoneOf(e);
  if (z) { clearTimeout(hoverTimer); PEEK.hover = z.dataset.hz; applyHelp(); }
});
document.addEventListener("pointerout", (e) => {
  if (e.pointerType === "touch") return;
  const z = zoneOf(e);
  if (!z || e.relatedTarget?.closest?.("[data-hz]")?.dataset.hz === z.dataset.hz) return; // still on the button or its note
  const id = z.dataset.hz;
  clearTimeout(hoverTimer);
  hoverTimer = setTimeout(() => { if (PEEK.hover === id) { PEEK.hover = ""; applyHelp(); } }, 250);
});
document.addEventListener("focusin", (e) => {
  const b = e.target.closest?.(".help-btn");
  if (b) { PEEK.focus = b.dataset.hz; applyHelp(); }
});
document.addEventListener("focusout", (e) => {
  if (e.target.closest?.(".help-btn")) { PEEK.focus = ""; applyHelp(); }
});
document.addEventListener("toggle", (e) => {
  const d = e.target;
  if (d.matches?.("details.adv")) (d.open ? ADV_OPEN.add(d.dataset.adv) : ADV_OPEN.delete(d.dataset.adv));
}, true);

document.addEventListener("click", (e) => {
  const t = e.target.closest("button, summary");
  if (!t) return;
  if (t.dataset.page) { S.page = t.dataset.page; S.edit = ""; S.open = ""; S.phoneList = false; document.querySelector(".main").scrollTop = 0; return draw(); }
  if (t.dataset.toggle) { const k = t.dataset.toggle; if (k in S) { S[k] = !S[k]; draw(); } else t.setAttribute("aria-checked", t.getAttribute("aria-checked") !== "true"); return; }
  if (t.classList.contains("switch")) { t.setAttribute("aria-checked", t.getAttribute("aria-checked") !== "true"); return; }
  if (t.dataset.cpanel) {
    const id = t.dataset.cpanel, list = S.open.split(",").filter(Boolean);
    S.open = (list.includes(id) ? list.filter((x) => x !== id) : [...list, id]).join(",");
    draw();
    document.querySelector(`[data-cpanel="${id}"]`)?.focus();
    return;
  }
  if (t.classList.contains("help-btn")) {
    const id = t.dataset.hz;
    if (S.help) { for (const b of document.querySelectorAll(".help-btn")) if (b.dataset.hz !== id) PINNED.add(b.dataset.hz); S.help = false; } // from "all open", closing one keeps the others open
    else if (PINNED.has(id)) PINNED.delete(id); else PINNED.add(id);
    S.hoverHelp = ""; PEEK.hover = ""; PEEK.focus = "";
    applyHelp(); writeHash(); return;
  }
  const set = t.dataset.set || t.dataset.act;
  if (set === "list") { S.phoneList = true; return draw(); }
  if (set && set.includes("=")) { let [k, v] = set.split("="); if (k === "theme" && v === "system") v = matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light"; S[k] = v; if (k === "page") { S.edit = ""; S.open = ""; } return draw(); }
  if (t.parentElement?.classList.contains("seg")) { for (const b of t.parentElement.children) b.setAttribute("aria-pressed", b === t); }
});
document.addEventListener("change", (e) => {
  const el = e.target;
  if (el.closest(".review")) { if (el.name === "help") PINNED.clear(); S[el.name] = BOOL.includes(el.name) ? el.value === "1" : el.value; draw(); }
});
addEventListener("resize", draw);
addEventListener("hashchange", () => { readHash(); draw(); });
readHash();
draw();
