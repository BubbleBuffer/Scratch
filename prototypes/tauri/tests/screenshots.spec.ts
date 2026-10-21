// Screenshots for the bake-off comparison, saved to screenshots/:
// <project>-<scheme>-<state>.png at 1440x900 (desktop) and 375x812 (mobile).
// The showcase page is left as it was (temporary edits are undone).
import { expect, test, type Page } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
import { blocks, isMobileProject, longPress, openHome, saved, showKeyboard } from "./helpers";

const dir = path.join(here, "..", "screenshots");
const shot = (page: Page, scheme: string, state: string) =>
  page.screenshot({ path: path.join(dir, `${test.info().project.name}-${scheme}-${state}.png`) });

async function openShowcase(page: Page) {
  await page.locator('[data-testid="page-row"]', { hasText: "Bake-off showcase" }).first().click();
  await expect(page.getByTestId("editor")).toBeVisible();
  await expect(blocks(page).first()).toBeVisible();
  await page.waitForTimeout(200);
}

/** The block whose text starts with `start`. */
const block = (page: Page, start: string) => blocks(page).filter({ has: page.locator(".txt", { hasText: start }) }).first();

for (const scheme of ["light", "dark"] as const) {
  test.describe(scheme, () => {
    test.use({ colorScheme: scheme, baseURL: "http://127.0.0.1:4481" });

    test(`screenshots ${scheme}`, async ({ page }) => {
      test.setTimeout(90_000);
      await openHome(page);
      await page.waitForTimeout(300);
      await shot(page, scheme, "home");

      await openShowcase(page);
      await page.mouse.move(0, 0);
      await shot(page, scheme, "showcase");

      if (!isMobileProject()) {
        // Selection toolbar over a word of the first paragraph.
        const p = block(page, "This page has").locator(".txt");
        await p.locator("b").first().dblclick();
        await expect(page.getByTestId("selection-toolbar")).toBeVisible();
        await shot(page, scheme, "selection-toolbar");

        // Block menu from the drag handle (the desktop counterpart of the sheet).
        await page.keyboard.press("Escape");
        await block(page, "A bulleted list item").hover();
        await page.getByTestId("grip").click();
        await expect(page.getByTestId("block-menu")).toBeVisible();
        await shot(page, scheme, "block-menu");
        await page.keyboard.press("Escape");
        await expect(page.getByTestId("block-menu")).toBeHidden();

        // Slash menu in a temporary block after "Headings come in…".
        const h = block(page, "Headings come in").locator(".txt");
        await h.click();
        await page.keyboard.press("End");
        await page.keyboard.press("Enter");
        await page.keyboard.type("/");
        await expect(page.getByTestId("slash-menu")).toBeVisible();
        await page.mouse.move(0, 0);
        await shot(page, scheme, "slash-menu");
        await page.keyboard.press("Escape");
        await page.keyboard.press("Backspace");
        await page.keyboard.press("Backspace");
      } else {
        // Keyboard toolbar with the caret in the first paragraph.
        const p = block(page, "This page has").locator(".txt");
        await p.tap();
        await showKeyboard(page);
        await expect(page.getByTestId("keyboard-toolbar")).toBeVisible();
        await page.locator(".editor .scroller").evaluate((e) => { e.scrollTop = 0; });
        await shot(page, scheme, "keyboard-toolbar");

        // Slash menu above the toolbar, in a temporary block.
        const h = block(page, "Headings come in").locator(".txt");
        await h.tap();
        await page.keyboard.press("End");
        await page.keyboard.press("Enter");
        await page.keyboard.type("/");
        await expect(page.getByTestId("slash-menu")).toBeVisible();
        await shot(page, scheme, "slash-menu");
        await page.keyboard.press("Escape");
        await page.keyboard.press("Backspace");
        await page.keyboard.press("Backspace");
        await page.evaluate(() => (document.activeElement as HTMLElement).blur());
        await showKeyboard(page, 0);
        await page.locator(".editor .scroller").evaluate((e) => { e.scrollTop = 0; });

        // Bottom sheet from a long-press.
        await longPress(page, block(page, "A bulleted list item"));
        const sheet = page.getByTestId("block-sheet");
        await expect(sheet).toHaveClass(/\bon\b/);
        await page.waitForTimeout(350);
        await shot(page, scheme, "bottom-sheet");
        await page.goBack();
        await expect(sheet).toBeHidden();
      }
      await saved(page);
      expect(await blocks(page).count()).toBe(19);
    });
  });
}
