// Editor and home interactions (both projects unless marked).
import { expect, test } from "@playwright/test";
import {
  blockText, blocks, isMobileProject, kinds, longPress, newPage, openByTitle, openHome, saved, selectBack, showKeyboard, texts,
} from "./helpers";

test.beforeEach(async ({ page }) => { await openHome(page); });

test("home lists every page, search filters, the showcase exists", async ({ page }) => {
  const rows = page.locator('[data-testid="page-row"]');
  await expect(rows.first()).toBeVisible();
  // The list is virtualized: far fewer rows in the DOM than pages.
  expect(await rows.count()).toBeLessThan(80);
  const height = await page.locator(".vrows").evaluate((e) => e.getBoundingClientRect().height);
  expect(height).toBeGreaterThan(10_000 * (isMobileProject() ? 44 : 36));
  await page.locator('[data-testid="search"]:visible').fill("Bake-off showcase");
  await expect(page.locator('[data-testid="search-row"]', { hasText: "Bake-off showcase" }).first()).toBeVisible();
  await page.locator('[data-testid="search"]:visible').fill("Big page");
  await expect(page.locator('[data-testid="search-row"]', { hasText: "Big page" }).first()).toBeVisible();
  await page.locator('[data-testid="search"]:visible').fill("");
  await expect(rows.first()).toBeVisible();
});

test("typing, Enter splits, Backspace merges", async ({ page }) => {
  await newPage(page, "Split and merge");
  await page.keyboard.type("Hello world");
  for (let i = 0; i < 5; i++) await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("Enter");
  expect(await texts(page)).toEqual(["Hello ", "world"]);
  await expect(blocks(page).nth(1)).toHaveClass(/focus/);
  await page.keyboard.press("Backspace");
  expect(await texts(page)).toEqual(["Hello world"]);
  await page.keyboard.type("big ");
  expect(await texts(page)).toEqual(["Hello big world"]);
});

test("markdown shortcuts turn blocks into every kind", async ({ page }) => {
  await newPage(page, "Markdown shortcuts");
  const steps: [string, string][] = [
    ["# ", "h1"], ["## ", "h2"], ["### ", "h3"], ["- ", "bul"], ["1. ", "num"], ["[] ", "todo"], ["> ", "quote"], ["```", "code"],
  ];
  for (const [i, [md, cls]] of steps.entries()) {
    await page.keyboard.type(md);
    await expect(blocks(page).nth(i)).toHaveClass(new RegExp(`\\b${cls}\\b`));
    await page.keyboard.type(`${cls} text`);
    if (cls === "code") break;
    await page.keyboard.press("Enter");
    // Enter on an empty list item ends the list: make sure each step starts as text.
    const k = (await kinds(page))[i + 1];
    if (k !== "p") { await page.keyboard.press("Backspace"); }
  }
  expect(await kinds(page)).toEqual(["h1", "h2", "h3", "bul", "num", "todo", "quote", "code"]);
  // `---` makes a divider.
  await blocks(page).nth(0).locator(".txt").click();
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await page.keyboard.type("---");
  expect((await kinds(page))[1]).toBe("div");
});

test("Tab and Shift+Tab indent and outdent list items", async ({ page }) => {
  await newPage(page, "Indenting");
  await page.keyboard.type("- one");
  await page.keyboard.press("Enter");
  await page.keyboard.type("two");
  await page.keyboard.press("Tab");
  const depth = (i: number) => blocks(page).nth(i).evaluate((e) => getComputedStyle(e).getPropertyValue("--depth").trim());
  expect(await depth(1)).toBe("1");
  await page.keyboard.type("!");
  expect(await texts(page)).toEqual(["one", "two!"]);
  await page.keyboard.press("Shift+Tab");
  expect(await depth(1)).toBe("0");
});

test("slash menu opens, filters, picks with the keyboard", async ({ page }) => {
  await newPage(page, "Slash menu");
  if (isMobileProject()) await showKeyboard(page);
  await page.keyboard.type("/");
  const menu = page.getByTestId("slash-menu");
  await expect(menu).toBeVisible();
  await expect(menu.locator(".item")).toHaveCount(10);
  await page.keyboard.type("head");
  await expect(menu.locator(".item")).toHaveCount(3);
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(menu).toBeHidden();
  await expect(blocks(page).first()).toHaveClass(/\bh2\b/);
  expect(await texts(page)).toEqual([""]);
  await page.keyboard.type("/zzz");
  await expect(menu.locator(".none")).toHaveText("No results");
  await page.keyboard.press("Escape");
  await expect(menu).toBeHidden();
});

