import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

export async function checkSourceIcons({context,browser,origin,database,output}) {
  const sources=['Google','Bing','DuckDuckGo','Yahoo!','Brave','Ecosia','Yandex','Baidu','Qwant','Startpage','Sogou','ChatGPT','Claude','Perplexity','Gemini','Copilot','Grok','DeepSeek','Facebook','Twitter','Instagram','LinkedIn','Reddit','GitHub','Hacker News','Product Hunt','Bluesky','Threads','Pinterest','Telegram','Discord','Slack','WhatsApp','Microsoft Teams','YouTube','TikTok','Vimeo','Twitch','Amazon','eBay','Etsy','Gmail','','independent.example','猫の日記','<svg onload=alert(1)>'];
  execFileSync('python3',['-c',`
import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
c.execute("INSERT INTO sites VALUES(12,'owl.example','UTC',0),(13,'github.com','UTC',0)")
browsers=['Chrome','Safari','Firefox','Edge','Opera','Samsung Internet','Brave','Other']
systems=['macOS','iOS','Windows','Android','Linux','Ubuntu','Chrome OS','']
for i,source in enumerate(json.loads(sys.argv[2])):
 c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,referrer,country,browser,device,os) VALUES(12,1759708800+?,'2025-10-06',randomblob(16),randomblob(16),'/',?,?,'US',?,?,?)",(i,source,'github.com/article' if source=='GitHub' else '',browsers[i%8],['Desktop','Mobile','Tablet'][i%3],systems[i%8]))
c.commit()
`,database,JSON.stringify(sources)]);
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  const owner=await browser.newContext({storageState,viewport:{width:1440,height:1050}});
  const errors=[],external=[],art=[];
  try {
    const page=await owner.newPage();
    page.on('pageerror',e=>errors.push(e.message));
    page.on('console',message=>{if(message.type()==='error'&&/Content Security Policy|violates/.test(message.text()))errors.push(message.text());});
    page.on('request',request=>{if(!request.url().startsWith(origin+'/'))external.push(request.url());});
    const base=origin+'/sites/12?from=2025-10-06&to=2025-10-06&compare=off&sources_tab=source';
    await page.goto(origin+'/');
    const card=page.locator('.site-card').filter({has:page.getByRole('heading',{name:'owl.example',exact:true})});
    const identity=await card.locator('.report-icon').innerHTML();
    assert.equal(await card.locator('.site-monogram text').textContent(),'O');
    assert.equal(await page.locator('.site-card').filter({has:page.getByRole('heading',{name:'github.com',exact:true})}).locator('.brand-github').count(),1);
    await page.screenshot({path:join(output,'site-icons-cards.png'),fullPage:true});
    for(const [width,scheme] of [[1440,'light'],[390,'dark'],[320,'light']]) {
      await page.setViewportSize({width,height:1050});await page.emulateMedia({colorScheme:scheme});
      await page.goto(base);
      assert.equal(await page.locator('.site-picker>summary .report-icon').innerHTML(),identity);
      await page.locator('.site-picker>summary').click();
      assert.equal(await page.locator('.site-option[href="/sites/12"] .report-icon').innerHTML(),identity);
      assert.equal(await page.locator('.site-option[href="/sites/13"] .brand-github').count(),1);
      assert.equal(await page.locator('.site-option[href="/sites/12"]').getAttribute('href'),'/sites/12');
      const bounds=await page.locator('.site-picker>.dropdown-content').boundingBox();
      assert.ok(bounds.x>=0&&bounds.x+bounds.width<=width+1);
      await page.screenshot({path:join(output,`site-icons-menu-${scheme}-${width}.png`),fullPage:true});
      await page.keyboard.press('Escape');
      await page.locator('.panel-source .expand-report').click();
      await page.waitForURL(url=>url.searchParams.get('detail')==='source');
      await page.waitForLoadState('load');
      const rows=page.locator('.expanded-report tbody tr');
      assert.equal(await rows.count(),sources.length,`Source rows at ${width}px: ${page.url()}`);
      assert.equal(await rows.locator('.site-monogram').count(),3);
      assert.equal(await rows.locator('.brand-direct').count(),1);
      assert.equal(await rows.locator('.report-icon>svg,.report-icon>img').count(),sources.length);
      assert.equal(await page.locator('.report-icon script').count(),0);
      await page.waitForFunction(()=>[...document.querySelectorAll('.report-icon>img')].every(image=>image.complete&&image.naturalWidth>0));
      for(const name of ['google','microsoftbing','duckduckgo','copilot','googlegemini','instagram','slack','gmail','tiktok']) {
        const image=rows.locator(`.brand-${name}>img`);
        assert.equal(await image.count(),1,`${name} uses the original multicolor artwork`);
        assert.match(await image.getAttribute('src'),new RegExp(`^/assets/brands/${name}\\.[a-f0-9]{16}\\.(svg|ico)$`));
      }
      const discord=rows.locator('.brand-discord>img');
      assert.ok(await discord.evaluate(image=>image.naturalWidth/image.naturalHeight<1.5),'Use Discord’s symbol, not its tiny horizontal wordmark');
      await page.screenshot({path:join(output,`source-icons-${scheme}-${width}.png`),fullPage:true});
      const sizes=await rows.locator('.report-icon').evaluateAll(es=>es.map(e=>({w:e.getBoundingClientRect().width,h:e.getBoundingClientRect().height})));
      assert.ok(sizes.every(({w,h})=>w===16&&h===16));
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
    }
    art.push(...await page.locator('.expanded-report tbody tr').evaluateAll(rows=>rows.map(row=>({label:[...row.querySelector('th').querySelector('a').childNodes].filter(node=>node.nodeType===3||!node.classList?.contains('report-icon')).map(node=>node.textContent).join('').trim(),html:row.querySelector('.report-icon').outerHTML}))));
    // Icons do not change the label or the link's audience; URL paths use the host icon.
    await page.setViewportSize({width:1440,height:1050});
    await page.locator('.expanded-report').getByRole('link',{name:'GitHub',exact:true}).click();
    assert.equal(await page.locator('.panel-referrer .brand-github').count(),1);
    assert.match(await page.locator('.panel-referrer').innerText(),/github.com\/article/);
    assert.equal(await page.locator('.metrics .metric').first().locator('strong').textContent(),'1');
    await page.goto(base);
    for(const [tab,panel,brands] of [['Browsers','browser',['googlechrome','safari','firefox','microsoftedge','opera','samsunginternet','brave','generic']],['OS','os',['apple','windows','android','linux','ubuntu','googlechrome','device']],['Devices','device',['device']]]) {
      await page.getByRole('link',{name:tab,exact:true}).click();
      for(const brand of brands) assert.ok(await page.locator(`.panel-${panel} .brand-${brand}`).count(),`${panel}: ${brand}`);
      const icons=page.locator(`.panel-${panel} .report-icon`);
      await page.waitForFunction(()=>[...document.querySelectorAll('.report-icon>img')].every(image=>image.complete&&image.naturalWidth>0));
      await page.locator(`.panel-${panel} .data-row`).first().hover();
      assert.ok((await icons.locator('svg,img').evaluateAll(elements=>elements.map(element=>getComputedStyle(element).opacity))).every(opacity=>opacity==='1'),'Row hover must not fade brand artwork');
      await page.locator(`.panel-${panel}`).screenshot({path:join(output,`software-icons-${panel}.png`)});
      if(panel!=='device') art.push(...await page.locator(`.panel-${panel} .data-row`).evaluateAll(rows=>rows.map(row=>({label:[...row.querySelector('.row-label').childNodes].filter(node=>node.nodeType===3||!node.classList?.contains('report-icon')).map(node=>node.textContent).join('').trim(),html:row.querySelector('.report-icon').outerHTML}))));
    }
    // Render the actual server-generated marks together for visual review at 16px and 32px.
    const escaped=text=>text.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;');
    await page.route(origin+'/__brand-review',route=>route.fulfill({contentType:'text/html',body:`<!doctype html><html><head><meta charset="utf-8"><link rel="stylesheet" href="/assets/app.css"><style>body{padding:24px}main{display:grid;grid-template-columns:repeat(5,1fr);gap:8px}article{display:flex;align-items:center;gap:12px;padding:12px;background:var(--panel);border-radius:6px;font-size:12px}.large>.report-icon,.large>.report-icon>svg,.large>.report-icon>img{width:32px;height:32px}.label{min-width:0;overflow-wrap:anywhere}</style></head><body><main>${art.map(item=>`<article><span class="large">${item.html}</span>${item.html}<span class="label">${escaped(item.label)}</span></article>`).join('')}</main></body></html>`}));
    await page.setViewportSize({width:1200,height:1000});
    await page.goto(origin+'/__brand-review');
    await page.waitForFunction(()=>[...document.images].every(image=>image.complete&&image.naturalWidth>0));
    for(const scheme of ['light','dark']) {
      await page.emulateMedia({colorScheme:scheme});
      await page.screenshot({path:join(output,`brand-art-${scheme}.png`),fullPage:true});
    }
    assert.deepEqual(errors,[]);assert.deepEqual(external,[]);
  } finally {await owner.close();}
  const noScript=await browser.newContext({storageState,javaScriptEnabled:false});
  try {
    const page=await noScript.newPage();
    await page.goto(origin+'/sites/12?from=2025-10-06&to=2025-10-06&compare=off');
    await page.locator('.site-picker>summary').click();
    const link=page.getByRole('link',{name:'github.com',exact:true});
    assert.equal(await link.locator('.brand-github').count(),1);
    await link.click();await page.waitForURL('**/sites/13');
    assert.equal(await page.locator('.site-picker>summary .brand-github').count(),1);
  } finally {await noScript.close();}
}
