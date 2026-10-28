import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdtempSync,writeFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';

export async function checkImports({page,context,browser,origin,database,binary,output}) {
  const folder=mkdtempSync(join(tmpdir(),'owlytics-import-browser-'));
  try {
    execFileSync('python3',['-c',`
import sqlite3,sys
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
c.execute("INSERT INTO sites VALUES(5,'import.example','UTC',0)")
for i,path in enumerate(['/','/docs']):
 c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,referrer,country,region,city,browser,browser_version,os,os_version,device) VALUES(5,unixepoch('2025-10-03')+?1,'2025-10-03',zeroblob(16),?2,?3,'Google','google.com/x','US','US: US-CA','US: 5391959','Chrome','1','macOS','14','Desktop')",(i*30,bytes([i])*16,path))
c.commit()
`,database]);
    const files={
      visitors:'date,visitors,pageviews,visits,bounces,visit_duration\n2025-10-01,5,10,5,2,50\n2025-10-02,5,10,5,1,70\n',
      sources:'date,source,referrer,utm_source,utm_medium,utm_campaign,visitors,pageviews,visits,bounces,visit_duration\n2025-10-01,Google,google.com/x,google,cpc,Launch,4,8,4,1,40\n2025-10-01,Facebook,facebook.com,fb,social,Launch,1,2,1,1,10\n2025-10-02,Google,google.com/x,google,cpc,Autumn,3,6,3,0,50\n2025-10-02,Facebook,facebook.com,fb,social,Autumn,2,4,2,1,20\n',
      locations:'date,country,region,city,visitors,pageviews,visits,bounces,visit_duration\n2025-10-01,US,US-CA,5391959,3,6,3,1,30\n2025-10-01,US,US-NY,5128581,1,2,1,0,10\n2025-10-01,DE,DE-BE,2950159,1,2,1,1,10\n2025-10-02,US,US-CA,5391959,2,4,2,0,40\n2025-10-02,DE,DE-BE,2950159,3,6,3,1,30\n',
      browsers:'date,browser,browser_version,visitors,pageviews,visits,bounces,visit_duration\n2025-10-01,Chrome,1,4,8,4,1,40\n2025-10-01,Firefox,1,1,2,1,1,10\n2025-10-02,Chrome,2,3,6,3,0,50\n2025-10-02,Firefox,1,2,4,2,1,20\n',
      pages:'date,hostname,page,visitors,pageviews,visits,total_time_on_page,total_time_on_page_visits,total_scroll_depth,total_scroll_depth_visits\n2025-10-01,import.example,/,5,10,5,100,5,250,5\n2025-10-02,import.example,/,5,10,5,100,5,250,5\n',
      entry_pages:'date,entry_page,visitors,pageviews,entrances,bounces,visit_duration\n2025-10-01,/,5,10,5,2,50\n2025-10-02,/,5,10,5,1,70\n'
    };
    for (const [table,csv] of Object.entries(files)) writeFileSync(join(folder,`imported_${table}_20251001_20251002.csv`),csv);
    execFileSync(binary,['import-plausible','import.example',folder],{env:{...process.env,OWLYTICS_DATABASE:database},stdio:'pipe'});
    const base=`${origin}/sites/5?from=2025-10-01&to=2025-10-03&compare=off`;
    const idle=()=>page.waitForFunction(()=>!document.querySelector('main[aria-busy=true]'));
    const visitors=async()=>Number(await page.locator('.metrics .metric').first().getAttribute('data-value'));
    for (const [region,city] of [['US: California','US: San Francisco, California'],['US: US-CA','US: 5391959']]) {
      await page.goto(`${base}&locations_tab=city&region=${encodeURIComponent(region)}&city=${encodeURIComponent(city)}`);
      assert.equal(await visitors(),6,'Legacy and canonical location links include native and imported visitors');
      assert.equal(await page.locator('.panel-city .data-row').count(),1);
      assert.match(await page.locator('.panel-city .row-label').innerText(),/San Francisco/);
      assert.equal(await page.locator('.panel-city .data-row strong').innerText(),'6');
      assert.equal(await page.locator('.metrics .metric').filter({hasText:'Total pageviews'}).getAttribute('data-value'),'12');
    }
    await page.goto(`${base}&country=US&filter=region`);
    assert.equal(await page.locator('.filter-values input[value="US: US-CA"]').count(),1);
    assert.equal(await page.locator('.filter-values input[value="US: California"]').count(),0);
    await page.goto(base);
    assert.equal(await visitors(),11);
    // The native GET switch must keep filters, metrics and form state without
    // storing the choice or altering imported data.
    await page.locator('.dashboard-menu>summary').click();
    let toggle=page.getByRole('switch',{name:'Include imported data'});
    assert.equal(await toggle.getAttribute('aria-checked'),'true');
    await toggle.focus();await page.keyboard.press('Space');
    await page.waitForURL(url=>url.searchParams.get('with_imported')==='false');
    assert.equal(await visitors(),1);
    assert.equal(await page.locator('.import-report-note,.import-note').count(),0);
    await page.getByRole('link',{name:'Show pageviews',exact:true}).click();
    assert.equal(new URL(page.url()).searchParams.get('with_imported'),'false');
    await page.locator('.dashboard-menu>summary').click();
    const hiddenExport=await page.getByRole('link',{name:'Export stats',exact:true}).getAttribute('href');
    const hiddenResponse=await context.request.get(new URL(hiddenExport,origin).href);
    const hidden=JSON.parse(execFileSync('python3',['-c',`
import sys,zipfile,io,csv,json
z=zipfile.ZipFile(io.BytesIO(sys.stdin.buffer.read()))
print(json.dumps({'summary':list(csv.DictReader(io.StringIO(z.read('summary.csv').decode())))[0],'note':z.read('readme.txt').decode()}))
`],{input:await hiddenResponse.body(),encoding:'utf8'}));
    assert.equal(hidden.summary.visitors,'1');assert.equal(hidden.summary.pageviews,'2');
    assert.match(hidden.note,/Imported history switched off: true/);
    for(const colorScheme of ['light','dark']) {
      await page.setViewportSize({width:320,height:844});await page.emulateMedia({colorScheme});
      assert.ok(await page.locator('.dashboard-menu>.dropdown-content').evaluate(element=>{const b=element.getBoundingClientRect();return b.left>=0&&b.right<=innerWidth+1;}));
      await page.screenshot({path:join(output,`import-switch-${colorScheme}-320.png`)});
    }
    await page.setViewportSize({width:1440,height:1050});await page.emulateMedia({colorScheme:'light'});
    await toggle.click();
    assert.equal(await visitors(),11);
    // Submit while an audience change is in flight: the switch must not lose it.
    await page.goto(`${base}&with_imported=false&filter=browser`);
    assert.equal(await page.locator('.filter-values input[value=Firefox]').count(),0,'Suggestions respect the imported-data switch');
    assert.equal(await page.locator('.filter-values input[value=Chrome]').count(),1);
    await page.goto(`${base}&filter=browser`);
    let release;
    const held=new Promise(resolve=>release=resolve);
    await context.route('**/sites/5/filter?**',async route=>{await held;await route.continue();});
    await page.evaluate(()=>{
      const checkbox=document.querySelector('.filter-values input[value="Chrome"]');
      checkbox.checked=true;checkbox.dispatchEvent(new Event('change',{bubbles:true}));
      document.querySelector('.import-switch button').click();
    });
    release();
    await page.waitForURL(url=>url.searchParams.get('with_imported')==='false');
    assert.deepEqual(JSON.parse(new URL(page.url()).searchParams.get('filters')),[{key:'browser',op:'is',values:['Chrome']}]);
    assert.equal(await visitors(),1);
    assert.equal(await page.locator('.filter-picker').count(),0);
    await context.unroute('**/sites/5/filter?**');
    await page.locator('.dashboard-menu>summary').click();await toggle.click();
    assert.equal(await visitors(),8,'Toggling on restores compatible filtered history');
    await page.goto(base);
    await page.locator('.panel-source .data-row').filter({hasText:'Google'}).click();
    assert.equal(await visitors(),8);
    assert.equal(await page.locator('.panel-referrer .import-report-note').count(),0);
    assert.equal(await page.locator('.panel-page .import-report-note').count(),1);
    assert.equal(await page.locator('.panel-page .data-row').count(),2);
    assert.equal(await page.locator('.panel-referrer .data-row strong').innerText(),'8');
    assert.equal(await page.locator('.overview .metric').filter({hasText:'Total pageviews'}).getAttribute('data-value'),'16');
    await page.locator('.dashboard-menu>summary').click();
    const exportPath=await page.getByRole('link',{name:'Export stats',exact:true}).getAttribute('href');
    const response=await context.request.get(new URL(exportPath,origin).href);
    assert.equal(response.status(),200);
    const exported=JSON.parse(execFileSync('python3',['-c',`
import sys,zipfile,io,csv,json
z=zipfile.ZipFile(io.BytesIO(sys.stdin.buffer.read()))
print(json.dumps({'summary':list(csv.DictReader(io.StringIO(z.read('summary.csv').decode()))),'sources':list(csv.DictReader(io.StringIO(z.read('sources.csv').decode()))),'note':z.read('readme.txt').decode()}))
`],{input:await response.body(),encoding:'utf8'}));
    assert.equal(exported.sources[0].visitors,'8');
    assert.match(exported.note,/Historical data partly or wholly excluded from:/);
    assert.match(exported.note,/page/);

    await page.goto(`${base}&country=US`);
    assert.equal(await visitors(),7);
    await page.locator('.panel-region .data-row').filter({hasText:'California'}).click();
    assert.equal(await visitors(),6);
    assert.match(await page.locator('.panel-city').innerText(),/San Francisco/);
    assert.equal(await page.locator('.panel-city .data-row strong').innerText(),'6');
    assert.equal(await page.locator('.panel-city .import-report-note').count(),0);
    await page.setViewportSize({width:390,height:844});
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
    assert.equal(await page.locator('header.nav').count(),1);
    await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
    await page.screenshot({path:join(output,'imported-location-overview-mobile.png'),animations:'disabled'});
    await page.locator('.panel-city').scrollIntoViewIfNeeded();
    await page.screenshot({path:join(output,'imported-location-report-mobile.png'),animations:'disabled'});
    await page.setViewportSize({width:1440,height:1050});
    await page.goto(`${base}&browser=Chrome`);
    assert.equal(await visitors(),8);
    await page.locator('.panel-browser_version .data-row').filter({has:page.locator('.row-label', {hasText:/^1$/})}).click();
    assert.equal(await visitors(),5);
    assert.equal(await page.locator('.panel-browser_version .import-report-note').count(),0);
    await page.goto(`${base}&browser=Chrome&country=US`);
    assert.equal(await visitors(),1);
    assert.match(await page.locator('.import-note').first().innerText(),/Historical data is excluded/);
    await page.goto(`${base.replace('compare=off','compare=custom')}&source=Google&metric=visitors&compare_from=2025-10-01&compare_to=2025-10-01`);
    assert.equal(await visitors(),8);
    await page.locator('.chart-hit').first().hover();
    assert.equal(await page.locator('.chart-tooltip-previous strong').innerText(),'4');
    execFileSync('python3',['-c',`
import sqlite3,sys
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
batch=c.execute("INSERT INTO import_batches(site_id,from_day,to_day,imported_at) VALUES(5,'2025-09-30','2025-09-30',0)").lastrowid
c.execute("INSERT INTO import_daily VALUES(5,'2025-09-30',?,30,60)",(batch,))
c.execute("INSERT INTO import_breakdowns VALUES(5,'2025-09-30','browser','Chrome',?,60)",(batch,))
c.execute("INSERT INTO import_metrics VALUES(5,'2025-09-30','browser','Chrome',?,30,30,10,300,NULL)",(batch,))
c.commit()
`,database]);
    await page.goto(`${origin}/sites/5?from=2025-09-30&to=2025-10-03&browser=Chrome&browser_version=1&compare=off`);
    assert.equal(await visitors(),5);
    assert.match(await page.locator('.import-note').first().innerText(),/Some historical data cannot be included/);
    await page.locator('.dashboard-menu>summary').click();
    assert.equal(await page.locator('.dashboard-menu').getByRole('link',{name:'Hours',exact:true}).count(),0,'Partial daily history still prevents hourly charts');
    const noScript=await browser.newContext({javaScriptEnabled:false,storageState:await context.storageState()});
    try {
      const p=await noScript.newPage();
      await p.goto(base);
      await p.locator('.dashboard-menu>summary').click();
      await p.getByRole('switch',{name:'Include imported data'}).click();
      assert.equal(await p.locator('.metrics .metric').first().getAttribute('data-value'),'1');
      await p.locator('.dashboard-menu>summary').click();
      await p.getByRole('switch',{name:'Include imported data'}).click();
      assert.equal(await p.locator('.metrics .metric').first().getAttribute('data-value'),'11');
      await p.goto(`${base}&country=US`);
      await p.locator('.panel-region .data-row').filter({hasText:'California'}).click();
      assert.equal(await p.locator('.metrics .metric').first().getAttribute('data-value'),'6');
      assert.match(await p.locator('.panel-city').innerText(),/San Francisco/);
    } finally {await noScript.close();}
  } finally {rmSync(folder,{recursive:true,force:true});}
  await page.goto(`${origin}/sites/1`);
}