test("selection toolbar and shortcuts format text (desktop)", async ({ page }) => {
  test.skip(isMobileProject(), "desktop only");
  await newPage(page, "Formatting");
  await page.keyboard.type("make this bold");
  await selectBack(page, 4);
  const bar = page.getByTestId("selection-toolbar");
  await expect(bar).toBeVisible();
  await bar.getByLabel("Bold").click();
  await expect(blockText(page, 0).locator("b")).toHaveText("bold");
  await page.keyboard.press("End");
  await page.keyboard.type(" and italic");
  await selectBack(page, 6);
  await page.keyboard.press("Control+i");
  await expect(blockText(page, 0).locator("i")).toHaveText("italic");
  await selectBack(page, 0);
  await page.keyboard.press("Home");
  await page.keyboard.press("Shift+End");
  await expect(bar).toBeVisible();
  await bar.getByLabel("Strikethrough").click();
  await bar.getByLabel("Inline code").click();
  // Marks split at other marks' edges, so check the struck text as a whole.
  const joined = (tag: string) => blockText(page, 0).locator(tag).evaluateAll((els) => els.map((e) => e.textContent).join(""));
  expect(await joined("s")).toBe("make this bold and italic");
  expect(await joined("code")).toBe("make this bold and italic");
  // Link through the toolbar.
  await page.keyboard.press("End");
  await page.keyboard.type(" site");
  await selectBack(page, 4);
  await bar.getByLabel("Link").click();
  await page.getByTestId("link-popover").locator("input").fill("example.com");
  await page.keyboard.press("Enter");
  await expect(blockText(page, 0).locator("a")).toHaveAttribute("data-href", "https://example.com");
});

test("keyboard toolbar formats text and changes blocks (mobile)", async ({ page }) => {
  test.skip(!isMobileProject(), "mobile only");
  await newPage(page, "Keyboard toolbar");
  await showKeyboard(page);
  const bar = page.getByTestId("keyboard-toolbar");
  await expect(bar).toBeVisible();
  await page.keyboard.type("make this bold");
  await selectBack(page, 4);
  await bar.getByLabel("Text styles").click();
  await bar.getByLabel("Bold").click();
  await expect(blockText(page, 0).locator("b")).toHaveText("bold");
  await bar.getByLabel("Italic").click();
  await expect(blockText(page, 0).locator("i")).toHaveText("bold");
  await bar.getByLabel("Text styles").click();
  await bar.getByLabel("To-do").click();
  await expect(blocks(page).first()).toHaveClass(/\btodo\b/);
  await bar.getByLabel("Bulleted list").click();
  await expect(blocks(page).first()).toHaveClass(/\bbul\b/);
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await page.keyboard.type("child");
  await bar.getByLabel("Indent").click();
  expect(await blocks(page).nth(1).evaluate((e) => getComputedStyle(e).getPropertyValue("--depth").trim())).toBe("1");
  await bar.getByLabel("Outdent").click();
  expect(await blocks(page).nth(1).evaluate((e) => getComputedStyle(e).getPropertyValue("--depth").trim())).toBe("0");
});

test("to-do checkboxes toggle with a tap", async ({ page }) => {
  await newPage(page, "To-do");
  await page.keyboard.type("[] buy milk");
  const b = blocks(page).first();
  await expect(b).toHaveClass(/\btodo\b/);
  const box = b.locator('[data-act="check"]');
  if (isMobileProject()) await box.tap(); else await box.click();
  await expect(b).toHaveClass(/\bdone\b/);
  if (isMobileProject()) await box.tap(); else await box.click();
  await expect(b).not.toHaveClass(/\bdone\b/);
});

test("drag handle moves blocks and opens the block menu (desktop)", async ({ page }) => {
  test.skip(isMobileProject(), "desktop only");
  await newPage(page, "Moving blocks");
  for (const t of ["one", "two", "three"]) { await page.keyboard.type(t); await page.keyboard.press("Enter"); }
  await page.keyboard.type("four");
  const first = blocks(page).nth(0), third = blocks(page).nth(2);
  await first.hover();
  const grip = page.getByTestId("grip");
  await expect(grip).toBeVisible();
  const g = (await grip.boundingBox())!;
  const t = (await third.boundingBox())!;
  await page.mouse.move(g.x + g.width / 2, g.y + g.height / 2);
  await page.mouse.down();
  await page.mouse.move(g.x + 5, g.y + 20, { steps: 4 });
  await page.mouse.move(t.x + 80, t.y + t.height - 4, { steps: 8 });
  await expect(page.locator(".dropline")).toBeVisible();
  await page.mouse.up();
  expect(await texts(page)).toEqual(["two", "three", "one", "four"]);
  // Click the grip: block menu, then "Move up".
  await blocks(page).nth(3).hover();
  await page.getByTestId("grip").click();
  const menu = page.getByTestId("block-menu");
  await expect(menu).toBeVisible();
  await menu.getByRole("button", { name: "Move up" }).click();
  expect(await texts(page)).toEqual(["two", "three", "four", "one"]);
  await blocks(page).nth(0).hover();
  await page.getByTestId("grip").click();
  await menu.getByRole("button", { name: "Turn into" }).click();
  await menu.getByRole("button", { name: "Quote" }).click();
  await expect(blocks(page).nth(0)).toHaveClass(/\bquote\b/);
  await blocks(page).nth(1).hover();
  await page.getByTestId("grip").click();
  await menu.getByRole("button", { name: "Delete" }).click();
  expect(await texts(page)).toEqual(["two", "four", "one"]);
});

