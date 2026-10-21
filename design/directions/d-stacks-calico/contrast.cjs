// No dependencies. Read the prototype itself so the checks cannot drift from it.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const html = fs.readFileSync(path.join(__dirname, 'index.html'), 'utf8');
const css = html.match(/<style>([\s\S]*?)<\/style>/)[1];
function tokens(selector) {
  const values = {};
  const re = selector === 'light' ? /:root\s*\{([^}]+)\}/g : /:root\[data-theme="dark"\]\s*\{([^}]+)\}/g;
  for (const block of css.matchAll(re)) {
    for (const pair of block[1].matchAll(/(--[\w-]+)\s*:\s*(#[\da-f]{6})\b/gi)) values[pair[1]] = pair[2];
  }
  return values;
}
function luminance(hex) {
  const rgb = [1,3,5].map(i => parseInt(hex.slice(i,i+2),16)/255).map(v => v <= .04045 ? v/12.92 : ((v+.055)/1.055)**2.4);
  return rgb[0]*.2126 + rgb[1]*.7152 + rgb[2]*.0722;
}
function ratio(a,b) {
  const [high,low] = [luminance(a),luminance(b)].sort((x,y)=>y-x);
  return (high+.05)/(low+.05);
}
const light = tokens('light');
let failures = 0;
for (const theme of ['light','dark']) {
  const t = theme === 'light' ? light : {...light,...tokens('dark')};
  for (const space of ['home','work','trips','reading','garden']) {
    const ink=t[`--sp-${space}-ink`], patch=t[`--sp-${space}`], contrast=ratio(ink,patch);
    const pass=contrast>=4.5; if(!pass) failures++;
    console.log(`${theme} ${space.padEnd(7)} ${ink} on ${patch}: ${contrast.toFixed(2)}:1 ${pass?'PASS':'FAIL'}`);
  }
  const contrast=ratio(t['--ink'],t['--sheet']),pass=contrast>=4.5;
  if(!pass) failures++;
  console.log(`${theme} textPrimary on sheet ${t['--ink']} on ${t['--sheet']}: ${contrast.toFixed(2)}:1 ${pass?'PASS':'FAIL'}`);
}
console.log(failures ? `${failures} pair(s) failed WCAG 4.5:1.` : 'All 12 pairs meet WCAG 4.5:1. No A patch inks needed adjustment.');
process.exitCode=failures?1:0;
