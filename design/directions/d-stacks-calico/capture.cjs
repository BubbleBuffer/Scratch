// Headless-only acceptance captures. No server and no external dependencies added.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const assert = require('node:assert/strict');
const { launch } = require('../playwright.cjs');
const root = __dirname;
const url = pathToFileURL(path.join(root, 'index.html')).href;
const shots = path.join(root, 'shots');
const shotsOnly = process.argv.includes('--shots-only');
const resultFile = path.join(root, shotsOnly ? 'capture-results.json' : 'verification.json');
fs.mkdirSync(shots, { recursive: true });
const result = { shots: [], checks: [], consoleErrors: [], pageErrors: [] };
let navigation = 0;
const wait = page => page.waitForTimeout(1600);
async function open(page, calico, fun='off', frame='d', theme='light', extra={}) {
  const hash = new URLSearchParams({ calico, fun, frame, theme, ...extra });
  await page.goto(url + '?capture=' + (++navigation) + '#' + hash, { waitUntil: 'load' });
  await page.evaluate(() => document.fonts.ready);
  await wait(page);
}
async function snap(page, name) {
  await page.mouse.move(0, 0);
  await page.waitForTimeout(250);
  await page.screenshot({ path: path.join(shots, name + '.png') });
  result.shots.push(name + '.png');
}
async function lisbon(page) {
  const tile = page.locator('.sheet:not(.covered) .home-spaces [data-open="lisbon"]');
  if (await tile.isVisible()) await tile.click();
  else await page.locator('.sheet:not(.covered) .fan [data-open="lisbon"]').click();
  await wait(page);
  // Keep the Home edge at its initial position for comparable captures.
  await page.locator('.sheet[data-id="home"] .sheet-body').evaluate(e => e.scrollTop = 0);
  assert.equal(await page.locator('.sheet:not(.covered) .ptitle').textContent(), 'Lisbon in May');
}
async function stack3(page) {
  await page.locator('.sheet:not(.covered) .mention[data-open="weekly"]').click();
  await wait(page);
  assert.equal(await page.locator('#desk > .sheet').count(), 3);
  assert.equal(await page.locator('.sheet:not(.covered) .ptitle').textContent(), 'Weekly review');
}
async function interactionCheck(page, calico, frame, theme) {
  await open(page, calico, 'off', frame, theme);
  await lisbon(page);
  const top = () => page.locator('.sheet:not(.covered)');
  await top().locator('[data-act="fold"]').click();
  await wait(page);
  assert.equal(await top().locator('#fold1').isVisible(), true);
  await top().locator('[data-act="fold"]').click();
  await wait(page);
  assert.equal(await top().locator('#fold1').isVisible(), false);
  await top().locator('[data-act="drawer"]').click();
  await wait(page);
  assert.equal(await top().locator('#drawer1').isVisible(), false);
  await top().locator('[data-act="drawer"]').click();
  await wait(page);
  assert.equal(await top().locator('#drawer1').isVisible(), true);
  await top().locator('[data-list="packing"] .open-list .tick').first().click();
  await wait(page);
  assert.equal(await top().locator('[data-ptext="packing"]').textContent(), '3 of 7 packed');
  await top().locator('[data-pile="packing"]').click();
  await wait(page);
  await top().locator('[data-list="packing"] .pile-list .tick').last().click();
  await wait(page);
  assert.equal(await top().locator('[data-ptext="packing"]').textContent(), '2 of 7 packed');
  await stack3(page);
  await page.locator('.sheet[data-id="lisbon"] > .spine').click();
  await wait(page);
  assert.equal(await page.locator('#desk > .sheet').count(), 2);
  if (frame === 'm') {
    await page.locator('.mb-page [data-act="plus"]').click();
  } else {
    await top().locator('.stop [data-act="plus"]').click();
  }
  await wait(page);
  assert.equal(await page.locator('#menu').isVisible(), true);
  await page.keyboard.press('Escape');
  await wait(page);
  assert.equal(await page.locator('#menu').isVisible(), false);
  if (frame === 'm') await page.locator('.mb-back').click();
  else await page.locator('.sheet[data-id="home"] > .spine').click();
  await wait(page);
  assert.equal(await page.locator('#desk > .sheet').count(), 1);
  const overflow = await page.evaluate(() => {
    const s = document.querySelector('.sheet:not(.covered) .sheet-body');
    return s.scrollWidth > s.clientWidth + 1;
  });
  assert.equal(overflow, false);
  result.checks.push(`${calico}/${frame}/${theme}: navigation, 3 sheets, spines/back, tick/undo, fold, drawer, menu/Escape, horizontal fit passed`);
}
(async () => {
  const browser = await launch();
  try {
    const context = await browser.newContext({ viewport: { width: 1440, height: 944 }, deviceScaleFactor: 1, colorScheme: 'light' });
    const page = await context.newPage();
    page.on('console', msg => { if (msg.type() === 'error') result.consoleErrors.push(msg.text()); });
    page.on('pageerror', error => result.pageErrors.push(error.message));
    for (const calico of ['quiet', 'tabs', 'patches']) {
      await open(page, calico);
      await snap(page, `${calico}-desktop-light-home`);
      await lisbon(page);
      await snap(page, `${calico}-desktop-light-page`);
      await stack3(page);
      await snap(page, `${calico}-desktop-light-stack3`);
    }
    for (const calico of ['tabs', 'patches']) {
      await open(page, calico, 'off', 'd', 'dark');
      await snap(page, `${calico}-desktop-dark-home`);
      await lisbon(page);
      await snap(page, `${calico}-desktop-dark-page`);
    }
    await page.setViewportSize({ width: 390, height: 844 });
    for (const calico of ['tabs', 'patches']) {
      await open(page, calico, 'off', 'm');
      await snap(page, `${calico}-mobile-light-home`);
      await lisbon(page);
      await snap(page, `${calico}-mobile-light-page`);
    }
    await page.setViewportSize({ width: 1440, height: 944 });
    await open(page, 'tabs', 'on');
    await snap(page, 'fun-desktop-light-home');
    await page.locator('#proto [data-act="blank"]').click();
    await wait(page);
    await snap(page, 'fun-desktop-light-empty');
    await open(page, 'tabs', 'on');
    await lisbon(page);
    await page.locator('#packBlock').scrollIntoViewIfNeeded();
    while (await page.locator('#packBlock .open-list .tick').count() > 1) {
      await page.locator('#packBlock .open-list .tick').first().click();
      await wait(page);
    }
    await page.locator('#packBlock .open-list .tick').click();
    await page.waitForFunction(() => document.querySelectorAll('.confetti-piece').length === 14);
    await page.waitForTimeout(250);
    assert.equal(await page.locator('#packBlock [data-ptext="packing"]').textContent(), 'All packed');
    // Do not use snap's settling wait here: freeze the requested 250 ms moment.
    await page.evaluate(() => document.getAnimations().forEach(a => a.pause()));
    await page.screenshot({ path: path.join(shots, 'fun-desktop-light-checklist-done.png') });
    result.shots.push('fun-desktop-light-checklist-done.png');
    await page.evaluate(() => document.getAnimations().forEach(a => a.play()));
    result.checks.push('Fun completion: 14 pieces and All packed captured ~250 ms into the 900 ms animation');
    for (const frame of shotsOnly ? [] : ['d', 'm']) {
      await page.setViewportSize(frame === 'm' ? {width:390,height:844} : {width:1440,height:944});
      for (const calico of ['quiet', 'tabs', 'patches']) for (const theme of ['light', 'dark']) {
        await interactionCheck(page, calico, frame, theme);
      }
    }
    assert.deepEqual(result.pageErrors, []);
    assert.deepEqual(result.consoleErrors, []);
    fs.writeFileSync(resultFile, JSON.stringify(result, null, 2) + '\n');
    console.log(JSON.stringify(result, null, 2));
  } finally { await browser.close(); }
})().catch(error => {
  result.failure = error.stack;
  fs.writeFileSync(resultFile, JSON.stringify(result, null, 2) + '\n');
  console.error(error);
  process.exitCode = 1;
});
