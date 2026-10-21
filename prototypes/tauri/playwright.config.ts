// Headless tests of the Tauri prototype's interface: the Vite build runs in
// headless Chromium against the test bridge (src-tauri/examples/bridge.rs),
// which serves dist/ and calls the real vault_core. No window is ever shown.
//
//   npm run build
//   cargo build -p notes-tauri --release --example bridge   (from the repo root)
//   npx playwright test
import { defineConfig } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
import os from "node:os";

const root = path.resolve(here, "../..");
const exe = process.platform === "win32" ? ".exe" : "";
const bridge = path.join(root, "target/release/examples/bridge" + exe);
const data = process.env.BRIDGE_DATA ?? path.join(root, "target/tauri-bridge-data");
const shotsData = process.env.BRIDGE_SHOTS_DATA ?? path.join(root, "target/tauri-bridge-shots");
const PORT = 4480;
export const SHOTS_PORT = 4481;

// Playwright's own Chromium if installed; otherwise the headless shell that
// is already on this machine (set CHROMIUM to override).
const pwDir = path.join(os.homedir(), "AppData/Local/ms-playwright");
const executablePath = process.env.CHROMIUM ??
  (process.platform === "win32" ? path.join(pwDir, "chromium_headless_shell-1234/chrome-headless-shell-win64/chrome-headless-shell.exe") : undefined);

const android = "Mozilla/5.0 (Linux; Android 15; sdk_gphone64_x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Mobile Safari/537.36";

export default defineConfig({
  testDir: "tests",
  timeout: 60_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  workers: 1,
  reporter: [["list"]],
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    headless: true,
    launchOptions: { executablePath },
    trace: "off",
  },
  // Two bridges: one vault for the interaction tests (they add pages), one
  // for the screenshots (left as on first launch, apart from edit times).
  webServer: [
    { port: PORT, data },
    { port: SHOTS_PORT, data: shotsData },
  ].map((w) => ({
    command: `"${bridge}" --port ${w.port} --data "${w.data}" --dist "${path.join(here, "dist")}"`,
    url: `http://127.0.0.1:${w.port}/`,
    reuseExistingServer: true,
    timeout: 180_000,
    stdout: "ignore" as const,
  })),
  projects: [
    { name: "desktop", use: { viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 } },
    {
      name: "mobile",
      use: {
        viewport: { width: 375, height: 812 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true, userAgent: android,
        browserName: "chromium",
      },
    },
  ],
});

