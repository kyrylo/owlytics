import assert from 'node:assert/strict';
import {selectCalendarRange} from './calendar-helpers.mjs';
import {join} from 'node:path';

export async function checkPreferences({context,browser,origin,output}) {
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  const owner=await browser.newContext({storageState,locale:'en-GB',viewport:{width:1440,height:1050}});
  try {
    const page=await owner.newPage(),errors=[],external=[];
    page.on('pageerror',e=>errors.push(e.message));
    page.on('request',r=>{if(!r.url().startsWith(origin))external.push(r.url());});
    const ready=()=>page.waitForFunction(()=>document.documentElement.hasAttribute('data-dashboard-enhanced'));
    const visit=async path=>{await page.goto(origin+path);await ready();};
    const stored=()=>page.evaluate(()=>JSON.parse(localStorage.getItem('owlytics:dashboard:example.com')));
    const choose=async selector=>{await page.locator(selector).click();await ready();};
    const interval=async label=>{await page.locator('.dashboard-menu>summary').click();await page.getByRole('link',{name:label,exact:true}).click();await ready();};
    const activeInterval=()=>page.locator('[data-preference=interval][aria-current=true]').textContent();
    await visit('/sites/1?period=28d&compare=off');
    await choose('.metric[data-preference=metric][aria-label="Show pageviews"]');
    await choose('[data-preference=pages_tab][href*="pages_tab=entry"]');
    await choose('[data-preference=locations_tab][href*="locations_tab=city"]');
    await choose('[data-preference=devices_tab][href*="devices_tab=os"]');
    await choose('[data-preference=sources_tab][href*="sources_tab=channel"]');
    await interval('Weeks');
    assert.deepEqual(await stored(),{period:'28d',compare:'off',metric:'pageviews',pages_tab:'entry',locations_tab:'city',devices_tab:'os',sources_tab:'channel',intervals:{'28d':'week'}});
    await visit('/sites/1');
    assert.equal(await page.locator('.metric[aria-current=true]').getAttribute('aria-label'),'Show pageviews');
    for(const name of ['entry','city','os','channel']) assert.equal(await page.locator('.panel-'+name).count(),1);
    assert.match(await activeInterval(),/^Weeks/);
    await visit('/sites/2');
    assert.equal(new URL(page.url()).search,'','Report preferences stay with their website');
    await visit('/sites/1?period=7d&metric=visits&pages_tab=exit&locations_tab=country&devices_tab=browser&sources_tab=source');
    assert.equal(await page.locator('.metric[aria-current=true]').getAttribute('aria-label'),'Show total visits');
    for(const name of ['exit','country','browser','source']) assert.equal(await page.locator('.panel-'+name).count(),1);
    assert.match(await activeInterval(),/^Days/);
    await interval('Hours');
    await page.locator('.date-picker>summary').click();await page.getByRole('link',{name:'Last 28 days',exact:true}).click();await ready();
    assert.match(await activeInterval(),/^Weeks/,'Each date preset restores its own interval');
    assert.deepEqual((await stored()).intervals,{'28d':'week','7d':'hour'});
    await visit('/sites/1?period=7d');assert.match(await activeInterval(),/^Hours/);
    await visit('/sites/1?period=day');await interval('Minutes');
    await page.locator('.date-picker>summary').click();await page.getByRole('link',{name:'Yesterday',exact:true}).click();await ready();
    assert.match(await activeInterval(),/^Minutes/,'Today and Yesterday share the calendar-day preference');
    assert.equal((await stored()).intervals.day,'minute');
    await visit('/sites/1?period=month');await interval('Weeks');
    await page.locator('.date-picker>summary').click();await page.getByRole('link',{name:'Last month',exact:true}).click();await ready();
    assert.match(await activeInterval(),/^Weeks/,'Calendar months share their interval preference');
    await page.locator('.date-picker>summary').click();await page.getByText('Custom range',{exact:true}).click();
    const recent=offset=>new Date(Date.now()+offset*86400000).toISOString().slice(0,10);
    await selectCalendarRange(page,recent(-2),recent(-1));await ready();
    assert.match(await activeInterval(),/^Days/,'Entering custom dates drops the preset interval');
    // Explicit filters and expanded reports take priority over preferred tabs.
    await visit('/sites/1?period=28d&country=US&browser=Chrome&source=google.com&detail=exit');
    for(const name of ['region','browser_version','referrer','exit']) assert.equal(await page.locator('.panel-'+name).count(),1,name);
    assert.equal(await page.locator('.expanded-report').count(),1);
    await visit('/sites/1?period=28d&page=%2Fdocs&interval=day');
    await page.getByRole('link',{name:'Show scroll depth',exact:true}).click();await ready();
    assert.equal((await stored()).metric,'scroll_depth');
    await visit('/sites/1?period=28d');
    assert.equal(await page.locator('.metric[aria-current=true]').getAttribute('aria-label'),'Show visitors');
    assert.equal((await stored()).metric,'visitors','An unavailable saved metric falls back to visitors');
    // A custom range uses calendar thresholds, not an inherited preset interval.
    await visit('/sites/1?from=2026-09-01&to=2026-10-01');assert.match(await activeInterval(),/^Weeks/);
    await interval('Months');
    await visit('/sites/1?from=2026-10-01&to=2026-10-02');assert.match(await activeInterval(),/^Days/);
    assert.equal(new URL(page.url()).searchParams.has('interval'),false,'An invalid saved custom interval is ignored');
    await visit('/sites/1?from=2026-08-01&to=2026-10-01');assert.match(await activeInterval(),/^Months/);
    await visit('/sites/1?period=year');assert.match(await activeInterval(),/^Months/);
    await visit('/sites/1?period=6mo');assert.match(await activeInterval(),/^Months/);
    // Daily imports cannot acquire invented minute values through storage.
    await page.evaluate(()=>localStorage.setItem('owlytics:dashboard:history.example',JSON.stringify({intervals:{day:'minute'}})));
    await visit('/sites/3?period=day&date=2025-10-01');
    assert.match(await activeInterval(),/^Days/);assert.equal(await page.locator('.chart-hit').count(),1);
    await visit('/sites/3?period=day&date=2025-10-01&with_imported=false');assert.match(await activeInterval(),/^Minutes/);
    // Minute buckets retain both occurrences of a repeated civil hour.
    await visit('/sites/2?period=day&date=2025-11-02&interval=minute&metric=pageviews&compare=off');
    assert.equal(await page.locator('.chart-hit').count(),1500);
    const labels=await page.locator('.chart-hit').evaluateAll(es=>es.map(e=>e.dataset.date));
    assert.ok(labels.some(v=>v?.includes('01:00 EDT')) && labels.some(v=>v?.includes('01:00 EST')));
    for(const scheme of ['light','dark']) {
      await page.emulateMedia({colorScheme:scheme});await page.setViewportSize({width:390,height:844});
      await page.locator('.dashboard-menu>summary').click();
      await page.screenshot({path:join(output,`minute-menu-${scheme}-390.png`),fullPage:true});
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
      await page.keyboard.press('Escape');
    }
    for(const value of ['null','{"intervals":null,"metric":"unknown","pages_tab":"<script>"}',JSON.stringify({intervals:{day:'javascript:',evil:'month'}})]) {
      await page.evaluate(value=>{localStorage.clear();localStorage.setItem('owlytics:dashboard:example.com',value);},value);
      await visit('/sites/1');assert.equal(new URL(page.url()).search,'');
    }
    assert.deepEqual(errors,[]);assert.deepEqual(external,[]);
  } finally {await owner.close();}
  const blocked=await browser.newContext({storageState});
  try {
    await blocked.addInitScript(()=>Object.defineProperty(window,'localStorage',{get(){throw new Error('Unavailable');}}));
    const page=await blocked.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));
    await page.goto(origin+'/sites/1?period=28d');
    await page.getByRole('link',{name:'Show pageviews',exact:true}).click();
    await page.locator('[data-preference=pages_tab][href*="pages_tab=entry"]').click();
    assert.equal(await page.locator('.panel-entry').count(),1);assert.deepEqual(errors,[]);
  } finally {await blocked.close();}
  const noScript=await browser.newContext({storageState,javaScriptEnabled:false});
  try {
    const page=await noScript.newPage();await page.goto(origin+'/sites/2?period=day&date=2025-11-02&compare=off');
    await page.locator('.dashboard-menu>summary').click();await page.getByRole('link',{name:'Minutes',exact:true}).click();
    await page.waitForURL(url=>url.searchParams.get('interval')==='minute',{waitUntil:'load'});
    assert.equal(await page.locator('.chart-hit title').count(),1500);
  } finally {await noScript.close();}
}
