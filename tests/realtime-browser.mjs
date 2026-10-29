import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {setTimeout as delay} from 'node:timers/promises';
import {join} from 'node:path';

export async function checkRealtime({context,browser,origin,database,output}) {
  const sql=script=>execFileSync('python3',['-c',`import sqlite3,sys,time,datetime,hashlib
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
${script}
c.commit()
`,database]);
  sql(`c.execute("INSERT INTO sites VALUES(6,'live.example','UTC',0),(7,'empty-live.example','UTC',0)")
now=int(time.time())
for i,(person,offset,path,source,browser) in enumerate([(1,-1200,'/older','OldOnly','Firefox'),(2,-90,'/active','Google','Chrome'),(3,-60,'/recent','GitHub','Safari'),(2,-90,'/active','Google','Chrome')]):
 stamp=now+offset;day=datetime.datetime.fromtimestamp(stamp,datetime.timezone.utc).date().isoformat()
 c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,referrer,country,browser,browser_version,os,os_version,device) VALUES(6,?,?,?,?,?,?,?,'US',?,'1','Linux','6','Desktop')",(stamp,day,bytes([person])*16,bytes([i])*16,path,source,source+'.com/x',browser))
`);
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  const owner=await browser.newContext({storageState,viewport:{width:1440,height:1050}});
  const errors=[],external=[],fetches=[];
  try {
    const page=await owner.newPage();
    page.on('pageerror',error=>errors.push(error.message));
    page.on('request',request=>{
      if(!request.url().startsWith(origin)) external.push(request.url());
      if(request.resourceType()==='fetch') fetches.push(request.url());
    });
    const until=async(predicate,label)=>{
      const deadline=Date.now()+8000;
      while(!(await predicate())) {assert.ok(Date.now()<deadline,label);await delay(20);}
    };
    const ready=()=>until(()=>page.evaluate(()=>document.documentElement.hasAttribute('data-dashboard-enhanced')&&!document.querySelector('main[aria-busy=true]')),'dashboard ready');
    const base=`${origin}/sites/6?period=realtime&compare=off`;
    const visit=async(url=base)=>{await page.goto(url);await ready();};
    const counts=()=>page.locator('.metrics .metric').evaluateAll(cards=>cards.map(card=>Number(card.dataset.value)));
    const blur=()=>page.evaluate(()=>document.activeElement?.blur());
    await visit(`${base}&metric=bounce_rate`);
    assert.deepEqual(await counts(),[2,3,4]);
    assert.equal(await page.locator('.metrics a.metric').count(),2);
    assert.equal(await page.locator('.comparison-picker,.current-visitors').count(),0);
    assert.deepEqual(await page.locator('.chart-labels span').allTextContents(),['-30m','-25m','-20m','-15m','-10m','-5m']);
    assert.equal(await page.locator('.chart-hit').count(),30);
    assert.match(await page.locator('.panel-page').innerText(),/Current visitors/);
    assert.doesNotMatch(await page.locator('.report-grid').innerText(),/OldOnly|\/older|Firefox/);
    await page.locator('.chart-hit').first().focus();
    assert.equal(await page.locator('.chart-tooltip-current span').innerText(),'30 minutes ago');
    await page.keyboard.press('End');
    assert.equal(await page.locator('.chart-tooltip-current span').innerText(),'1 minute ago');
    await page.keyboard.press('Escape');await blur();
    await page.getByRole('link',{name:'Show pageviews',exact:true}).click();
    assert.match(await page.locator('.chart').getAttribute('aria-label'),/pageviews/i);
    await page.locator('.panel-page .expand-report').click();
    assert.equal(await page.locator('.expanded-report tbody tr').count(),2);
    assert.match(await page.locator('.expanded-report thead').innerText(),/Current visitors/);
    await visit(`${base}&source=OldOnly`);
    assert.deepEqual(await counts(),[2,1,1]);
    assert.equal(await page.locator('.data-row').count(),0);
    await visit(`${base}&source=Missing`);
    assert.deepEqual(await counts(),[2,0,0]);
    assert.equal(await page.locator('.chart-hit').count(),30);
    await visit(`${origin}/sites/7?period=realtime`);
    assert.deepEqual(await counts(),[0,0,0]);
    assert.equal(await page.locator('.chart-hit').count(),30);
    await visit();
    for(const colorScheme of ['light','dark']) {
      await page.emulateMedia({colorScheme});
      for(const width of [1440,768,390,320]) {
        await page.setViewportSize({width,height:900});
        assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`${colorScheme}/${width} overflow`);
        await page.screenshot({path:join(output,`realtime-${colorScheme}-${width}.png`),fullPage:true});
      }
    }
    await page.setViewportSize({width:1440,height:1050});
    await page.emulateMedia({colorScheme:'light'});

    // Browser time controls only refresh scheduling. The server continues to
    // query its real clock, and real database writes prove each refreshed view.
    const start=new Date('2030-01-01T00:00:00Z');
    // Freeze before loading the dashboard. A slow navigation must not overrun
    // pauseAt's target and turn refresh assertions into a wall-clock race.
    await page.goto('about:blank');
    await page.clock.install({time:new Date(start.getTime()-3600000)});
    await page.clock.pauseAt(start);await visit();
    await page.clock.fastForward(1000);
    await page.mouse.move(0,0);await blur();
    const history=await page.evaluate(()=>window.history.length),url=page.url();
    const initialFetches=fetches.length;
    await page.clock.fastForward(28000);
    assert.equal(fetches.length,initialFetches,'Refresh waits thirty seconds');
    sql(`stamp=int(time.time());day=datetime.datetime.fromtimestamp(stamp,datetime.timezone.utc).date().isoformat()
c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(6,?,?,randomblob(16),randomblob(16),'/new','New','DE','Chrome','Desktop')",(stamp,day))`);
    await page.clock.fastForward(2000);
    await until(async()=>JSON.stringify(await counts())==='[3,4,5]','thirty-second refresh shows new visitor');
    assert.equal(fetches.length,initialFetches+1);
    assert.equal(page.url(),url);assert.equal(await page.evaluate(()=>window.history.length),history);
    const refresh=async()=>{
      await page.evaluate(()=>document.querySelector('main').dataset.beforeRefresh='');
      const before=fetches.length;
      await page.clock.fastForward(30000);
      await until(()=>page.evaluate(()=>!document.querySelector('main[data-before-refresh]')),'refresh replaces dashboard');
      assert.equal(fetches.length,before+1,'Only one timer survives dashboard replacement');
      assert.equal(await page.locator('.live-status').innerText(),'Live');
    };
    await refresh();

    // Do not steal focus, discard an unfinished edit, or hide chart details.
    for(const control of ['menu','chart','input']) {
      if(control==='menu') await page.locator('.date-picker>summary').click();
      if(control==='chart') await page.locator('.chart-hit').first().focus();
      if(control==='input') {
        await visit(`${base}&detail=page`);
        await page.getByRole('searchbox',{name:'Search report'}).fill('unfinished');
        await page.clock.fastForward(250);
        await until(()=>Promise.resolve(new URL(page.url()).searchParams.get('search')==='unfinished'),'live report search finishes');
        await ready();
      }
      const before=fetches.length;
      await page.clock.fastForward(30000);
      assert.equal(fetches.length,before,`${control} pauses refresh`);
      assert.equal(await page.locator('.live-status').innerText(),'Paused');
      if(control==='input') assert.equal(await page.getByRole('searchbox',{name:'Search report'}).inputValue(),'unfinished');
      if(control!=='input')await page.keyboard.press('Escape');
      await page.mouse.move(0,0);await blur();
      if(control==='input') {
        await page.clock.fastForward(30000);
        assert.equal(fetches.length,before,'An expanded report also pauses after blur');
        await page.getByRole('link',{name:'Close expanded report',exact:true}).click();await ready();await blur();
      }
      await refresh();
    }
    await visit(`${base}&filter=source`);
    const filterFetches=fetches.length;
    await page.clock.fastForward(30000);
    assert.equal(fetches.length,filterFetches,'An open filter keeps its current edit');
    await page.getByRole('link',{name:'Done',exact:true}).click();await ready();await blur();
    await refresh();

    // Visibility is controlled here so this check also runs in headless Chrome.
    await page.evaluate(()=>{Object.defineProperty(document,'hidden',{configurable:true,get:()=>true});document.dispatchEvent(new Event('visibilitychange'));});
    const hiddenFetches=fetches.length;
    await page.clock.fastForward(30000);
    assert.equal(fetches.length,hiddenFetches);
    await page.evaluate(()=>{document.querySelector('main').dataset.beforeRefresh='';delete document.hidden;document.dispatchEvent(new Event('visibilitychange'));});
    await until(()=>page.evaluate(()=>!document.querySelector('main[data-before-refresh]')),'visibility resumes refresh');

    const pattern=`${origin}/sites/6?**`;
    await owner.route(pattern,route=>route.request().resourceType()==='fetch'?route.fulfill({status:503,body:'Temporarily unavailable'}):route.continue());
    const beforeError=await counts();
    await page.clock.fastForward(30000);
    await until(async()=>await page.locator('.live-status').innerText()==='Reconnecting…','failed refresh indicator');
    assert.deepEqual(await counts(),beforeError,'A failed refresh keeps the last useful report');
    await owner.unroute(pattern);await refresh();

    let release;
    const held=new Promise(resolve=>release=resolve);
    await owner.route(pattern,async route=>{
      if(route.request().resourceType()!=='fetch')return route.continue();
      await held;await route.continue().catch(()=>{});
    });
    const beforeTimeout=fetches.length;
    await page.clock.fastForward(30000);
    await until(()=>fetches.length===beforeTimeout+1,'held refresh starts');
    await page.clock.fastForward(15000);
    await until(async()=>await page.locator('.live-status').innerText()==='Reconnecting…','timeout indicator');
    assert.deepEqual(await counts(),beforeError);
    release();await owner.unroute(pattern);await refresh();

    // A late refresh cannot replace a filter selected while it was in flight.
    let staleRelease,started=false;
    const stale=new Promise(resolve=>staleRelease=resolve);
    await owner.route(pattern,async route=>{
      if(route.request().resourceType()==='fetch'&&route.request().url()===page.url()&&!started) {
        started=true;const response=await route.fetch();await stale;
        await route.fulfill({response}).catch(()=>{});
      } else await route.continue();
    });
    await page.clock.fastForward(30000);await until(()=>started,'stale refresh starts');
    await page.locator('.panel-source .data-row').filter({hasText:'Google'}).click();
    await ready();staleRelease();await owner.unroute(pattern);
    await until(async()=>JSON.stringify(await counts())==='[3,1,2]','filter wins over stale refresh');
    assert.equal(new URL(page.url()).searchParams.get('source'),'Google');
    await blur();await page.mouse.move(0,0);await refresh();
    assert.deepEqual(await counts(),[3,1,2]);
    await visit(`${origin}/sites/6?period=28d&compare=off`);
    const normalFetches=fetches.length;
    await page.clock.fastForward(60000);
    assert.equal(fetches.length,normalFetches,'Normal reports do not auto-refresh');

    const noScript=await browser.newContext({storageState,javaScriptEnabled:false});
    try {
      const fallback=await noScript.newPage();await fallback.goto(base);
      assert.deepEqual(await fallback.locator('.metrics .metric').evaluateAll(cards=>cards.map(card=>Number(card.dataset.value))),[3,4,5]);
      assert.equal(await fallback.locator('.chart-hit title').count(),30);
      assert.equal(await fallback.locator('.live-status').isVisible(),false,'Manual reload does not claim to be live');
      await fallback.locator('.panel-source .data-row').filter({hasText:'Google'}).click();
      assert.deepEqual(await fallback.locator('.metrics .metric').evaluateAll(cards=>cards.map(card=>Number(card.dataset.value))),[3,1,2]);
    } finally {await noScript.close();}
    await visit();await blur();await page.mouse.move(0,0);
    await owner.clearCookies();
    await page.clock.fastForward(30000);
    await page.waitForURL(`${origin}/login`);
    assert.equal(await page.getByRole('button',{name:'Sign in',exact:true}).count(),1,'Expired owner sessions return to login');
    assert.deepEqual(errors,[]);assert.deepEqual(external,[]);
  } finally {await owner.close();}
}
