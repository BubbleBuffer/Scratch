'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { launch } = require('../playwright.cjs');
const url = pathToFileURL(path.join(__dirname, 'index.html')).href;
const checks = [], errors = [];
let run = 0;
async function open(page, hash) {
  await page.goto(url + '?details=' + (++run) + '#' + hash);
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(1700);
}
(async () => {
  const browser = await launch();
  try {
    const page = await browser.newPage({ viewport: { width:1440, height:944 } });
    page.on('pageerror', e => errors.push(e.message));
    page.on('console', e => { if(e.type()==='error') errors.push(e.text()); });
    for (const calico of ['tabs','patches']) {
      for (const width of [1440,900,390]) {
        await page.setViewportSize({width,height:width===390?844:944});
        await open(page, `calico=${calico}&fun=off&theme=light&frame=${width===390?'m':'d'}`);
        const tiles = await page.locator('.space-tile').evaluateAll(es => es.map(e => {
          const r=e.getBoundingClientRect(); return {width:r.width,height:r.height};
        }));
        assert.ok(tiles.every(t=>Math.abs(t.width-tiles[0].width)<1 && t.height===tiles[0].height));
        const fit = await page.locator('.sheet-body').evaluate(e=>e.scrollWidth<=e.clientWidth+1);
        assert.equal(fit,true);
        await page.locator('.space-tile.sp-trips').click();
        await page.waitForTimeout(800);
        const geometry = await page.locator('.sheet[data-id="lisbon"]').evaluate(e=>({
          band:parseFloat(getComputedStyle(e,'::before').height),
          hero:e.querySelector('.hero-band').getBoundingClientRect().height,
          tile:e.querySelector('.pico').getBoundingClientRect().height,
          railTile:document.querySelector('.space.sp-trips .space-icon').getBoundingClientRect().width,
          underline:getComputedStyle(e.querySelector('.mention')).borderBottomWidth,
          pageBodyOverflow:e.querySelector('.sheet-body').scrollWidth>e.querySelector('.sheet-body').clientWidth+1,
        }));
        assert.equal(geometry.railTile,22); assert.equal(geometry.underline,'3px');
        assert.equal(geometry.pageBodyOverflow,false);
        if(calico==='tabs') assert.equal(geometry.band,6);
        else { assert.equal(geometry.hero,width===390?72:104); assert.equal(geometry.tile,56); }
        checks.push(`${calico}/${width}px: equal tiles, no horizontal sheet overflow, rail tile22px, mention3px, ${calico==='tabs'?'top band6px':`cover${geometry.hero}px and icon56px`} passed`);
      }
    }
    await page.setViewportSize({width:1440,height:944});
    for(const calico of ['quiet','tabs','patches']) {
      await open(page,`calico=${calico}&fun=on&theme=light&frame=d&motion=reduced`);
      await page.locator('#proto [data-act="blank"]').click();
      await page.waitForTimeout(80);
      await page.locator('.sheet:not(.covered) [data-act="start-check"]').click();
      while(await page.locator('.sheet:not(.covered) .open-list .tick').count()) {
        await page.locator('.sheet:not(.covered) .open-list .tick').first().click();
        await page.waitForTimeout(80);
      }
      assert.equal(await page.locator('.sheet:not(.covered) .ptext').textContent(),'All done');
      assert.equal(await page.locator('.confetti-piece').count(),0);
      await page.locator('.sheet:not(.covered) .start-out .pile').click();
      await page.waitForTimeout(30);
      await page.locator('.sheet:not(.covered) .pile-list .tick').first().click();
      await page.waitForTimeout(30);
      assert.equal(await page.locator('.sheet:not(.covered) .ptext').textContent(),'2 of 3 done');
      checks.push(`${calico}/Fun On/manual reduced: blank checklist All done, no confetti, undo restores progress passed`);
    }
    await open(page,'calico=tabs&fun=on&theme=light&frame=d');
    while(await page.locator('[data-list="today"] .open-list .tick').count()) {
      await page.locator('[data-list="today"] .open-list .tick').first().click();
      await page.waitForTimeout(1100);
    }
    assert.equal(await page.locator('.confetti-piece').count(),14);
    assert.equal(await page.locator('.today .sec-h .meta').textContent(),'All done');
    assert.ok((await page.locator('.confetti-piece').evaluateAll(es=>es.map(e=>getComputedStyle(e).backgroundColor))).every(c=>c!=='rgba(0, 0, 0, 0)'));
    checks.push('Home Today/Fun On: All done and 14 coloured confetti pieces passed');
    assert.deepEqual(errors,[]);
    fs.writeFileSync(path.join(__dirname,'details.json'),JSON.stringify({checks,errors},null,2)+'\n');
    console.log(JSON.stringify({checks,errors},null,2));
  } finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
