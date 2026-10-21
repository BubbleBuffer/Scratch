// Headless Chromium for design-direction captures. Uses the Playwright install
// in prototypes/tauri and the local headless shell (override with CHROMIUM).
// Moved here when the reference captures
// were removed from the repo (2026-10-13).
const path = require("node:path");
const os = require("node:os");
const { createRequire } = require("node:module");

const root = path.resolve(__dirname, "../..");
const req = createRequire(path.join(root, "prototypes/tauri/package.json"));
const { chromium } = req("playwright");

const executablePath = process.env.CHROMIUM ?? path.join(os.homedir(),
  "AppData/Local/ms-playwright/chromium_headless_shell-1234/chrome-headless-shell-win64/chrome-headless-shell.exe");

const android = "Mozilla/5.0 (Linux; Android 15; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Mobile Safari/537.36";

const SIZES = {
  desktop: { viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 },
  mobile: { viewport: { width: 375, height: 812 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true, userAgent: android },
};

async function launch() {
  return chromium.launch({ headless: true, executablePath });
}

async function newPage(browser, size = "desktop", scheme = "light") {
  const ctx = await browser.newContext({ ...SIZES[size], colorScheme: scheme, locale: "en-US" });
  return ctx.newPage();
}

module.exports = { launch, newPage, SIZES, executablePath };
