import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createServer} from 'node:http';
import {join} from 'node:path';

// Real navigation verifies keepalive delivery; the owner session is supplied by
// the parity runner. All clicks and report fixtures live in its disposable DB.
export async function checkOutbound({browser,context: owner,origin,database,output}) {
  execFileSync('python3',['-c',"import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.execute(\"INSERT INTO sites VALUES(102,'localhost','UTC',0)\");c.commit()",database]);
  const context=await browser.newContext({userAgent:'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36'});
  await context.addInitScript(()=>Object.defineProperty(navigator,'webdriver',{get:()=>false}));
  const website=createServer((request,response)=>{
    response.writeHead(200,{'Content-Type':'text/html'});
    const port=website.address().port;
    const destination=`http://127.0.0.1:${port}/destination?secret=omitted#private`;
    response.end(request.url==='/' ? `<!doctype html><title>Outbound fixture</title>
      <a id="same" href="/internal">Internal</a>
      <a id="external" href="${destination}"><span>Leave</span></a>
      <a id="middle" href="${destination}" target="_blank">Middle</a>
      <a id="keyboard" href="${destination}" target="_blank">Keyboard</a>
      <div id="shadow"></div>
      <script>document.querySelector('#shadow').attachShadow({mode:'open'}).innerHTML='<a href="${destination}" target="_blank">Shadow link</a>';</script>
      <script defer src="${origin}/script.js" data-domain="localhost" data-local></script>` : '<!doctype html><title>Destination</title>Arrived');
  });
  await new Promise((resolve,reject)=>{website.once('error',reject);website.listen(0,resolve);});
  const errors=[];
  const stored=()=>JSON.parse(execFileSync('python3',['-c',`import sqlite3,sys,json
c=sqlite3.connect(sys.argv[1]);print(json.dumps(c.execute("SELECT o.url,e.path,COUNT(*),COUNT(DISTINCT e.visitor) FROM outbound_clicks o JOIN events e ON e.id=o.event_id WHERE o.site_id=102 GROUP BY o.url,e.path").fetchall()))`,database],{encoding:'utf8'}));
  async function count(expected) {
    const deadline=Date.now()+5000;
    while(Date.now()<deadline) { const rows=stored();if(rows.reduce((n,row)=>n+row[2],0)===expected)return rows;await new Promise(resolve=>setTimeout(resolve,100)); }
    assert.fail(`Expected ${expected} clicks: ${JSON.stringify(stored())}`);
  }
  try {
    const page=await context.newPage();page.setDefaultTimeout(15000);page.setDefaultNavigationTimeout(15000);page.on('pageerror',e=>errors.push(e.message));
    const [view]=await Promise.all([page.waitForResponse(r=>r.url()===origin+'/api/event'),page.goto(`http://localhost:${website.address().port}/`)]);assert.equal(view.status(),204);
    const clicked=page.waitForResponse(r=>r.url()===origin+'/api/outbound');
    await page.locator('#middle').click({button:'middle'});assert.equal((await clicked).status(),204);
    await count(1);
    await page.bringToFront();await page.locator('#keyboard').focus();await page.keyboard.press('Enter');await count(2);
    await page.bringToFront();await page.getByRole('link',{name:'Shadow link',exact:true}).click();await count(3);
    // Clicking a nested span leaves the page immediately. The click must still arrive.
    await page.bringToFront();await page.locator('#external span').click();await page.waitForURL('**/destination?secret=omitted#private');
    const rows=await count(4);
    assert.deepEqual(rows,[[`http://127.0.0.1:${website.address().port}/destination`,'/',4,1]]);
    assert.deepEqual(errors,[]);
    const dashboard=await owner.newPage();dashboard.setDefaultTimeout(15000);dashboard.setDefaultNavigationTimeout(15000);dashboard.on('pageerror',e=>errors.push(e.message));
    await dashboard.goto(`${origin}/sites/102?period=day&compare=off`);
    const panel=dashboard.locator('.panel-outbound');
    assert.match(await panel.innerText(),/Goals/i);
    assert.deepEqual(await panel.locator('thead th').allTextContents(),['Goal','Uniques','Total','CR']);
    assert.deepEqual(await panel.locator('tbody td').allTextContents(),['1','4','100.0%']);
    await panel.getByRole('link',{name:'Expand Goal conversions report',exact:true}).click();
    await dashboard.waitForURL('**&detail=goals');
    await dashboard.locator('.expanded-report').getByRole('link',{name:'Outbound Link: Click',exact:true}).click();
    await dashboard.waitForURL(url=>url.searchParams.get('goal')==='Outbound Link: Click'&&!url.searchParams.has('detail'));
    assert.equal(await dashboard.locator('.metric').filter({hasText:'Unique conversions'}).getAttribute('data-value'),'1');
    assert.equal(await dashboard.locator('.metric').filter({hasText:'Total conversions'}).getAttribute('data-value'),'4');
    assert.equal(await dashboard.locator('.metric').filter({hasText:'Conversion rate'}).getAttribute('data-value'),'100');
    await panel.getByRole('link',{name:'Expand url report',exact:true}).click();
    await dashboard.waitForURL('**&detail=outbound');
    const expanded=dashboard.locator('.expanded-report.panel-outbound');
    await expanded.getByLabel('Search report').fill('missing');
    await dashboard.waitForURL('**&search=missing');assert.match(await expanded.innerText(),/No matching results/);
    await expanded.getByLabel('Search report').fill('127.0.0.1');
    await dashboard.waitForURL('**&search=127.0.0.1');assert.equal(await expanded.locator('tbody tr').count(),1);
    await expanded.getByRole('link',{name:'Total',exact:false}).click();
    await dashboard.waitForURL(url=>url.searchParams.get('sort')==='clicks'&&url.searchParams.get('order')==='desc');
    assert.equal(await expanded.locator('thead th').nth(2).getAttribute('aria-sort'),'descending');
    await expanded.getByRole('link',{name:'Total',exact:false}).click();
    await dashboard.waitForURL(url=>url.searchParams.get('sort')==='clicks'&&url.searchParams.get('order')==='asc');
    assert.equal(await expanded.locator('thead th').nth(2).getAttribute('aria-sort'),'ascending');
    for (const [width,theme] of [[1440,'light'],[390,'light'],[390,'dark'],[320,'light'],[320,'dark']]) {
      await dashboard.setViewportSize({width,height:950});await dashboard.emulateMedia({colorScheme:theme});
      await dashboard.screenshot({path:join(output,`outbound-${width}-${theme}.png`),fullPage:true});
      assert.equal(await dashboard.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),true,`Expanded outbound report fits ${width}px`);
      assert.equal(await expanded.locator('tbody a').first().isVisible(),true);
    }
    await dashboard.keyboard.press('Escape');await dashboard.waitForURL(url=>!url.searchParams.has('detail'));
    for(const width of [390,320]) {
      await dashboard.setViewportSize({width,height:950});
      assert.equal(await dashboard.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),true,`Goal dashboard fits ${width}px`);
      const labels=await dashboard.locator('.metrics-goals .metric>span').evaluateAll(es=>es.map(e=>{
        const range=document.createRange();range.selectNodeContents(e);
        const text=range.getBoundingClientRect(),metric=e.parentElement.getBoundingClientRect();
        return {left:text.left,right:text.right,start:metric.left,end:metric.right};
      }));
      assert.ok(labels.every(b=>b.left>=b.start-1&&b.right<=b.end+1),`Goal labels fit their cards at ${width}px`);
      await dashboard.screenshot({path:join(output,`outbound-dashboard-${width}.png`),fullPage:true});
    }
    await dashboard.setViewportSize({width:1440,height:1000});await dashboard.emulateMedia({colorScheme:'light'});
    await dashboard.screenshot({path:join(output,'outbound-dashboard.png'),fullPage:true});
    await panel.locator('tbody th a').first().click();
    await dashboard.waitForURL(url=>url.searchParams.has('url')&&!url.searchParams.has('detail'));
    assert.equal(await dashboard.getByRole('link',{name:'Remove url filter',exact:true}).isVisible(),true);
    assert.equal(await dashboard.locator('.metric').filter({hasText:'Total conversions'}).getAttribute('data-value'),'4');
    await dashboard.getByRole('link',{name:'Remove url filter',exact:true}).click();
    await dashboard.waitForURL(url=>!url.searchParams.has('url')&&url.searchParams.has('goal'));
    await dashboard.getByRole('link',{name:'Remove goal filter',exact:true}).click();
    await dashboard.waitForURL(url=>!url.searchParams.has('goal'));
    assert.equal(await dashboard.locator('.metric').filter({hasText:'Total pageviews'}).getAttribute('data-value'),'1');
    assert.equal(await panel.getByRole('link',{name:'Outbound Link: Click',exact:true}).isVisible(),true);
    assert.deepEqual(errors,[]);
    await dashboard.close();
  } finally {await context.close();await new Promise(resolve=>website.close(resolve));}
}
