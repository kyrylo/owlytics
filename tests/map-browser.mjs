import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

export async function checkMaps({context,browser,origin,database,output}) {
  execFileSync('python3',['-c',`
import sqlite3,sys
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
c.execute("INSERT INTO sites VALUES(140,'map.example','UTC',0),(141,'map-legacy.example','UTC',0)")
for site in [140,141]:
 c.execute("INSERT INTO import_batches VALUES(?,?,'2025-02-01','2025-02-01',0)",(site,site))
 c.execute("INSERT INTO import_daily VALUES(?,'2025-02-01',?,2606,5212)",(site,site))
for code,count in [('US',2000),('PH',500),('DE',100),('SG',1),('JP',2),('AQ',3)]:
 c.execute("INSERT INTO import_breakdowns VALUES(140,'2025-02-01','country',?,140,?)",(code,count*2))
 c.execute("INSERT INTO import_metrics VALUES(140,'2025-02-01','country',?,140,?,?,0,0,NULL)",(code,count,count))
c.execute("INSERT INTO import_breakdowns VALUES(141,'2025-02-01','country','US',141,3)")
c.commit()
`,database]);
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  for(const [width,scheme,touch,javaScriptEnabled] of [[1440,'light',false,true],[1440,'dark',false,true],[768,'light',false,true],[320,'light',false,true],[390,'dark',true,true],[390,'light',false,false],[1440,'dark',false,false]]) {
    const owner=await browser.newContext({storageState,colorScheme:scheme,viewport:{width,height:1000},hasTouch:touch,isMobile:touch,javaScriptEnabled});
    try {
      const page=await owner.newPage(),errors=[],external=[];
      page.on('pageerror',e=>errors.push(e.message));
      page.on('console',message=>{if(message.type()==='error'&&/Content Security Policy|violates/.test(message.text()))errors.push(message.text());});
      page.on('request',request=>{if(!request.url().startsWith(origin))external.push(request.url());});
      const base='/sites/140?from=2025-02-01&to=2025-02-01&compare=off&metric=pageviews&locations_tab=map';
      const visit=async query=>{
        await page.goto(origin+query);
        if(javaScriptEnabled)await page.waitForFunction(()=>document.documentElement.hasAttribute('data-dashboard-enhanced'));
        await page.locator('.report-panel:has(.country-map)').scrollIntoViewIfNeeded();
        await page.waitForFunction(()=>document.querySelector('.map-country').getBBox().width>0);
      };
      await visit(base);
      assert.equal(await page.locator('.map-canvas>svg').getAttribute('viewBox'),'0 0 475 335');
      assert.equal(await page.locator('.map-legend').count(),0);
      assert.equal(await page.locator('.country-map a[data-country]').count(),5);
      assert.match(await page.locator('.country-map a[data-country=SG]').getAttribute('aria-label'),/Singapore · 1 visitor$/);
      const geometry=await page.locator('.map-country').evaluateAll(es=>es.map(e=>{const b=e.getBBox();return [b.x,b.y,b.width,b.height];}));
      assert.equal(geometry.length,236);
      assert.ok(geometry.every(([x,y,w,h])=>w>0&&h>0&&x>=0&&y>=0&&x+w<=475&&y+h<=335),'Every country, including small states, stays inside the projected frame');
      assert.equal(await page.locator('.map-note').innerText(),'Not on map: 3 visitors');
      const colors=await page.locator('.country-map a[data-country] .map-country').evaluateAll(es=>es.map(e=>getComputedStyle(e).fill));
      assert.equal(new Set(colors).size,5,'Linear shading distinguishes all five known counts');
      assert.ok(colors.every(c=>c!=='none'&&c!=='rgb(0, 0, 0)'),'Presentation attributes retain the theme palette without inline styles');
      const point=async(lon,lat)=>{
        const b=await page.locator('.map-canvas>svg').boundingBox();
        return {x:b.x+(237.5+75*lon*Math.PI/180)*b.width/475,y:b.y+(335/1.5-75*Math.log(Math.tan(Math.PI/4+lat*Math.PI/360)))*b.height/335};
      };
      const us=await point(-100,38);
      if(javaScriptEnabled&&!touch) {
        await page.mouse.move(us.x,us.y);
        const tip=page.locator('.map-tooltip');
        assert.equal(await tip.locator('strong').innerText(),'United States');
        assert.equal(await tip.locator('b').innerText(),'2k');
        assert.equal(await tip.locator('span').innerText(),'visitors');
        assert.equal(await page.locator('.map-outline').getAttribute('hidden'),null);
        assert.equal(await page.locator('.country-map a[data-country]>title').count(),0,'Native hover labels are removed only after enhancement');
        const jp=await point(140,36);await page.mouse.move(jp.x,jp.y);
        assert.equal(await tip.locator('strong').innerText(),'Japan');
        assert.ok(await tip.evaluate(t=>{const r=t.getBoundingClientRect(),b=t.parentElement.getBoundingClientRect();return r.left>=b.left&&r.right<=b.right+1&&r.top>=b.top&&r.bottom<=b.bottom+1;}),'Country tooltip stays inside a narrow map');
        const empty=await point(2,46);await page.mouse.move(empty.x,empty.y);
        assert.equal(await tip.isVisible(),false,'A country with no data has no invented count');
        assert.equal(await page.locator('.map-outline').getAttribute('hidden'),null,'Empty outlines can still be inspected');
        const before=page.url();await page.mouse.click(empty.x,empty.y);assert.equal(page.url(),before,'Empty countries do not invent a filter');
        await page.mouse.move(us.x,us.y);
      } else if(!javaScriptEnabled) {
        assert.match(await page.locator('.country-map a[data-country=US]>title').textContent(),/United States · 2,000 visitors/);
      }
      await page.locator('.report-panel:has(.country-map)').screenshot({path:join(output,`map-${scheme}-${width}-${javaScriptEnabled}.png`)});
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
      const target=await point(-100,38);
      assert.equal(await page.evaluate(({x,y})=>document.elementFromPoint(x,y)?.closest('a')?.dataset.country,target),'US','Drilldown targets the visible United States geometry');
      if(touch)await page.touchscreen.tap(target.x,target.y);else await page.mouse.click(target.x,target.y);
      await page.waitForURL(url=>url.searchParams.get('country')==='US');
      await page.waitForLoadState('domcontentloaded');
      for(const [key,value] of [['locations_tab','region'],['from','2025-02-01'],['to','2025-02-01'],['metric','pageviews'],['compare','off']])assert.equal(new URL(page.url()).searchParams.get(key),value);
      assert.equal(await page.locator('.panel-region').count(),1);
      await visit(base.replace('/140?','/141?'));
      assert.match(await page.locator('.country-map a[data-country=US]').getAttribute('aria-label'),/3 pageviews/,'Unavailable imported visitors retain the labeled pageview fallback');
      if(javaScriptEnabled&&!touch) {
        const p=await point(-100,38);await page.mouse.move(p.x,p.y);
        assert.equal(await page.locator('.map-tooltip span').innerText(),'pageviews');
      }
      assert.deepEqual(errors,[]);assert.deepEqual(external,[]);
    } finally {await owner.close();}
  }
}
