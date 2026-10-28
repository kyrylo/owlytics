import assert from 'node:assert/strict';
import {join} from 'node:path';

export async function checkMenus({context,browser,origin,output}) {
  // Cookies authenticate the owner; each case starts without saved preferences.
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  const owner=await browser.newContext({storageState,viewport:{width:1440,height:1050}});
  try {
    const page=await owner.newPage(), errors=[];
    page.on('pageerror',error=>errors.push(error.message));
    const ready=()=>page.waitForFunction(()=>document.documentElement.hasAttribute('data-dashboard-enhanced'));
    const visit=async path=>{await page.goto(origin+path);await ready();};
    const date=offset=>new Date(Date.now()+offset*86400000).toISOString().slice(0,10);
    const settings=()=>page.evaluate(()=>JSON.parse(localStorage.getItem('owlytics:dashboard:example.com')));
    const choose=async(menu,label)=>{await page.locator(`.${menu}>summary`).click();await page.getByRole('link',{name:label,exact:true}).click();await ready();};
    await visit('/sites/1');
    assert.equal(await page.locator('.comparison-picker').count(),0,'Comparison starts disabled');
    assert.equal(await page.evaluate(()=>localStorage.length),0,'Opening a default dashboard does not create preferences');
    assert.equal(await page.getByRole('switch',{name:'Include imported data',includeHidden:true}).count(),0,'No import control for sites without history');
    await page.keyboard.press('s');await page.waitForURL(url=>url.searchParams.get('period')==='6mo');await ready();
    assert.equal(new URL(page.url()).searchParams.get('period'),'6mo');
    await page.keyboard.press('x');await page.waitForURL(url=>url.searchParams.get('compare')==='previous');await ready();
    assert.equal(await page.locator('.comparison-picker .dropdown-content a[aria-current=true]').filter({hasText:'Match day of week'}).count(),1);
    await choose('comparison-picker','Year over year');
    await choose('comparison-picker','Match exact date');
    assert.deepEqual(await settings(),{period:'6mo',compare:'year',compare_match:'date'});
    await visit('/sites/1');
    assert.equal(new URL(page.url()).searchParams.get('period'),'6mo');
    assert.equal(new URL(page.url()).searchParams.get('compare'),'year');
    assert.match(await page.locator('.comparison-picker>summary').innerText(),/vs\.\s+Year over year/);
    await visit('/sites/2');
    assert.equal(new URL(page.url()).search,'','Choices are scoped to the website');
    assert.equal(await page.locator('.comparison-picker').count(),0);
    await visit(`/sites/1?from=${date(-7)}&to=${date(-5)}&compare=off&with_imported=false&browser=Chrome`);
    assert.equal(new URL(page.url()).searchParams.has('period'),false,'A custom range wins over the saved preset');
    assert.equal(await page.locator('.chart-hit').count(),3);
    assert.equal(await page.locator('.comparison-picker').count(),0);
    assert.deepEqual(await settings(),{period:'6mo',compare:'off',compare_match:'date'},'Audience filters and import visibility are not saved');
    await visit('/sites/1?from=2025-10-01&to=2025-10-03&compare=custom&compare_from=2025-09-01&compare_to=2025-09-03');
    assert.equal((await settings()).compare,'off','Custom comparisons do not replace the saved mode');
    await visit('/sites/1?period=realtime&compare=year');
    assert.equal(await page.locator('.comparison-picker').count(),0);
    assert.equal((await settings()).period,'6mo','Realtime is not restored on the next visit');
    await visit('/sites/1?period=all&compare=previous');
    assert.equal(await page.locator('.comparison-picker').count(),0,'All time has no comparison');
    await choose('date-picker','Last 30 days');
    assert.equal(await page.locator('.chart-hit').count(),30);
    assert.equal((await settings()).period,'30d');
    await page.evaluate(()=>localStorage.setItem('owlytics:dashboard:example.com','{"period":"bad","compare":"custom","compare_match":"bad","with_imported":false}'));
    await visit('/sites/1');
    assert.equal(new URL(page.url()).search,'','Invalid or unsupported stored values are ignored');
    for(const corrupt of ['{','null','["day"]']) {
      await page.evaluate(value=>localStorage.setItem('owlytics:dashboard:example.com',value),corrupt);
      await visit('/sites/1');
      assert.equal(await page.locator('.comparison-picker').count(),0);
    }
    await page.evaluate(()=>localStorage.clear());
    await choose('date-picker','Last 30 days');
    for(const colorScheme of ['light','dark']) {
      await page.emulateMedia({colorScheme});await page.setViewportSize({width:320,height:844});
      await page.locator('.date-picker>summary').click();
      const menu=page.locator('.date-picker>.dropdown-content');
      assert.ok(await menu.evaluate(element=>{const b=element.getBoundingClientRect();return b.left>=0&&b.right<=innerWidth+1&&b.bottom<=innerHeight;}));
      await page.screenshot({path:join(output,`date-presets-${colorScheme}-320.png`)});
      await page.keyboard.press('Escape');
    }
    await page.keyboard.press('c');
    assert.equal(await page.getByLabel('From',{exact:true}).evaluate(element=>element===document.activeElement),true);
    await page.keyboard.press('s');
    assert.equal(new URL(page.url()).searchParams.get('period'),'30d','Shortcuts do not fire while editing dates');
    await page.keyboard.press('Escape');await page.keyboard.press('i');
    assert.equal(await page.locator('.dashboard-menu').getAttribute('open'),'');
    assert.deepEqual(errors,[]);
  } finally {await owner.close();}
  const blocked=await browser.newContext({storageState});
  try {
    await blocked.addInitScript(()=>Object.defineProperty(window,'localStorage',{get(){throw new Error('Storage unavailable');}}));
    const page=await blocked.newPage(), errors=[];page.on('pageerror',error=>errors.push(error.message));
    await page.goto(`${origin}/sites/1?period=30d&compare=previous`);
    await page.locator('.comparison-picker>summary').click();
    await page.getByRole('link',{name:'Disable comparison',exact:true}).click();
    assert.equal(await page.locator('.comparison-picker').count(),0);
    assert.deepEqual(errors,[],'Unavailable storage cannot break navigation');
  } finally {await blocked.close();}
}
