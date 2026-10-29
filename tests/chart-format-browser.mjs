import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

export async function checkChartFormatting({context,browser,origin,database,output}) {
  execFileSync('python3',['-c',`
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
c.execute("INSERT INTO sites VALUES(11,'chart-labels.example','UTC',0)")
for day in ['2024-02-03','2024-02-05','2024-02-14']:
 c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(11,unixepoch(?1)+36000,?1,randomblob(16),randomblob(16),'/','','US','Chrome','Desktop')",(day,))
c.commit()
`,database]);
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  for(const locale of ['en-US','en-GB','en-US-u-hc-h23']) {
    // A different browser timezone must not shift the site's civil clock.
    const owner=await browser.newContext({storageState,locale,timezoneId:'Pacific/Auckland',viewport:{width:1440,height:1050}});
    try {
      const page=await owner.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));
      page.on('console',message=>{if(message.type()==='error'&&/Content Security Policy|violates/.test(message.text()))errors.push(message.text());});
      const visit=async query=>{await page.goto(origin+query);await page.waitForFunction(()=>document.documentElement.hasAttribute('data-dashboard-enhanced'));};
      const base='/sites/2?period=day&date=2025-11-02&metric=pageviews&compare=off';
      const twelve=locale==='en-US';
      await visit(base+'&interval=hour');
      let labels=await page.locator('.chart-hit').evaluateAll(es=>es.map(e=>e.dataset.date));
      assert.equal(labels.length,25);
      for(const zone of ['EDT','EST']) assert.ok(labels.includes(`Sun, 02 Nov 2025, ${twelve?'1am':'01:00'} ${zone}`));
      assert.ok(labels.includes(`Sun, 02 Nov 2025, ${twelve?'12pm':'12:00'} EST`));
      assert.equal(await page.locator('.chart-ticks>text').first().textContent(),twelve?'12am':'00:00');
      const first=page.locator('.chart-hit').first();
      assert.ok((await first.getAttribute('aria-label')).includes(twelve?'12am EDT':'00:00 EDT'));
      await visit(base+'&interval=minute');
      labels=await page.locator('.chart-hit').evaluateAll(es=>es.map(e=>e.dataset.date));
      assert.equal(labels.length,1500);
      for(const zone of ['EDT','EST']) assert.ok(labels.includes(`Sun, 02 Nov 2025, ${twelve?'1:00am':'01:00'} ${zone}`));
      assert.ok(labels.includes(`Sun, 02 Nov 2025, ${twelve?'12:00pm':'12:00'} EST`));
      const repeated=page.locator('.chart-hit[data-clock="01:00"]');
      await repeated.nth(1).focus();
      assert.equal(await page.locator('.chart-tooltip-current span').innerText(),`Sun, 02 Nov 2025, ${twelve?'1:00am':'01:00'} EST`);
      await visit('/sites/2?period=day&date=2025-11-03&interval=hour&metric=pageviews&compare=previous&compare_match=date');
      const extra=page.locator('.chart-hit').last();
      assert.equal(await extra.getAttribute('data-date'),'');
      assert.equal(await extra.getAttribute('data-previous-date'),`Sun, 02 Nov 2025, ${twelve?'11pm':'23:00'} EST`);
      await extra.focus();assert.equal(await page.locator('.chart-tooltip-current').isVisible(),false);

      await visit('/sites/11?from=2024-02-03&to=2024-02-14&interval=week&metric=pageviews&compare=off');
      assert.deepEqual(await page.locator('.chart-hit').evaluateAll(es=>es.map(e=>e.dataset.date)),['Partial week of 3 Feb 24','Week of 5 Feb 24','Partial week of 12 Feb 24']);
      assert.equal(await page.locator('.chart-incomplete').count(),0,'Clipped historical weeks are complete');
      await visit('/sites/1?period=month&interval=week&metric=pageviews&compare=off');
      assert.equal(await page.locator('.chart-hit[data-incomplete=true]').count(),1);
      const pointCount=await page.locator('.chart-hit').count();
      assert.equal(await page.locator('.chart-incomplete').count(),pointCount>1?1:0);
      if(pointCount>1) assert.deepEqual(await page.locator('.chart-incomplete').evaluate(e=>getComputedStyle(e).strokeDasharray.split(/[ ,]+/).map(parseFloat)),[3,3]);

      if(twelve) {
        for(const [width,scheme] of [[1440,'light'],[390,'dark'],[360,'light']]) {
          await page.setViewportSize({width,height:1000});await page.emulateMedia({colorScheme:scheme});
          await visit('/sites/1?period=year&interval=month&metric=pageviews&compare=off');
          await page.screenshot({path:join(output,`chart-months-${scheme}-${width}.png`),fullPage:true});
          const boxes=await page.locator('.chart-ticks>text').evaluateAll(es=>es.map(e=>({left:e.getBoundingClientRect().left,right:e.getBoundingClientRect().right})));
          const bounds=await page.locator('.chart-plot').boundingBox();
          assert.ok(boxes.every(b=>b.left>=bounds.x-1&&b.right<=bounds.x+bounds.width+1),JSON.stringify({width,boxes,bounds}));
          assert.ok(boxes.slice(1).every((b,i)=>b.left>=boxes[i].right-1),'Month labels must not overlap');
          const full=page.locator('.axis-long').first(),short=page.locator('.axis-short').first();
          assert.equal(await full.isVisible(),width>600);assert.equal(await short.isVisible(),width<=600);
          await visit(base+'&interval=minute');
          await page.screenshot({path:join(output,`chart-minute-clock-${scheme}-${width}.png`),fullPage:true});
          assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
          const ticks=await page.locator('.chart-ticks>text').evaluateAll(es=>es.filter(e=>e.getBoundingClientRect().width>0).map(e=>({left:e.getBoundingClientRect().left,right:e.getBoundingClientRect().right})));
          assert.ok(ticks.slice(1).every((b,i)=>b.left>=ticks[i].right-1),JSON.stringify({width,ticks}));
        }
      }
      assert.deepEqual(errors,[]);
    } finally {await owner.close();}
  }
  const noScript=await browser.newContext({storageState,locale:'en-US',timezoneId:'Pacific/Auckland',javaScriptEnabled:false});
  try {
    const page=await noScript.newPage();await page.goto(origin+'/sites/2?period=day&date=2025-11-02&interval=hour&metric=pageviews&compare=off');
    assert.equal(await page.locator('.chart-hit title').count(),25);
    assert.match(await page.locator('.chart-hit title').first().textContent(),/Sun, 02 Nov 2025, 00:00 EDT/);
    assert.equal(await page.locator('.chart-ticks>text').first().textContent(),'00:00');
  } finally {await noScript.close();}
}
