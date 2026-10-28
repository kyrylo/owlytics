import assert from 'node:assert/strict';
import {join} from 'node:path';

export async function checkFilters({page, context, browser, origin, output}) {
  const idle=()=>page.waitForFunction(()=>!document.querySelector('main[aria-busy=true]'));
  const values=()=>page.getByLabel('Search filter values',{exact:true});
  const group=()=>JSON.parse(new URL(page.url()).searchParams.get('filters')||'[]');
  const visitors=async()=>Number(await page.locator('.metric[data-value]').first().getAttribute('data-value'));
  await page.goto(`${origin}/sites/1`);
  const all=await visitors();
  await page.locator('.filter-menu-toggle>summary').click();
  await page.locator('.filter-group>summary').filter({hasText:/^Page/}).click();
  await page.locator('[data-filter-link="open:page"]').click();await idle();
  assert.equal(await page.locator('.filter-picker').count(),1);
  assert.equal(await values().evaluate(input=>input===document.activeElement),true);

  // Two changes and a search during a slow request must all survive in order.
  await context.route('**/sites/1/filter?**',async route=>{await new Promise(resolve=>setTimeout(resolve,150));await route.continue();});
  await page.evaluate(()=>{
    for(const value of ['/','/pricing']) {
      const input=[...document.querySelectorAll('.filter-values input[type=checkbox]')].find(input=>input.value===value);
      input.checked=true;input.dispatchEvent(new Event('change',{bubbles:true}));
    }
  });
  await values().fill('/article/109');
  await page.waitForURL(url=>url.searchParams.get('filter_search')==='/article/109');await idle();
  assert.deepEqual(group()[0].values,['/','/pricing']);
  for(const value of ['/','/pricing']) assert.equal(await page.locator(`.filter-values input[type=checkbox][value="${value}"]`).isChecked(),true);
  await page.locator('.filter-values input[type=checkbox][value="/article/109"]').check();await idle();
  assert.deepEqual(group()[0].values,['/','/pricing','/article/109']);
  assert.equal(await page.locator('.filter').count(),1);
  assert.match(await page.locator('.filter-selected').innerText(),/\/article\/109/);
  await context.unroute('**/sites/1/filter?**');
  await values().fill('');await page.waitForURL(url=>url.searchParams.get('filter_search')==='');await idle();
  await page.locator('[data-filter-link="page:Next"]').click();await idle();
  assert.equal(new URL(page.url()).searchParams.get('filter_offset'),'50');
  assert.equal(await page.locator('.filter-values input:checked').count(),3);
  await page.getByRole('link',{name:'Edit Page operator',exact:true}).click();await idle();
  assert.deepEqual(await page.locator('.operator-options button').allTextContents(),['is ✓','is not','contains','does not contain']);
  await page.getByRole('button',{name:'contains',exact:true}).click();await idle();
  await page.evaluate(()=>{
    for(const input of document.querySelectorAll('.filter-values input:checked')) {
      input.checked=false;input.dispatchEvent(new Event('change',{bubbles:true}));
    }
  });await idle();
  await page.getByLabel('Text to match',{exact:true}).fill('article');
  await page.getByRole('button',{name:'Add',exact:true}).click();await idle();
  assert.deepEqual(group(),[{key:'page',op:'contains',values:['article']}]);
  assert.equal(await visitors(),110);
  assert.equal(await page.locator('.chart[data-interactive=true]').count(),1);

  for(const width of [320,390,768,1440]) for(const colorScheme of ['light','dark']) {
    await page.setViewportSize({width,height:1050});await page.emulateMedia({colorScheme});
    await page.screenshot({path:join(output,`filters-${colorScheme}-${width}.png`),fullPage:true});
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`Filter overflow at ${width}`);
    assert.ok(await page.locator('.filter-picker').evaluate(element=>{const box=element.getBoundingClientRect();return box.left>=0&&box.right<=innerWidth+1;}),'Picker stays in viewport');
  }
  await page.getByRole('link',{name:'Edit Page operator',exact:true}).click();await idle();
  await page.getByRole('button',{name:'does not contain',exact:true}).click();await idle();
  assert.equal(await visitors(),all-110);
  await page.keyboard.press('Escape');await idle();
  assert.equal(await page.locator('.filter-picker').count(),0);
  assert.equal(group()[0].op,'contains_not');
  await page.keyboard.press('Escape');await idle();
  assert.equal(group().length,0);assert.equal(await visitors(),all);
  await page.goBack();await page.waitForURL(url=>url.searchParams.has('filters'));
  await page.locator('.filter-selected').waitFor();
  assert.equal(group()[0].op,'contains_not');
  assert.equal(await visitors(),all-110);
  await page.locator('.chart-hit').first().hover();
  assert.equal(await page.getByRole('tooltip').isVisible(),true);
  await page.keyboard.press('Escape');await idle();
  assert.equal(group().length,1,'Chart Escape closes the tooltip before clearing filters');

  await page.goto(`${origin}/sites/1?filter=browser`);
  await context.route('**/sites/1/filter?**',async route=>{await new Promise(resolve=>setTimeout(resolve,150));await route.continue();});
  await page.locator('.filter-values input[type=checkbox][value="Chrome"]').check();
  await page.getByRole('link',{name:'Show pageviews',exact:true}).click();
  await page.waitForURL(url=>url.searchParams.get('metric')==='pageviews');
  await page.locator('.filter-selected').waitFor();
  assert.deepEqual(group(),[{key:'browser',op:'is',values:['Chrome']}],'Chart navigation retains a selection still being applied');
  await context.unroute('**/sites/1/filter?**');

  const noScript=await browser.newContext({javaScriptEnabled:false,storageState:await context.storageState()});
  try {
    const p=await noScript.newPage();
    await p.goto(`${origin}/sites/1?filter=browser`);
    await p.locator('.filter-values input[type=checkbox][value="Chrome"]').check();
    await p.locator('.filter-values input[type=checkbox][value="Safari"]').check();
    await p.getByRole('button',{name:'Apply',exact:true}).click();
    const filters=JSON.parse(new URL(p.url()).searchParams.get('filters'));
    assert.deepEqual(filters,[{key:'browser',op:'is',values:['Chrome','Safari']}]);
    await p.getByLabel('Search filter values',{exact:true}).fill('Firefox');
    await p.getByRole('button',{name:'Search',exact:true}).click();
    assert.equal(await p.locator('.filter-values input:checked').count(),2);
    assert.equal(await p.locator('.filter-values input[type=checkbox][value="Firefox"]').count(),1);
    await p.getByRole('link',{name:'Done',exact:true}).click();
    assert.equal(await p.locator('.filter-picker').count(),0);
    assert.equal(await p.locator('.filter').count(),1);
    await p.getByRole('link',{name:'Remove browser filter',exact:true}).click();
    assert.equal(await p.locator('.filter').count(),0);
  } finally {await noScript.close();}
  await page.setViewportSize({width:1440,height:1050});await page.emulateMedia({colorScheme:'light'});
  await page.goto(`${origin}/sites/1`);
}