test("long-press opens the block sheet: move, turn into, delete (mobile)", async ({ page }) => {
  test.skip(!isMobileProject(), "mobile only");
  await newPage(page, "Block sheet");
  for (const t of ["one", "two"]) { await page.keyboard.type(t); await page.keyboard.press("Enter"); }
  await page.keyboard.type("three");
  await page.evaluate(() => (document.activeElement as HTMLElement).blur());
  const sheet = page.getByTestId("block-sheet");
  await longPress(page, blocks(page).nth(0));
  await expect(sheet).toHaveClass(/\bon\b/);
  await sheet.getByRole("button", { name: "Move down" }).click();
  // The sheet stays open for further moves; the back gesture closes it.
  expect(await texts(page)).toEqual(["two", "one", "three"]);
  await page.goBack();
  await expect(sheet).toBeHidden();
  await longPress(page, blocks(page).nth(2));
  await sheet.getByRole("button", { name: "Turn into" }).click();
  await sheet.getByRole("button", { name: "Heading 1" }).click();
  await expect(blocks(page).nth(2)).toHaveClass(/\bh1\b/);
  await longPress(page, blocks(page).nth(0));
  await sheet.getByRole("button", { name: "Delete" }).click();
  expect(await texts(page)).toEqual(["one", "three"]);
  // The back gesture closes the sheet, not the page.
  await longPress(page, blocks(page).nth(0));
  await expect(sheet).toHaveClass(/\bon\b/);
  await page.goBack();
  await expect(sheet).toBeHidden();
  await expect(page.getByTestId("editor")).toBeVisible();
});

test("edits persist across back navigation and a reload", async ({ page }) => {
  const title = `Persist ${Date.now()}`;
  await newPage(page, title);
  await page.keyboard.type("- first");
  await page.keyboard.press("Enter");
  await page.keyboard.type("second");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Backspace");
  await page.keyboard.type("[] done");
  await blocks(page).nth(2).locator('[data-act="check"]').click();
  await saved(page);
  // Back button returns to the list; the page is at the top.
  await page.getByTestId("back").click();
  await expect(page.getByTestId("home")).toBeVisible();
  await expect(page.getByTestId("editor")).toHaveCount(0);
  await expect(page.locator('[data-testid="page-row"]').first()).toContainText(title);
  // Reload (a restart of the interface; the core keeps running) and reopen.
  await page.reload();
  await openHome(page);
  await openByTitle(page, title);
  expect(await texts(page)).toEqual(["first", "second", "done"]);
  expect(await kinds(page)).toEqual(["bul", "bul", "todo"]);
  await expect(blocks(page).nth(2)).toHaveClass(/\bdone\b/);
  expect(await blocks(page).nth(1).evaluate((e) => getComputedStyle(e).getPropertyValue("--depth").trim())).toBe("1");
  // Browser back (the Android back gesture) returns to the list too.
  await page.goBack();
  await expect(page.getByTestId("editor")).toHaveCount(0);
});

test("the 2,000-block big page opens, scrolls and takes typing", async ({ page }) => {
  await openByTitle(page, "Big page");
  // Rendered progressively, a chunk per frame; wait for all of it.
  await expect.poll(() => blocks(page).count(), { timeout: 20_000 }).toBeGreaterThanOrEqual(2000);
  const scroller = page.locator(".editor .scroller");
  await scroller.evaluate((e) => { e.scrollTop = e.scrollHeight / 2; });
  const target = blocks(page).nth(1000).locator(".txt");
  await target.scrollIntoViewIfNeeded();
  const kind = await blocks(page).nth(1000).evaluate((e) => e.className);
  test.skip(/\bdiv\b/.test(kind), "block 1000 is a divider");
  if (isMobileProject()) await target.tap(); else await target.click();
  await page.keyboard.press("End");
  await page.keyboard.type(" typed");
  await expect(target).toContainText("typed");
  for (let i = 0; i < 6; i++) await page.keyboard.press("Backspace");
  await saved(page);
});
