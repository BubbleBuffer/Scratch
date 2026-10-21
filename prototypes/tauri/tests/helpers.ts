import { expect, test, type Locator, type Page } from "@playwright/test";

export const isMobileProject = () => test.info().project.name === "mobile";

export async function openHome(page: Page) {
  await page.goto("/");
  await expect(page.locator('[data-testid="page-row"] .t').first()).toBeVisible({ timeout: 60_000 });
}

export const blocks = (page: Page) => page.locator(".blocks .blk");
export const blockText = (page: Page, i: number) => blocks(page).nth(i).locator(".txt");

/** Texts of all blocks, in order (textContent: innerText is empty for
 *  blocks that content-visibility has not laid out yet). */
export async function texts(page: Page) {
  return blocks(page).locator(".txt").evaluateAll((els) => els.map((e) => e.textContent ?? ""));
}
export async function kinds(page: Page) {
  return blocks(page).evaluateAll((els) => els.map((e) => e.className.split(" ")[1]));
}

/** Creates a page from the home screen, types its title and moves to the first block. */
export async function newPage(page: Page, title: string) {
  await page.locator('[data-testid="new-page"]:visible').click();
  await expect(page.getByTestId("editor")).toBeVisible();
  await expect(blocks(page)).toHaveCount(1);
  const t = page.getByTestId("title");
  await t.click();
  await page.keyboard.type(title);
  await page.keyboard.press("Enter");
  await expect(blocks(page).first()).toHaveClass(/focus/);
}

/** Opens a page by searching for its title from the home screen. */
export async function openByTitle(page: Page, title: string) {
  const search = page.locator('[data-testid="search"]:visible');
  await search.fill(title);
  const hit = page.locator('[data-testid="search-row"]', { hasText: title }).first();
  await expect(hit).toBeVisible();
  await hit.click();
  await expect(page.getByTestId("editor")).toBeVisible();
  await expect(blocks(page).first()).toBeVisible();
}

/** Waits until every change has been acknowledged by the core. */
export async function saved(page: Page) {
  await expect(page.getByTestId("save-state")).toHaveAttribute("data-state", "Saved");
}

/** A touch long-press through the DevTools protocol (headless, no OS input). */
export async function longPress(page: Page, target: Locator, ms = 650) {
  const box = (await target.boundingBox())!;
  const cdp = await page.context().newCDPSession(page);
  const x = box.x + Math.min(40, box.width / 2), y = box.y + box.height / 2;
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x, y }] });
  await page.waitForTimeout(ms);
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await cdp.detach();
}

/** Simulates the on-screen keyboard (headless Chromium has none). */
export async function showKeyboard(page: Page, px = 336) {
  await page.evaluate((h) => (window as any).__bakeoffKeyboard(h), px);
}

/** Selects the last `n` characters before the caret with the keyboard. */
export async function selectBack(page: Page, n: number) {
  for (let i = 0; i < n; i++) await page.keyboard.press("Shift+ArrowLeft");
}
