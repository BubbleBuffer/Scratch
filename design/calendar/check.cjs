// Optional verification; viewing index.html needs no tooling or dependencies.
// CALENDAR_PLAYWRIGHT=/path/to/playwright node check.cjs
const { chromium } = require(process.env.CALENDAR_PLAYWRIGHT || 'playwright');
const { spawn } = require('node:child_process');
const net = require('node:net');
const path = require('node:path');
const fs = require('node:fs');
const assert = require('node:assert/strict');
const output = path.join(__dirname, '.checks');
fs.mkdirSync(output, { recursive: true });
const screens = ['month','week','day','calendars','task','create','overflow','jump','note','cards','today','today-future','today-past','today-sync','source','today-create','today-created','legend','shortcuts'];
async function startServer(){
  const port = await new Promise((resolve,reject)=>{
    const socket = net.createServer();socket.once('error',reject);
    socket.listen(0,'127.0.0.1',()=>{const port=socket.address().port;socket.close(error=>error?reject(error):resolve(port));});
  });
  const server = spawn('python',['-m','http.server',String(port),'--bind','127.0.0.1','--directory',__dirname],{stdio:['ignore','ignore','pipe'],windowsHide:true});
  const stopped = new Promise(resolve=>server.once('close',resolve));
  let startupError=null;server.once('error',error=>{startupError=error;});
  let stderr='';server.stderr.on('data',chunk=>{stderr=(stderr+chunk).slice(-2000);});
  const url=`http://127.0.0.1:${port}/index.html`;
  const stop=async()=>{if(server.exitCode===null&&!startupError)server.kill();await stopped;};
  try{
    for(let attempt=0;attempt<100;attempt++){
      if(startupError)throw startupError;
      if(server.exitCode!==null)throw new Error(`HTTP server exited: ${stderr}`);
      try{if((await fetch(url,{signal:AbortSignal.timeout(500)})).ok)return {url,stop};}catch{}
      await new Promise(resolve=>setTimeout(resolve,100));
    }
    throw new Error('HTTP server did not become ready');
  }catch(error){await stop();throw error;}
}
(async()=>{
  const server = await startServer();
  let browser;
  const errors = [], results = [];let captures=0;
  try {
    browser = await chromium.launch({headless:true,channel:'chrome'});
    const page = await browser.newPage({viewport:{width:1440,height:1000}});
    page.on('pageerror',e=>errors.push(e.message));
    page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
    await page.goto(server.url);
    await page.evaluate(()=>Promise.all(['Atkinson','Bricolage','Fragment'].map(name=>document.fonts.load(`14px ${name}`))));
    await page.evaluate(()=>document.fonts.ready);
    assert.deepEqual(await page.evaluate(()=>Object.fromEntries(window.CALENDAR_DATA.calendars.map(c=>[c.id,c.palette]))),{personal:'apricot',work:'sky',agent:'duvet',projects:'moss',reading:'sun'});
    assert(await page.locator('.day-cell').count()>80,'Several months are loaded');
    assert.equal(await page.locator('.day-cell[data-day="2026-10-12"] .item-chip').count(),3);
    assert((await page.locator('.day-cell[data-day="2026-10-12"] .more-items').innerText()).includes('+5'));
    for(const width of ['desktop','phone'])for(const theme of ['light','dark']){
      await page.setViewportSize({width:width==='phone'?390:1440,height:1000});
      await page.selectOption('#width',width);await page.selectOption('#theme',theme);
      for(const screen of screens){
        await page.selectOption('#screen',screen);await page.waitForTimeout(40);
        assert.equal(await page.evaluate(()=>document.querySelector('#frame').scrollWidth<=document.querySelector('#frame').clientWidth),true,`${width}/${theme}/${screen}: no horizontal frame overflow`);
        if(width==='phone')assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth),true,`${width}/${theme}/${screen}: no horizontal page overflow at 390px`);
        const fontFaces=await page.evaluate(()=>[...document.fonts].map(face=>face.status));assert.equal(fontFaces.length,3);assert(fontFaces.every(status=>status==='loaded'),`${width}/${theme}/${screen}: all local fonts loaded over HTTP`);
        const text=await page.locator('#frame').innerText();assert(text.trim().length>100,`${screen} has content`);
        if(['task','create','overflow','jump','shortcuts','today-create','legend'].includes(screen))assert.equal(await page.locator('.dialog').count(),1);
        if(screen==='task')assert(text.includes('Scheduled runs come later.'));
        if(screen==='task'){
          assert.equal(await page.locator('[data-edit="duration"]').isVisible(),false,'Untimed sheet hides Duration');
          assert.equal(await page.locator('.property>label').filter({hasText:/^Calendar$/}).locator('.dot').count(),0,'Calendar label has no stray dot');
        }
        if(screen==='create'||screen==='today-create'){
          assert.equal(await page.locator('#composer [name="timed"]').evaluate(el=>el.getBoundingClientRect().width),22,'Composer time toggle stays compact');
          assert.equal(await page.locator('#composer [name="time"]').isVisible(),false);
          assert.equal(await page.locator('#composer [name="duration"]').isVisible(),false);
        }
        if(screen==='calendars'){
          const expected={personal:'rgb(222, 148, 95)',work:'rgb(203, 213, 223)',agent:theme==='dark'?'rgb(153, 157, 166)':'rgb(47, 49, 56)',projects:'rgb(105, 122, 107)',reading:'rgb(240, 210, 156)'};
          const dots=await page.locator('.calendar-row').evaluateAll(rows=>Object.fromEntries(rows.map(row=>[row.querySelector('[data-action="toggle-calendar"]').dataset.id,getComputedStyle(row.querySelector('.dot')).backgroundColor])));
          assert.deepEqual(dots,expected,`${theme} panel colours use spaceColor`);
        }
        if(screen==='month'&&width==='desktop'){
          assert.deepEqual(await page.locator('.item-chip input[type="checkbox"]').first().evaluate(el=>({width:el.getBoundingClientRect().width,height:el.getBoundingClientRect().height})),{width:12,height:12});
          assert.equal(await page.locator('.item-chip.mention').first().evaluate(el=>getComputedStyle(el).backgroundColor),'rgba(0, 0, 0, 0)');
        }
        if(screen==='week'&&width==='desktop'){
          const gutter=await page.locator('.hours').boundingBox(),label=await page.locator('.now-gutter-label').boundingBox();
          assert(label.x>=gutter.x&&label.x+label.width<=gutter.x+gutter.width+1,'Now label stays in the time gutter');
          assert.equal(await page.locator('.now-line span').count(),0);
          assert(await page.locator('.now-line').evaluate(el=>Number(getComputedStyle(el).zIndex)<Number(getComputedStyle(el.parentElement.querySelector('.time-block')).zIndex)),'Now line sits behind event text');
        }
        if(screen.startsWith('today')||screen==='source'){
          assert.equal(await page.locator('.today-header h1').innerText(),'Today');
          assert((await page.locator('.breadcrumb').innerText()).includes('Personal vault')&&(await page.locator('.breadcrumb').innerText()).includes('Today'));
          assert.equal(await page.locator('.today-header [data-action="calendar-page"],.today-range,.today-header [data-action="how-fits"],#active-agenda-date').count(),0,'Today header has no redundant labels');
          assert.equal(await page.locator('.today-view .date-chip').count(),0,'Day headers provide card dates');
          assert.equal(await page.locator('.today-view [aria-label="Assigned to Me"]').count(),0);
          assert.equal(await page.locator('.today-overdue [data-item="i10"] .overdue-date').innerText(),'Fri 9 Oct');
          if(width==='desktop'){
            const heights=await page.locator('.today-view .item-card').evaluateAll(cards=>cards.map(card=>card.getBoundingClientRect().height));
            assert(heights.length>0&&heights.every(height=>height<=48),`${screen}: Today desktop cards <=48px (max ${Math.max(...heights)})`);
          }
          if(screen==='today')assert(await page.locator('#today-scroll').evaluate(el=>{const section=el.querySelector('[data-date="2026-10-12"]');return Math.abs(section.offsetTop-el.scrollTop-64)<=1;}),'End of yesterday peeks above today');
          if(screen==='today-future'){const empty=page.locator('.today-day[data-date="2026-10-23"] .today-empty');assert.equal(await empty.innerText(),'Nothing planned');assert(await empty.evaluate(el=>{const view=el.closest('#today-scroll').getBoundingClientRect(),rect=el.getBoundingClientRect();return rect.top>=view.top&&rect.bottom<=view.bottom;}),'Next-week empty day is in view');}
          assert.equal(await page.locator('#today-scroll').count(),1);
          assert(await page.locator('.today-day').count()>=43,'Past and future are continuously loaded');
          assert.equal(await page.locator('.today-view .item-card[data-kind="mention"]').count(),0,'Plain mentions excluded from Today');
          assert.equal(await page.locator('.today-view .item-card[data-calendar-id="agent"]').count(),0,'Agent calendar excluded from Today');
          const expectedDate=screen==='today-past'?'2026-10-07':screen==='today-future'?'2026-10-23':screen==='today-sync'?'2026-10-20':screen==='today-create'||screen==='today-created'?'2026-10-22':screen==='source'?null:'2026-10-12';
          if(expectedDate)await page.waitForFunction(date=>document.querySelector('#today-week-strip .week-tile.selected')?.dataset.date===date,expectedDate);
          if(screen==='today-past')assert(await page.locator('.today-day[data-date="2026-10-07"] .item-card.done').count()>0);
          if(screen==='today-create')assert.equal(await page.inputValue('#composer [name="date"]'),'2026-10-22');
          if(screen==='today-created')assert.equal(await page.locator('.today-day[data-date="2026-10-22"] [data-item="demo-future-task"]').count(),1);
          if(screen==='source'){
            const row=page.locator('.today-view .item-card.show-source');assert.equal(await row.count(),1);
            assert((await row.locator('.item-source-label').innerText()).includes('Trip plan'));
            assert.equal(await row.locator('.item-calendar-tooltip').isVisible(),true);
          }
        }
        results.push(`${width}/${theme}/${screen}`);
        if(['month','week','day','task','note','cards','calendars','create','legend','source'].includes(screen)||screen.startsWith('today')){
          const filename=screen.startsWith('today')||screen==='source'?`today-${width}-${theme}-${screen==='today'?'anchor':screen==='source'?'source':screen.slice(6)}.png`:`${width}-${theme}-${screen}.png`;
          await page.locator('#frame').screenshot({path:path.join(output,filename)});captures++;
        }
      }
    }
    console.log(`Screen matrix: ${screens.length*4} combinations passed`);
    results.push('exact calendar palettes, chip checkbox size, gutter label, conditional duration across theme/width matrix');
    await page.setViewportSize({width:1440,height:1000});
    await page.selectOption('#width','desktop');await page.selectOption('#screen','task');
    await page.click('[data-action="add-time"]');
    assert.equal(await page.locator('[data-edit="time"]').isVisible(),true);
    assert.equal(await page.locator('[data-edit="duration"]').isVisible(),true);
    await page.click('[data-action="remove-time"]');
    assert.equal(await page.locator('[data-edit="duration"]').isVisible(),false);
    await page.selectOption('#screen','create');await page.locator('[name="timed"]').check();
    assert.equal(await page.locator('#composer [name="time"]').isVisible(),true);
    assert.equal(await page.locator('#composer [name="duration"]').isVisible(),true);
    await page.locator('[name="timed"]').uncheck();
    assert.equal(await page.locator('#composer [name="duration"]').isVisible(),false);
    results.push('Add time reveals Time and Duration together in sheet and composer; remove hides Duration');
    await page.selectOption('#width','desktop');await page.selectOption('#theme','light');await page.click('#reset');
    assert(await page.locator('.item-chip [aria-label="Assigned to Agent"]').count()>0);
    await page.click('[data-action="toggle-calendar"][data-id="agent"]');
    assert.equal(await page.locator('.item-chip [aria-label="Assigned to Agent"]').count(),0);
    await page.click('[data-action="toggle-calendar"][data-id="agent"]');
    await page.click('.sidebar [data-action="today-page"]');assert.equal(await page.locator('.agenda-row [aria-label="Assigned to Agent"]').count(),0);
    await page.click('.header-actions [data-action="calendars"]');await page.click('[data-action="toggle-today"][data-id="agent"]');
    assert(await page.locator('.agenda-row [aria-label="Assigned to Agent"]').count()>0);
    results.push('calendar and Today switches independent');
    await page.click('#reset');
    await page.locator('.recent-note[data-note="reading-notes"]').dragTo(page.locator('.day-cell[data-day="2026-10-16"]'));
    assert((await page.locator('#toast').innerText()).includes("Added 16 Oct to 'Thinking in Systems'"));
    await page.click('#toast [data-action="undo"]');
    assert.equal(await page.locator('.day-cell[data-day="2026-10-16"] [title="Thinking in Systems"]').count(),0);
    results.push('note drag/drop and undo');
    await page.locator('.day-cell[data-day="2026-10-12"] [data-item="i12"]').dragTo(page.locator('.day-cell[data-day="2026-10-16"]'));
    assert.equal(await page.locator('.day-cell[data-day="2026-10-16"] [data-item="i12"]').count(),1);
    await page.click('#toast [data-action="undo"]');
    assert.equal(await page.locator('.day-cell[data-day="2026-10-12"] [data-item="i12"]').count(),1);
    results.push('task drag/drop and undo');
    await page.locator('.header-title').click();await page.keyboard.press('n');
    await page.fill('#composer [name="title"]','Mockup verification task');await page.keyboard.press('Enter');
    assert.equal(await page.locator('.dialog').count(),0);
    await page.locator('.header-title').click();await page.keyboard.press('w');assert(await page.locator('.time-grid').count()>0);
    await page.click('.hour-slot[data-date="2026-10-14"][data-time="14:00"]');
    assert.equal(await page.inputValue('#composer [name="time"]'),'14:00');
    assert.equal(await page.inputValue('#composer [name="duration"]'),'60');
    await page.fill('#composer [name="title"]','Timed verification task');await page.keyboard.press('Enter');
    assert((await page.locator('.time-block').allTextContents()).some(t=>t.includes('Timed verification task')));
    await page.locator('.header-title').click();await page.keyboard.press('g');assert.equal(await page.locator('.mini-calendar').count(),1);await page.keyboard.press('Escape');
    await page.selectOption('#screen','cards');await page.locator('.compact-card [data-check="i12"]').first().check();
    assert.equal(await page.locator('.compact-card.done[data-item="i12"]').count(),2);
    await page.selectOption('#screen','task');await page.selectOption('[data-edit="assignee"]','me');
    await page.fill('[data-edit="title"]','Edited task title');await page.locator('[data-edit="body"]').click();
    await page.keyboard.press('Escape');
    results.push('keyboard, quick creation, timed slot, shared card checkbox, task edits');
    // Regressions found in review: empty board samples, live date links, and title growth.
    await page.selectOption('#screen','cards');
    const unfinishedIds=await page.locator('.compact-card input:not(:checked)').evaluateAll(els=>[...new Set(els.map(el=>el.dataset.check))]);
    for(const id of unfinishedIds)await page.locator(`.compact-card input[data-check="${id}"]`).first().check();
    assert.equal(await page.locator('.kanban-column').count(),3);
    await page.selectOption('#screen','task');
    const longTitle='A much longer task title that wraps across several lines while I edit it in the detail sheet';
    const oldHeight=await page.locator('.task-title').evaluate(el=>el.clientHeight);
    await page.fill('.task-title',longTitle);
    assert((await page.locator('.task-title').evaluate(el=>el.clientHeight))>oldHeight);
    assert(await page.locator('.task-title').evaluate(el=>el.scrollHeight<=el.clientHeight+1));
    await page.locator('#frame').screenshot({path:path.join(output,'desktop-long-task-title.png')});
    await page.fill('[data-edit="date"]','2026-11-05');await page.locator('[data-edit="body"]').click();
    await page.click('[data-action="show-task-day"]');
    assert((await page.locator('.grid-day').innerText()).includes('5'));
    assert.equal(await page.locator('.hour-slot[data-time="00:00"]').count(),1);
    assert.equal(await page.locator('.hour-slot[data-time="23:00"]').count(),1);
    results.push('all board samples completed; long title autosizing; live edited-date navigation; 24-hour slots');
    await page.selectOption('#screen','note');await page.locator('.note-meta [data-action="date-link"]').first().click();
    assert.equal(await page.locator('.day-layout').count(),1);assert((await page.locator('.grid-day').innerText()).includes('16'));
    await page.selectOption('#screen','calendars');
    await page.click('[data-action="new-calendar"]');await page.fill('#calendar-name','Studio');
    await page.click('[data-color="paw"]');await page.click('#calendar-form button[type="submit"]');
    assert((await page.locator('.calendar-name').allTextContents()).includes('Studio'));
    results.push('date chip navigation and new calendar');
    await page.selectOption('#width','phone');await page.selectOption('#screen','week');
    const swipe=await page.locator('.swipe-surface').first().boundingBox();
    const before=await page.locator('.week-strip').innerText();
    await page.mouse.move(swipe.x+swipe.width-25,swipe.y+25);await page.mouse.down();
    await page.mouse.move(swipe.x+25,swipe.y+25,{steps:8});await page.mouse.up();
    assert.notEqual(await page.locator('.week-strip').innerText(),before);
    await page.selectOption('#screen','day');const dayBefore=await page.locator('.agenda-heading h3').first().innerText();
    const surface=await page.locator('[data-swipe="day"]').boundingBox();
    await page.mouse.move(surface.x+surface.width-25,surface.y+60);await page.mouse.down();await page.mouse.move(surface.x+25,surface.y+60,{steps:8});await page.mouse.up();
    assert.notEqual(await page.locator('.agenda-heading h3').first().innerText(),dayBefore);
    await page.setViewportSize({width:390,height:1000});
    for(const screen of screens){await page.selectOption('#screen',screen);assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth),true,`390 page ${screen} has no horizontal overflow`);}
    results.push('phone week/day mouse swipes; all screens at actual 390px without page overflow');
    await page.setViewportSize({width:1440,height:1000});await page.selectOption('#width','desktop');await page.click('#reset');
    await page.selectOption('#screen','today');
    assert((await page.locator('.today-day[data-date="2026-10-12"]').innerText()).includes('Overdue'));
    assert.equal(await page.locator('.today-day[data-date="2026-10-12"] .item-card[data-item="i10"]').count(),1);
    const overdueToggle=page.locator('.today-day[data-date="2026-10-12"] .today-overdue summary');
    await overdueToggle.click();assert.equal(await page.locator('.today-day[data-date="2026-10-12"] .item-card[data-item="i10"]').isVisible(),false);
    await overdueToggle.click();assert.equal(await page.locator('.today-day[data-date="2026-10-12"] .item-card[data-item="i10"]').isVisible(),true);
    results.push('Today overdue section is pinned within today and collapsible');
    for(const width of ['desktop','phone']){
      await page.selectOption('#width',width);await page.selectOption('#screen','today-future');
      assert.equal(await page.locator('[data-action="back-to-today"]').isVisible(),true);
      await page.click('#today-week-strip [data-action="select-day"][data-date="2026-10-23"]');
      await page.waitForFunction(()=>document.querySelector('#today-week-strip .week-tile.selected')?.dataset.date==='2026-10-23');
      await page.locator('#today-scroll').evaluate(scroller=>{const target=scroller.querySelector('[data-date="2026-10-07"]');scroller.scrollTop+=target.getBoundingClientRect().top-scroller.getBoundingClientRect().top;});
      await page.waitForFunction(()=>document.querySelector('#today-week-strip .week-tile.selected')?.dataset.date==='2026-10-07');
      assert.equal(await page.locator('#today-week-strip [data-date="2026-10-05"]').count(),1,'Strip pages to previous week');
      await page.click('[data-action="back-to-today"]');
      await page.waitForFunction(()=>document.querySelector('#today-week-strip .week-tile.selected')?.dataset.date==='2026-10-12');
      await page.selectOption('#screen','today-future');
      if(width==='desktop'){await page.locator('.today-header h1').click();await page.keyboard.press('t');await page.waitForFunction(()=>document.querySelector('#today-week-strip .week-tile.selected')?.dataset.date==='2026-10-12');}
      await page.selectOption('#screen','today-future');
      await page.click('[data-action="today-week-prev"]');
      await page.waitForFunction(()=>document.querySelector('#today-week-strip .week-tile.selected')?.dataset.date==='2026-10-12');
      await page.click('[data-action="today-week-next"]');
      await page.waitForFunction(()=>document.querySelector('#today-week-strip .week-tile.selected')?.dataset.date==='2026-10-19');
      await page.click('.today-month');assert.equal(await page.locator('.mini-calendar').count(),1);
      await page.fill('#jump-form [name="date"]','2026-11-05');await page.click('#jump-form button[type="submit"]');
      await page.waitForFunction(()=>document.querySelector('#today-week-strip .week-tile.selected')?.dataset.date==='2026-11-05');
      assert.equal(await page.locator('.today-header h1').innerText(),'Today');
      assert.equal(await page.locator('.today-month').innerText(),'November 2026');
      if(width==='desktop'){await page.locator('.today-header h1').click();await page.keyboard.press('g');assert.equal(await page.locator('.mini-calendar').count(),1);await page.keyboard.press('Escape');}
      results.push(`${width}: week arrows select Monday; month label and G jump stay in Today and update the month`);
      await page.selectOption('#screen','today-created');
      await page.click(width==='desktop'?'.sidebar [data-action="calendar-page"]':'.mobile-dock [data-action="calendar-page"]');
      assert.equal(await page.locator('.today-view').count(),0,'Calendar is a sibling view');
      if(width==='desktop')assert.equal(await page.locator('.all-day-cell[data-day="2026-10-22"] [data-item="demo-future-task"]').count(),1);
      else assert.equal(await page.locator('.week-tile.selected[data-date="2026-10-22"]').count(),1);
      await page.click(width==='desktop'?'.sidebar [data-action="today-page"]':'.mobile-dock [data-action="today-page"]');
      await page.waitForFunction(()=>document.querySelector('#today-week-strip .week-tile.selected')?.dataset.date==='2026-10-22');
      const addTask=page.locator('.today-day[data-date="2026-10-22"] [data-action="create"]');
      await addTask.click();assert.equal(await page.inputValue('#composer [name="date"]'),'2026-10-22');
      const title=`Added from future ${width} agenda`;await page.fill('#composer [name="title"]',title);await page.keyboard.press('Enter');
      const newCard=page.locator('.today-day[data-date="2026-10-22"] .item-card').filter({hasText:title});assert.equal(await newCard.count(),1);
      await newCard.locator('[data-check]').check();assert(await newCard.evaluate(el=>el.classList.contains('done')));
      await page.click('.document-top .header-actions [data-action="calendars"]');
      await page.click('[data-action="toggle-today"][data-id="work"]');
      await page.waitForFunction(()=>document.querySelector('#today-week-strip .week-tile.selected')?.dataset.date==='2026-10-22');
      assert.equal(await page.locator('.today-view .item-card[data-calendar-id="work"]').count(),0);
      await page.click('[data-action="toggle-today"][data-id="work"]');
      await page.waitForFunction(()=>document.querySelector('#today-week-strip .week-tile.selected')?.dataset.date==='2026-10-22');
      await page.keyboard.press('Escape');
      results.push(`${width}: strip tap/manual scroll/week paging, Back to today, date continuity, future creation, shared checkbox and filter anchoring`);
    }
    await page.selectOption('#width','desktop');await page.selectOption('#screen','source');
    const sourceId=await page.locator('.show-source').getAttribute('data-item');const sourceMarker=page.locator(`.today-view .item-card[data-item="${sourceId}"] .item-calendar-marker`);await page.keyboard.press('Escape');await sourceMarker.hover();
    assert.equal(await sourceMarker.locator('[role="tooltip"]').isVisible(),true);
    await page.selectOption('#width','phone');await page.selectOption('#screen','source');
    const phoneSourceId=await page.locator('.show-source').getAttribute('data-item');const phoneMarker=page.locator(`.today-view .item-card[data-item="${phoneSourceId}"] .item-calendar-marker`);await phoneMarker.click();await phoneMarker.click();
    assert.equal(await page.locator('.show-source .item-calendar-tooltip').isVisible(),true);
    results.push('Calendar/source hover tooltip on desktop and tap label on phone');
    for(const width of ['desktop','phone']){
      await page.selectOption('#width',width);await page.click('#reset');await page.selectOption('#screen','jump');
      await page.fill('#jump-form [name="date"]','2026-11-02');await page.click('#jump-form button[type="submit"]');
      await page.click(width==='desktop'?'.sidebar [data-action="today-page"]':'.mobile-dock [data-action="today-page"]');
      await page.waitForFunction(()=>document.querySelector('#today-week-strip .week-tile.selected')?.dataset.date==='2026-11-02');
      assert.equal(await page.locator('.today-day').last().getAttribute('data-date'),'2026-11-02');
      await page.click('#today-week-strip [data-action="select-day"][data-date="2026-11-02"]');
      assert(await page.locator('#today-scroll').evaluate(el=>Math.abs(el.scrollTop-el.querySelector('[data-date="2026-11-02"]').offsetTop)<=1));
      results.push(`${width}: final loaded day stays selected after Calendar→Today and strip tap`);
    }
    // Same item-card structure and status appears in board and embed.
    await page.selectOption('#screen','cards');
    const boardParts=await page.locator('.compact-card[data-item="i12"]').evaluateAll(cards=>cards.map(card=>[...card.children].map(el=>el.className).join('|')));
    assert.equal(boardParts.length,2);assert.equal(boardParts[0],boardParts[1]);
    await page.selectOption('#screen','legend');assert((await page.locator('.dialog').innerText()).includes('Calendar.'));
    results.push('Unified board/embed card structure and How it fits legend');
    assert.deepEqual(errors,[],'No browser console or runtime errors');
    const fonts=await page.evaluate(()=>[...document.fonts].map(face=>({name:face.family,status:face.status,loaded:face.status==='loaded'})));
    assert(fonts.every(f=>f.loaded));
    fs.writeFileSync(path.join(output,'results.json'),JSON.stringify({passed:results.length,screenCombinations:screens.length*4,screenshots:captures+1,servedOverHttp:true,results,fonts,errors},null,2));
    console.log(JSON.stringify({passed:results.length,fonts,errors,screenshots:captures+1},null,2));
  } finally {try{await browser?.close();}finally{await server.stop();}}
})().catch(e=>{console.error(e);process.exitCode=1;});
