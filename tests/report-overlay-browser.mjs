import assert from 'node:assert/strict';
import {join} from 'node:path';

export async function checkReportOverlay({context,browser,origin,output}) {
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  for (const [colorScheme,width,javaScriptEnabled] of [['light',1280,true],['dark',768,true],['light',390,true],['dark',320,true],['light',1280,false]]) {
    const owner=await browser.newContext({storageState,colorScheme,viewport:{width,height:900},javaScriptEnabled});
    try {
      const page=await owner.newPage(),errors=[];
      page.on('pageerror',e=>errors.push(e.message));
      const base=`${origin}/sites/1?period=28d&compare=previous&compare_match=date&browser=Chrome`;
      await page.goto(base);
      if(width<768) {
        const cards=await page.locator('.report-grid>.report-panel').evaluateAll(es=>es.map(e=>({name:e.className,y:e.getBoundingClientRect().y})));
        assert.match(cards[0].name,/panel-source/);
        assert.ok(cards.every((card,index)=>index===0||card.y>cards[index-1].y),'Mobile reports retain source, page, location and device order');
        const tabsFit=await page.locator('.report-tabs').evaluateAll(es=>es.every(e=>e.scrollWidth<=e.clientWidth+1));
        assert.ok(tabsFit,'Filtered browser tabs fit narrow screens');
      }
      await page.locator('.panel-page .expand-report').click();
      await page.waitForURL(url=>url.searchParams.get('detail')==='page');
      await page.waitForLoadState('domcontentloaded');
      const modal=page.locator('.expanded-report'), search=page.getByRole('searchbox',{name:'Search report'});
      assert.equal(await page.locator('.report-overlay').count(),1);
      assert.equal(await page.locator('.report-grid>.report-panel').count(),4,'Keep all four cards behind the report');
      const bounds=await modal.boundingBox();
      assert.ok(bounds.x>=15 && bounds.y>=15 && bounds.x+bounds.width<=width-15 && bounds.y+bounds.height<=885);
      assert.ok(await page.locator('.report-table-scroll').evaluate(e=>e.scrollHeight>e.clientHeight),'Large tables scroll inside the overlay');
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
      assert.equal(await page.locator('body').evaluate(e=>getComputedStyle(e).overflow),'hidden');
      assert.equal(await page.locator('.report-search button').isVisible(),!javaScriptEnabled);
      await page.screenshot({path:join(output,`report-overlay-${colorScheme}-${width}-${javaScriptEnabled}.png`)});
      await search.fill('/article/109');
      if (!javaScriptEnabled) await search.press('Enter');
      await page.waitForURL(url=>url.searchParams.get('search')==='/article/109');
      await page.waitForFunction(()=>!document.querySelector('main[aria-busy=true]'));
      assert.equal(await modal.locator('tbody tr').count(),1);
      assert.match(await modal.locator('tbody tr').innerText(),/article\/109/);
      assert.equal(new URL(page.url()).searchParams.get('browser'),'Chrome');
      if (javaScriptEnabled) {
        assert.equal(await search.evaluate(e=>e===document.activeElement),true,'Live search preserves input focus');
        await page.keyboard.press('Escape');
      } else await page.getByRole('link',{name:'Close expanded report',exact:true}).click();
      await page.waitForURL(url=>!url.searchParams.has('detail'));
      for (const key of ['search','sort','order','offset']) assert.equal(new URL(page.url()).searchParams.has(key),false,key);
      assert.equal(new URL(page.url()).searchParams.get('browser'),'Chrome');
      assert.equal(await page.locator('.report-overlay').count(),0);
      if(javaScriptEnabled) {
        await page.locator('.panel-page .expand-report').click();
        let release,arrive;
        const held=new Promise(r=>release=r),started=new Promise(r=>arrive=r);
        await owner.route('**/sites/1?**',async route=>{
          if(new URL(route.request().url()).searchParams.get('search')==='article') {arrive();await held;}
          await route.continue();
        });
        await search.fill('article');await started;
        await page.getByRole('link',{name:'Close expanded report',exact:true}).click();
        release();
        await page.waitForURL(url=>!url.searchParams.has('detail'));
        await owner.unroute('**/sites/1?**');
        assert.equal(new URL(page.url()).searchParams.has('search'),false,'An in-flight search is discarded on close');
        assert.equal(new URL(page.url()).searchParams.get('compare'),'previous');
        assert.deepEqual(JSON.parse(new URL(page.url()).searchParams.get('filters')),[{key:'browser',op:'is',values:['Chrome']}]);
        await page.locator('.panel-page .expand-report').click();
        await page.locator('.report-backdrop').click({position:{x:2,y:2}});
        await page.waitForURL(url=>!url.searchParams.has('detail'));
      }
      assert.deepEqual(errors,[]);
    } finally { await owner.close(); }
  }
}
