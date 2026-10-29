import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

export async function checkNavigation({context,browser,origin,database,output}) {
  execFileSync('python3',['-c',`
import sqlite3,sys,datetime
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
c.execute("INSERT INTO sites VALUES(8,'calendar.example','UTC',0),(9,'calendar-dst.example','America/New_York',0),(10,'empty-calendar.example','UTC',0)")
c.execute("INSERT INTO import_batches VALUES(80,8,'2023-02-17','2023-02-17',0)")
c.execute("INSERT INTO import_daily VALUES(8,'2023-02-17',80,5,7)")
for day in ['2024-02-29','2024-03-01']:
 c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(8,unixepoch(?1)+36000,?1,randomblob(16),randomblob(16),'/docs','Google','US','Chrome','Desktop')",(day,))
c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(9,unixepoch('2025-11-03T04:10:00Z'),'2025-11-02',randomblob(16),randomblob(16),'/','','US','Chrome','Desktop')")
c.commit()
`,database]);
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  const owner=await browser.newContext({storageState,locale:'en-GB',viewport:{width:1440,height:1050}});
  try {
    const page=await owner.newPage(),errors=[],external=[];
    page.on('pageerror',error=>errors.push(error.message));
    page.on('request',request=>{if(!request.url().startsWith(origin))external.push(request.url());});
    const ready=()=>page.waitForFunction(()=>document.documentElement.hasAttribute('data-dashboard-enhanced')&&!document.querySelector('main[aria-busy=true]'));
    const visit=async(path)=>{await page.goto(origin+path);await ready();};
    const base='/sites/8?period=month&date=2024-02-29&compare=off&source=Google&with_imported=false';
    const params=()=>new URL(page.url()).searchParams;
    await visit(base+'&metric=pageviews&detail=page&search=docs&sort=name&order=asc&offset=50');
    assert.equal(await page.locator('.date-picker>summary>span').innerText(),'February 2024');
    await page.getByRole('link',{name:'Close expanded report',exact:true}).click();
    await page.getByRole('link',{name:'Following month',exact:true}).click();await ready();
    await page.locator('.panel-page .expand-report').click();await ready();
    for(const [key,value] of Object.entries({date:'2024-03-01',period:'month',source:'Google',metric:'pageviews',detail:'page',with_imported:'false',compare:'off'}))assert.equal(params().get(key),value,key);
    assert.equal(params().has('offset'),false);
    assert.equal(await page.locator('.expanded-report tbody tr').count(),1);
    await page.keyboard.press('/');
    assert.equal(await page.getByRole('searchbox',{name:'Search report'}).evaluate(e=>e===document.activeElement),true);
    const beforeTyping=page.url();await page.keyboard.press('ArrowLeft');await page.keyboard.press('1');
    assert.equal(page.url(),beforeTyping,'Date/site shortcuts do not intercept editing');
    await visit(base);
    await page.keyboard.press('ArrowLeft');await page.waitForURL(url=>url.searchParams.get('date')==='2024-01-01');
    await page.keyboard.press('ArrowRight');await page.waitForURL(url=>url.searchParams.get('date')==='2024-02-01');
    const beforeModified=page.url();await page.keyboard.press('Shift+ArrowLeft');
    assert.equal(page.url(),beforeModified);
    await page.locator('.date-picker>summary').click();await page.keyboard.press('ArrowLeft');
    assert.equal(page.url(),beforeModified,'Date arrows do not change the period inside a menu');
    assert.equal(await page.getByRole('link',{name:'Month to date',exact:true}).getAttribute('aria-current'),null,'A past month is not labelled current');
    await page.keyboard.press('Escape');
    await page.getByRole('link',{name:'Previous month',exact:true}).focus();
    await page.keyboard.press('ArrowRight');await page.waitForURL(url=>url.searchParams.get('date')==='2024-03-01');
    await visit(base);
    const hits=page.locator('.chart-hit');
    await hits.first().focus();await page.keyboard.press('ArrowRight');
    assert.equal(await hits.nth(1).evaluate(e=>e===document.activeElement),true);
    assert.equal(params().get('date'),'2024-02-29','Chart arrows inspect points without moving the date');
    await page.keyboard.press('End');await page.keyboard.press('Enter');
    await page.waitForURL(url=>url.searchParams.get('period')==='day');
    assert.equal(params().get('date'),'2024-02-29');assert.equal(params().get('source'),'Google');
    assert.equal(params().has('interval'),false);
    assert.equal(await hits.count(),24);
    assert.equal(await page.locator('a.chart-hit').count(),0,'Hourly points have no drilldown');
    await page.getByRole('link',{name:'Following day',exact:true}).click();
    assert.equal(params().get('date'),'2024-03-01');
    await page.locator('.date-picker>summary').click();await page.getByRole('link',{name:'Today',exact:true}).click();
    assert.equal(params().has('date'),false,'A preset clears the old anchor');
    assert.equal(await page.getByRole('link',{name:'Following day',exact:true}).getAttribute('aria-disabled'),'true');
    await visit('/sites/8?period=month&date=2023-02-17&with_imported=false&source=Missing&compare=off');
    assert.equal(await page.getByRole('link',{name:'Previous month',exact:true}).getAttribute('aria-disabled'),'true','Bounds include disabled/unmatched historical data');
    const bounded=page.url();await page.keyboard.press('ArrowLeft');assert.equal(page.url(),bounded);
    await visit('/sites/10?period=day&compare=off');
    assert.equal(await page.locator('.date-navigation [aria-disabled=true]').count(),2);
    for(const period of ['7d','6mo','24h','realtime','all']) {
      await visit(`/sites/8?period=${period}&compare=off`);
      assert.equal(await page.locator('.date-navigation').count(),0,period);
    }
    await visit('/sites/9?period=day&date=2025-11-02&compare=off');
    assert.equal(await hits.count(),25,'An anchored date uses the site timezone across DST');
    assert.match(await hits.last().getAttribute('data-date'),/23:00 EST/);
    // A stored period must not override a link that specifies only a date.
    await page.evaluate(()=>localStorage.setItem('owlytics:dashboard:calendar.example',JSON.stringify({period:'year'})));
    await visit('/sites/8?date=2024-02-29&compare=off');
    assert.equal(params().has('period'),false);assert.equal(await hits.count(),24);

    await visit('/sites/8?from=2024-02-01&to=2024-03-31&interval=month&compare=off&source=Google');
    assert.equal(await hits.count(),2);
    await hits.first().hover();await hits.first().click();
    await page.waitForURL(url=>url.searchParams.get('period')==='month');
    assert.equal(params().get('date'),'2024-02-01');assert.equal(await hits.count(),29);
    await hits.last().click({button:'right'});
    const pinned=page.getByRole('dialog',{name:'Chart details'});
    await page.mouse.move(0,0);assert.equal(await pinned.isVisible(),true);
    await pinned.getByRole('link',{name:'View day'}).focus();await page.keyboard.press('Escape');
    assert.equal(await page.locator('.chart-tooltip').isVisible(),false);
    assert.equal(await hits.last().evaluate(e=>e===document.activeElement),true);
    await hits.last().click({button:'right'});
    await pinned.getByRole('link',{name:'View day'}).click();await page.waitForURL(url=>url.searchParams.get('period')==='day');
    assert.equal(params().get('date'),'2024-02-29');

    // Date navigation and native SVG links merge a filter still in flight.
    for(const navigation of ['arrow','chart']) {
      await visit('/sites/8?period=month&date=2024-02-01&compare=off&filter=source');
      let release,begin;const held=new Promise(resolve=>release=resolve),arrived=new Promise(resolve=>begin=resolve);
      await owner.route('**/sites/8/filter?**',async route=>{begin();await held;await route.continue();});
      await page.locator('.filter-values input[value=Google]').check();
      await arrived;
      if(navigation==='arrow')await page.getByRole('link',{name:'Following month',exact:true}).click();
      else await page.locator('a.chart-hit').last().click();
      release();
      await page.waitForURL(url=>url.searchParams.get('date')===(navigation==='arrow'?'2024-03-01':'2024-02-29'));
      await ready();await owner.unroute('**/sites/8/filter?**');
      assert.deepEqual(JSON.parse(params().get('filters')),[{key:'source',op:'is',values:['Google']}]);
      assert.equal(await page.locator('.filter-picker').count(),0);
    }
    await visit(base);
    const firstSite=await page.locator('.site-picker a[data-shortcut="1"]').getAttribute('href');
    await page.keyboard.press('1');await page.waitForURL(url=>url.pathname===firstSite);
    for(const colorScheme of ['light','dark']) {
      await page.emulateMedia({colorScheme});
      for(const width of [1440,768,390,320]) {
        await page.setViewportSize({width,height:1000});
        await visit(base.replace('compare=off','compare=previous')+'&compare_match=date');
        assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
        await page.screenshot({path:join(output,`calendar-${colorScheme}-${width}.png`),fullPage:true});
      }
    }
    const touch=await browser.newContext({storageState,hasTouch:true,isMobile:true,viewport:{width:390,height:844}});
    try {
      const tap=await touch.newPage();await tap.goto(origin+base);
      const beforeTap=tap.url();await tap.locator('.chart-hit').last().tap();
      assert.equal(tap.url(),beforeTap,'Touch pins details before choosing a drilldown');
      const details=tap.getByRole('dialog',{name:'Chart details'});assert.equal(await details.isVisible(),true);
      await tap.screenshot({path:join(output,'calendar-touch-drilldown.png')});
      await details.getByRole('link',{name:'View day'}).tap();await tap.waitForURL(url=>url.searchParams.get('period')==='day');
      assert.equal(new URL(tap.url()).searchParams.get('date'),'2024-02-29');
    } finally {await touch.close();}
    const noScript=await browser.newContext({storageState,javaScriptEnabled:false});
    try {
      const fallback=await noScript.newPage();await fallback.goto(origin+base);
      await fallback.locator('a.chart-hit').last().focus();await fallback.keyboard.press('Enter');
      await fallback.waitForURL(url=>url.searchParams.get('period')==='day');
      assert.equal(new URL(fallback.url()).searchParams.get('date'),'2024-02-29');
      await fallback.getByRole('link',{name:'Following day',exact:true}).click();
      assert.equal(new URL(fallback.url()).searchParams.get('date'),'2024-03-01');
    } finally {await noScript.close();}
    assert.deepEqual(errors,[]);assert.deepEqual(external,[]);
  } finally {await owner.close();}
}
