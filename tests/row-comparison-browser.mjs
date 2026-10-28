import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

export async function checkRowComparisons({context,browser,origin,database,output}) {
  execFileSync('python3',['-c',`
import sqlite3,sys,hashlib,datetime
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
c.execute("INSERT INTO sites VALUES(14,'comparison.example','UTC',0),(15,'missing-history.example','UTC',0)")
for day,counts in [('2025-02-01',{'GitHub':2,'Grow':1,'Shrink':4,'Same':1,'Old':2}),('2025-02-02',{'GitHub':3,'Grow':3,'Shrink':1,'Same':1,'New':1})]:
 for source,count in counts.items():
  for person in range(count):
   visitor=hashlib.sha256(f'{day}-{source}-{person}'.encode()).digest()[:16]
   stamp=int(datetime.datetime.fromisoformat(day+'T12:00:00+00:00').timestamp())
   steps=([(0,'/shared'),(30,'/done')] if day.endswith('01') else [(0,'/shared'),(60,'/done'),(120,'/again')]) if source=='GitHub' and person==0 else [(0,'/shared')]
   for seconds,path in steps:
    c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,referrer,country,region,city,browser,browser_version,os,os_version,device) VALUES(14,?,?,?,randomblob(16),?,?,'github.com/article','US','US: US-CA','US: 5391959','Chrome','130','Linux','6','Desktop')",(stamp+seconds,day,visitor,path,source))
c.execute("INSERT INTO import_batches VALUES(10,15,'2025-02-01','2025-02-01',0)")
c.execute("INSERT INTO import_daily VALUES(15,'2025-02-01',10,10,20)")
c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(15,1738497600,'2025-02-02',randomblob(16),randomblob(16),'/','New','US','Chrome','Desktop')")
c.commit()
`,database]);
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  const owner=await browser.newContext({storageState,viewport:{width:1440,height:1050}});
  const errors=[];
  const query='from=2025-02-02&to=2025-02-02&compare=previous&compare_match=date&sources_tab=source&interval=day';
  const base=origin+'/sites/14?'+query;
  try {
    const page=await owner.newPage();page.on('pageerror',error=>errors.push(error.message));
    page.on('console',message=>{if(message.type()==='error'&&/Content Security Policy|violates/.test(message.text()))errors.push(message.text());});
    const tooltip=page.locator('#row-comparison');
    const row=name=>page.locator(`.panel-source .data-row:has([data-row-comparison][data-name="${name}"])`);
    const value=name=>row(name).locator('[data-row-comparison]');
    await page.goto(base);
    for(const [name,old,change] of [['Grow','1','↗ 200%'],['Shrink','4','↘ 75%'],['Same','1','→ 0%'],['New','0','↗ 100%']]) {
      assert.equal(await value(name).getAttribute('data-previous'),old);
      assert.equal(await value(name).getAttribute('data-change'),change);
    }
    assert.equal(await row('Old').count(),0,'Only the primary period determines displayed rows');
    await value('Grow').hover();
    await tooltip.waitFor({state:'visible'});
    assert.equal(await tooltip.locator('.row-current-value').textContent(),'3');
    assert.equal(await tooltip.locator('.row-previous-value').textContent(),'1');
    assert.equal(await tooltip.locator('.row-previous-date').textContent(),'1 Feb 2025');
    assert.equal(await tooltip.locator('.row-tooltip-change').textContent(),'↗ 200%');
    await tooltip.hover();assert.equal(await tooltip.isVisible(),true);
    await page.mouse.move(0,0);await tooltip.waitFor({state:'hidden'});
    await row('Grow').focus();assert.equal(await tooltip.isVisible(),true);
    await page.keyboard.press('Escape');assert.equal(await tooltip.isVisible(),false);
    assert.equal(await row('Grow').evaluate(e=>e===document.activeElement),true);
    await value('Grow').click();assert.equal(await tooltip.getAttribute('role'),'dialog');
    await tooltip.getByRole('button',{name:'Close comparison'}).click();
    // Hiding the dialog can expose another row beneath the stationary pointer.
    // Allow those pointer events to run: dismissal must survive them.
    await page.evaluate(()=>new Promise(done=>requestAnimationFrame(()=>requestAnimationFrame(done))));
    assert.equal(await tooltip.isVisible(),false);
    assert.equal(await row('Grow').evaluate(element=>element===document.activeElement),true);
    await page.mouse.move(0,0);
    await value('New').hover();await tooltip.waitFor({state:'visible'});
    assert.equal(await tooltip.locator('.row-tooltip-heading strong').textContent(),'New');
    await page.keyboard.press('Escape');
    await page.evaluate(()=>new Promise(done=>requestAnimationFrame(()=>requestAnimationFrame(done))));
    assert.equal(await tooltip.isVisible(),false);
    await page.locator('.panel-source .expand-report').click();
    const github=page.locator('.report-table tbody tr').filter({has:page.getByRole('link',{name:'GitHub',exact:true})});
    const bounce=github.locator('[data-heading="Bounce rate"]');
    assert.equal(await bounce.getAttribute('data-current'),'67%');
    assert.equal(await bounce.getAttribute('data-previous'),'50%');
    assert.equal(await bounce.getAttribute('data-change'),'↗ 17 pp');
    assert.equal(await bounce.getAttribute('data-tone'),'negative');
    const newSource=page.locator('.report-table tbody tr').filter({has:page.getByRole('link',{name:'New',exact:true})});
    const newBounce=newSource.locator('[data-heading="Bounce rate"]');
    assert.equal(await newBounce.getAttribute('data-previous'),'0%');
    assert.equal(await newBounce.getAttribute('data-change'),'');
    const newDuration=newSource.locator('[data-heading="Visit duration"]');
    assert.equal(await newDuration.getAttribute('data-previous'),'—');
    assert.equal(await newDuration.getAttribute('data-change'),'');
    await newDuration.hover();await tooltip.waitFor({state:'visible'});
    assert.equal(await tooltip.locator('.row-previous-value').textContent(),'—');
    assert.equal(await tooltip.locator('.row-tooltip-change').textContent(),'');
    await page.screenshot({path:join(output,'zero-baseline-new-source-light.png'),fullPage:true});
    await bounce.focus();await page.keyboard.press('Enter');
    assert.equal(await tooltip.getAttribute('role'),'dialog');
    await page.keyboard.press('Escape');assert.equal(await bounce.evaluate(e=>e===document.activeElement),true);
    await bounce.hover();
    await page.screenshot({path:join(output,'row-comparison-expanded-light.png'),fullPage:true});
    for(const extra of ['pages_tab=entry','pages_tab=exit','locations_tab=region','locations_tab=city','devices_tab=os','devices_tab=device','browser=Chrome','os=Linux']) {
      await page.goto(base+'&'+extra);
      const comparisons=page.locator('.report-panel [data-row-comparison]');
      assert.ok(await comparisons.count()>0);
      assert.equal(await comparisons.evaluateAll(es=>es.some(e=>e.dataset.previous==='—')),false,extra);
    }
    await page.goto(base+'&browser=Chrome');
    await row('Grow').focus();await page.keyboard.press('Escape');
    assert.equal(new URL(page.url()).searchParams.get('browser'),'Chrome','Escape closes details before clearing audience filters');
    await row('Grow').locator('.row-label').click();
    assert.equal(await page.locator('.panel-referrer [data-row-comparison]').getAttribute('data-previous'),'1');
    await page.goto(base.replace('compare=previous','compare=off'));
    assert.equal(await page.locator('[data-row-comparison]').count(),0);
    assert.equal(await page.locator('#row-comparison').count(),0);
    for(const [date,old,expected,bounceChange] of [
      ['2025-02-02','2025-01-31','↗ 100%',''],
      ['2025-01-31','2025-01-30','→ 0%',''],
      ['2025-01-31','2025-02-02','↘ 100%','↘ 89 pp']
    ]) {
      const zeroQuery=`from=${date}&to=${date}&compare=custom&compare_from=${old}&compare_to=${old}&interval=day`;
      await page.goto(origin+'/sites/14?'+zeroQuery+'&metric=visitors');
      const visitors=page.getByRole('link',{name:'Show visitors',exact:true});
      assert.equal(await visitors.locator('.metric-change').textContent(),expected);
      const point=page.locator('.chart-hit').first();
      assert.equal(await point.getAttribute('data-change'),expected);
      await point.focus();
      const chartTooltip=page.locator('.chart-tooltip');
      await chartTooltip.waitFor({state:'visible'});
      assert.ok((await chartTooltip.textContent()).includes(expected));
      if(old==='2025-02-02') await page.screenshot({path:join(output,'zero-baseline-empty-current-light.png'),fullPage:true});
      await page.goto(origin+'/sites/14?'+zeroQuery+'&metric=bounce_rate');
      assert.equal(await page.locator('.chart-hit').first().getAttribute('data-change'),bounceChange);
    }
    await page.goto(origin+'/sites/15?'+query);
    const history=page.locator('.panel-source [data-row-comparison]');
    assert.equal(await history.getAttribute('data-previous'),'—');
    assert.equal(await history.getAttribute('data-change'),'');
    execFileSync('python3',['-c',"import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.execute('INSERT INTO import_files VALUES(10,?,?)',('sources',sys.argv[2]));c.commit()",database,JSON.stringify(['source','visitors','pageviews'])]);
    await page.reload();assert.equal(await history.getAttribute('data-previous'),'0');
    assert.equal(await history.getAttribute('data-change'),'↗ 100%');
    assert.deepEqual(errors,[]);
  } finally {await owner.close();}
  const touch=await browser.newContext({storageState,viewport:{width:320,height:720},isMobile:true,hasTouch:true,colorScheme:'dark'});
  try {
    const page=await touch.newPage();await page.goto(base);
    const value=page.locator('.panel-source [data-row-comparison][data-name="Grow"]');
    await value.tap();const dialog=page.getByRole('dialog',{name:'Row comparison'});await dialog.waitFor();
    const box=await dialog.boundingBox();assert.ok(box.x>=0&&box.x+box.width<=320&&box.y>=0&&box.y+box.height<=720);
    assert.equal(new URL(page.url()).searchParams.has('filters'),false,'Tapping the number opens details without applying a filter');
    await page.screenshot({path:join(output,'row-comparison-touch-dark-320.png')});
    await dialog.getByRole('button',{name:'Close comparison'}).tap();await dialog.waitFor({state:'hidden'});
    await page.locator('.panel-source .data-row').filter({hasText:'Grow'}).locator('.row-label').tap();
    await page.locator('.panel-referrer').waitFor({state:'visible'});
    assert.equal(await page.locator('.panel-referrer').count(),1,'The source label still applies its filter');
  } finally {await touch.close();}
  const noScript=await browser.newContext({storageState,javaScriptEnabled:false});
  try {
    const page=await noScript.newPage();await page.goto(base);
    const value=page.locator('.panel-source [data-row-comparison][data-name="Grow"]');
    assert.match(await value.getAttribute('title'),/2 Feb 2025: 3\n1 Feb 2025: 1\nUp 200 percent/);
    await page.locator('.panel-source .expand-report').click();
    const count=page.locator('[data-row-comparison][data-name="Grow"][data-heading="Visitors"]');
    await count.focus();assert.match(await count.getAttribute('aria-label'),/Up 200 percent/);
  } finally {await noScript.close();}
}
