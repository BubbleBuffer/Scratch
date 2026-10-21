// Screenshots for the search-card mockup. Serve this folder on 8797 first:
//   python -m http.server 8797 --directory design/search-card
const path = require("node:path");
const { launch, newPage } = require("../directions/playwright.cjs");

const base = "http://localhost:8797/index.html";
const out = path.join(__dirname, "shots");
const shots = [];
for (const size of ["desktop", "mobile"]) {
  for (const theme of ["light", "dark"]) {
    shots.push({ v: "a", s: "live", size, theme });
    shots.push({ v: "b", s: "open", size, theme });
  }
  for (const v of ["a", "b"]) for (const s of ["edit", "ticked", "empty"]) shots.push({ v, s, size, theme: "light" });
}
shots.push({ v: "b", s: "live", size: "desktop", theme: "light" });

(async () => {
  const browser = await launch();
  try {
    for (const x of shots) {
      const page = await newPage(browser, x.size, x.theme);
      await page.goto(`${base}?clean&v=${x.v}&s=${x.s}&theme=${x.theme}`, { waitUntil: "networkidle" });
      await page.evaluate(() => document.fonts.ready);
      await page.waitForTimeout(150);
      const file = path.join(out, `${x.v}-${x.s}-${x.size}-${x.theme}.png`);
      await page.screenshot({ path: file, fullPage: true });
      console.log(path.relative(__dirname, file));
      await page.context().close();
    }
  } finally {
    await browser.close();
  }
})();
