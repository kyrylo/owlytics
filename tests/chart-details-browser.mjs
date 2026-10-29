import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

export async function checkChartDetails({context,browser,origin,database,output}) {
  execFileSync('python3',['-c',`
import sqlite3,sys
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
c.execute("INSERT INTO sites VALUES(130,'chart-details.example','UTC',0)")
c.execute("INSERT INTO import_batches VALUES(130,130,'2025-02-01','2025-02-06',0)")
for day in range(1,7):
 c.execute("INSERT INTO import_daily VALUES(130,?,130,1000,9000)",(f'2025-02-{day:02}',))
for day,visits in [(2,7),(4,1234),(5,2345),(6,3456)]:
 c.execute("INSERT INTO import_metrics VALUES(130,?,'total','',130,?,?,0,100,NULL)",(f'2025-02-{day:02}',visits,visits))
c.commit()
`,database]);
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  for(const [width,scheme,touch] of [[1440,'light',false],[1440,'dark',false],[320,'light',false],[390,'dark',true]]) {
    const owner=await browser.newContext({storageState,locale:'en-GB',viewport:{width,height:1000},colorScheme:scheme,hasTouch:touch,isMobile:touch});
    try {
      const page=await owner.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));
      const visit=async query=>{await page.goto(origin+query);await page.waitForFunction(()=>document.documentElement.hasAttribute('data-dashboard-enhanced'));};
      await visit('/sites/130?from=2025-02-04&to=2025-02-06&interval=day&metric=visits&compare=previous&compare_match=date');
      const hits=page.locator('.chart-hit'),tooltip=page.locator('.chart-tooltip'),middle=hits.nth(1);
      assert.equal(await page.locator('.chart-point-previous').count(),1,'The lone known comparison value is visible between two unknown buckets');
      const styles=await page.locator('.chart').evaluate(chart=>{
        const line=getComputedStyle(chart.querySelector('.chart-previous')),dot=getComputedStyle(chart.querySelector('.chart-point-previous'));
        return {line:line.stroke,dash:line.strokeDasharray,width:line.strokeWidth,dot:dot.stroke,dotWidth:dot.strokeWidth};
      });
      assert.equal(styles.dash,'none');assert.equal(styles.width,'2px');assert.equal(styles.dotWidth,'5px');assert.equal(styles.line,styles.dot);
      if(touch) await middle.tap();else await middle.hover();
      assert.equal(await tooltip.locator('.chart-tooltip-current span').innerText(),'Wed, 05 Feb 2025');
      assert.equal(await tooltip.locator('.chart-tooltip-previous span').innerText(),'Sun, 02 Feb 2025');
      assert.equal(await tooltip.locator('.chart-tooltip-current strong').innerText(),'2.3k');
      assert.equal(await tooltip.locator('.chart-tooltip-current strong').getAttribute('title'),'2,345','Exact counts remain available');
      assert.equal(await tooltip.locator('.chart-tooltip-previous strong').innerText(),'7');
      assert.equal(await tooltip.locator('footer').isVisible(),!touch);
      const contained=await tooltip.evaluate(t=>{const tip=t.getBoundingClientRect(),plot=t.parentElement.getBoundingClientRect();return tip.left>=plot.left&&tip.right<=plot.right+1&&Math.abs(tip.top-plot.top)<1;});
      assert.ok(contained,'Tooltip stays at the top and inside the plot');
      await page.screenshot({path:join(output,`chart-details-${scheme}-${width}.png`),fullPage:true});
      if(!touch) {
        await hits.first().hover();
        assert.equal(await tooltip.locator('.chart-tooltip-current strong').innerText(),'1.2k','Hover can reach a bucket underneath the floating tooltip');
        await middle.hover();
        await middle.click({button:'right'});
        assert.equal(await tooltip.getAttribute('role'),'dialog');
        assert.equal(await tooltip.locator('footer').isVisible(),false,'Pinning the same point immediately hides its hint');
        await hits.first().hover({position:{x:2,y:170}});
        assert.equal(await tooltip.locator('.chart-tooltip-current strong').innerText(),'2.3k','Pinned tooltip does not follow pointer movement');
      }
      await tooltip.locator('.chart-tooltip-zoom').click();
      await page.waitForURL(url=>url.searchParams.get('period')==='day'&&url.searchParams.get('date')==='2025-02-05');
      await page.waitForLoadState('domcontentloaded');
      assert.equal(await page.locator('.date-picker>summary>span').innerText(),'Wed, 05 Feb 2025');

      await visit('/sites/2?period=day&date=2025-11-02&interval=hour&compare=custom&compare_from=2024-11-02&compare_to=2024-11-02');
      const noon=page.locator('.chart-hit[data-clock="12:00"]');
      await noon.focus();
      assert.equal(await tooltip.locator('.chart-tooltip-current span').innerText(),'Sun, 02 Nov 2025, 12:00');
      assert.match(await tooltip.locator('.chart-tooltip-previous span').innerText(),/Sat, 02 Nov 2024, /,'Years distinguish comparison dates');
      for(const [index,zone] of [[0,'EDT'],[1,'EST']]) {
        await page.locator('.chart-hit[data-clock="01:00"]').nth(index).focus();
        assert.equal(await tooltip.locator('.chart-tooltip-current span').innerText(),`Sun, 02 Nov 2025, 01:00 ${zone}`,'Repeated clocks retain their timezone');
      }
      assert.deepEqual(errors,[]);
    } finally {await owner.close();}
  }
  const page=await context.newPage();
  try {
    for(const [from,to,label] of [['2025-02-01','2025-02-06','1 Feb - 6 Feb 25'],['2023-12-31','2024-01-02','31 Dec 23 - 2 Jan 24']]) {
      await page.goto(`${origin}/sites/130?from=${from}&to=${to}&compare=previous`);
      assert.equal(await page.locator('.date-picker>summary>span').innerText(),label);
      assert.equal(await page.locator('.metric-date').first().innerText(),label);
    }
  } finally {await page.close();}
}
