'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { launch } = require('../playwright.cjs');
const { PNG } = require('../../../prototypes/tauri/node_modules/playwright-core/lib/utilsBundle.js');
const d = pathToFileURL(path.join(__dirname, 'index.html')).href;
const c = pathToFileURL(path.join(__dirname, '../c-stacks/index.html')).href;
const checks = [];
let navigation = 0;
const settle = async page => { await page.evaluate(() => document.fonts.ready); await page.waitForTimeout(1800); };
(async () => {
  const browser = await launch();
  try {
    const ctx = await browser.newContext({ viewport: {width:1440,height:944}, reducedMotion:'reduce' });
    const a = await ctx.newPage(), b = await ctx.newPage();
    for (const theme of ['light','dark']) for (const state of ['', 'lisbon','lisbon,weekly']) {
      await a.goto(c + `?theme=${theme}&frame=d&open=${state}`);
      await b.goto(d + `?review=${++navigation}#calico=quiet&fun=off&theme=${theme}&frame=d&open=${state}`);
      await settle(a); await settle(b);
      await a.mouse.move(0,0); await b.mouse.move(0,0);
      const ca = await a.locator('#stage').screenshot(), db = await b.locator('#stage').screenshot();
      const x = PNG.sync.read(ca), y = PNG.sync.read(db);
      assert.equal(x.width, y.width); assert.equal(x.height, y.height);
      let changed=0, significant=0;
      for(let i=0;i<x.data.length;i+=4) {
        const delta=Math.max(...[0,1,2].map(k=>Math.abs(x.data[i+k]-y.data[i+k])));
        if(delta) changed++;
        if(delta>32) significant++;
      }
      const fraction=changed/(x.width*x.height);
      assert.ok(fraction<.001 && significant<10, `Quiet differs from C: ${theme}/${state || 'home'}, ${changed} pixels, ${significant} significant`);
      checks.push(`Quiet matches C: ${theme}/${state || 'home'} (${(100*fraction).toFixed(4)}% changed edge pixels, ${significant} >32 channel delta)`);
    }
    await b.goto(d);
    await settle(b);
    assert.equal(await b.locator('button[data-calico="tabs"]').getAttribute('aria-pressed'), 'true');
    assert.equal(await b.locator('button[data-fun="off"]').getAttribute('aria-pressed'), 'true');
    for (const [selector, key, value] of [
      ['button[data-calico="patches"]','calico','patches'],
      ['button[data-fun="on"]','fun','on'],
      ['[data-theme-set="dark"]','theme','dark'],
      ['[data-frame="m"]','frame','m'],
      ['[data-motion="reduced"]','motion','reduced'],
    ]) {
      await b.locator(selector).click();
      assert.equal(await b.evaluate(key => new URLSearchParams(location.hash.slice(1)).get(key), key), value);
    }
    await b.reload(); await settle(b);
    assert.equal(await b.locator('button[data-calico="patches"]').getAttribute('aria-pressed'), 'true');
    assert.equal(await b.locator('button[data-fun="on"]').getAttribute('aria-pressed'), 'true');
    assert.equal(await b.locator('#app').evaluate(e => e.classList.contains('m')), true);
    checks.push('Defaults Tabs/Off; all five setting controls update hash and reload restores settings');
    await b.locator('#aboutBtn').click();
    assert.equal(await b.locator('#about').isVisible(), true);
    for (const rule of ['R1','R2','R3','R4','R5']) assert.match(await b.locator('#about').textContent(), new RegExp(rule));
    await b.keyboard.press('Escape'); await b.waitForTimeout(50);
    assert.equal(await b.locator('#about').isVisible(), false);
    checks.push('About names trial, R1–R5 present, Escape dismisses');
    for (const calico of ['quiet','tabs','patches']) {
      await b.goto(d + `?review=${++navigation}#calico=${calico}&fun=on&theme=light&frame=d&open=lisbon`);
      await settle(b);
      while (await b.locator('#packBlock .open-list .tick').count()) {
        await b.locator('#packBlock .open-list .tick').first().click();
        await b.waitForTimeout(50);
      }
      assert.equal(await b.locator('.confetti-piece').count(), 0);
      assert.equal(await b.locator('#packBlock [data-ptext="packing"]').textContent(), 'All packed');
      checks.push(`${calico}/Fun On/system reduced motion: no confetti; All packed remains visible`);
    }
    fs.writeFileSync(path.join(__dirname,'review.json'), JSON.stringify(checks,null,2)+'\n');
    console.log(JSON.stringify(checks,null,2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode=1; });
