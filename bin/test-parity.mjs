#!/usr/bin/env node
// Report parity checks run against a disposable local database.
import assert from 'node:assert/strict';
import {selectCalendarRange} from '../tests/calendar-helpers.mjs';
import {spawn,execFileSync} from 'node:child_process';
import {mkdtempSync,mkdirSync,rmSync,writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {resolve,join} from 'node:path';
import {createRequire} from 'node:module';
import {createServer} from 'node:net';
import {checkFilters} from '../tests/filter-browser.mjs';
import {checkAcquisition} from '../tests/acquisition-browser.mjs';
import {checkMenus} from '../tests/menu-browser.mjs';
import {checkImports} from '../tests/import-browser.mjs';
import {checkRealtime} from '../tests/realtime-browser.mjs';
import {checkNavigation} from '../tests/navigation-browser.mjs';
import {checkPreferences} from '../tests/preferences-browser.mjs';
import {checkChartFormatting} from '../tests/chart-format-browser.mjs';
import {checkSourceIcons} from '../tests/source-icons-browser.mjs';
import {checkRowComparisons} from '../tests/row-comparison-browser.mjs';
import {checkSessionMeans} from '../tests/session-means-browser.mjs';
import {checkVisitAttributes} from '../tests/visit-attributes-browser.mjs';
import {checkVisitBreakdowns} from '../tests/visit-breakdowns-browser.mjs';
import {checkCrossdayReports} from '../tests/crossday-browser.mjs';
import {checkEngagementFocus} from '../tests/engagement-focus-browser.mjs';
import {checkReportWindows} from '../tests/report-windows-browser.mjs';
import {checkReportPercentages} from '../tests/report-percentages-browser.mjs';
import {checkReportOverlay} from '../tests/report-overlay-browser.mjs';
import {checkCalendars} from '../tests/calendar-browser.mjs';
import {checkHeadlineComparisons} from '../tests/headline-comparison-browser.mjs';
const {chromium}=createRequire(import.meta.url)('playwright');
const folder=mkdtempSync(join(tmpdir(),'owlytics-parity-'));
const database=join(folder,'test.sqlite');
const binary=resolve(process.env.OWLYTICS_BINARY||'target/release/owlytics');
const output=resolve('tmp/parity-checks');mkdirSync(output,{recursive:true});
const password='disposable report parity';let child,browser;
try {
 const reservation=createServer();await new Promise(r=>reservation.listen(0,'127.0.0.1',r));const port=reservation.address().port;await new Promise(r=>reservation.close(r));const origin=`http://127.0.0.1:${port}`;
 const hash=execFileSync(binary,['hash-password'],{input:password,encoding:'utf8'}).trim();
 child=spawn(binary,[],{env:{...process.env,OWLYTICS_DATABASE:database,OWLYTICS_PASSWORD_HASH:hash,OWLYTICS_BIND:`127.0.0.1:${port}`,OWLYTICS_URL:origin,OWLYTICS_TRUSTED_PROXIES:''},stdio:['ignore','pipe','pipe']});
 await new Promise((done,fail)=>{const t=setTimeout(()=>fail(new Error('startup timeout')),15000);child.stdout.on('data',d=>{if(String(d).includes('Owlytics listening')){clearTimeout(t);done();}});child.stderr.on('data',d=>process.stderr.write(d));child.once('exit',c=>fail(new Error(`Exited ${c}`)));});
 execFileSync('python3',['-c',`
import sqlite3,sys,hashlib,datetime,time
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)")
now=int(time.time());today=datetime.datetime.now(datetime.timezone.utc).date()
for offset in range(57):
 day=today-datetime.timedelta(days=offset)
 for person in range(10+offset%7):
  visitor=hashlib.sha256(f'{day}-{person}'.encode()).digest()[:16]
  start=int(datetime.datetime.combine(day,datetime.time(8+person%9),datetime.timezone.utc).timestamp())
  if offset==0:start=now-240+person*3
  for step in range(1+person%4):
   stamp=start+step*25
   path=['/','/pricing','/docs','/blog'][step]
   request=hashlib.sha256(f'{day}-{person}-{step}'.encode()).digest()[:16]
   browser=['Chrome','Safari','Firefox'][person%3]
   country=['US','PH','DE'][person%3]
   region=['US: US-CA','PH: PH-00','DE: DE-BE'][person%3]
   city=['US: 5391959','PH: 1701668','DE: 2950159'][person%3]
   source=['google.com','github.com','', 'news.ycombinator.com'][person%4]
   c.execute('INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device,browser_version,os,os_version,region,city,referrer) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',(1,stamp,str(day),visitor,request,path,source,country,browser,['Desktop','Mobile','Tablet'][person%3],['130.0','18.1','131.0'][person%3],['macOS','iOS','Windows'][person%3],['14.0','18.1','10'][person%3],region,city,source+'/article' if source else ''))
day=today-datetime.timedelta(days=2)
for extra in range(110):
 request=hashlib.sha256(f'extra-{extra}'.encode()).digest()[:16]
 stamp=int(datetime.datetime.combine(day,datetime.time(12),datetime.timezone.utc).timestamp())
 c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?,?,?,?,?,'','','Chrome','Desktop')",(stamp,str(day),request,request,f'/article/{extra:03}'))
c.execute("INSERT INTO page_engagement SELECT id,15000+(id%100)*1000,20+(id%81) FROM events")
c.execute("INSERT INTO daily_engagement SELECT site_id,day,path,SUM(time_ms),COUNT(*),SUM(depth) FROM (SELECT e.site_id,e.day,e.path,e.session_id,SUM(g.time_ms) AS time_ms,MAX(g.scroll_depth) AS depth FROM events e JOIN page_engagement g ON g.event_id=e.id GROUP BY e.site_id,e.day,e.path,e.session_id) GROUP BY site_id,day,path")
c.execute("INSERT INTO sites VALUES(2,'dst.example','America/New_York',0),(3,'history.example','UTC',0)")
for site,instant,day in [(2,'2025-11-03T04:10:00+00:00','2025-11-02'),(2,'2025-11-04T04:10:00+00:00','2025-11-03'),(3,'2025-10-03T12:00:00+00:00','2025-10-03')]:
 stamp=int(datetime.datetime.fromisoformat(instant).timestamp())
 c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(?,?,?,randomblob(16),randomblob(16),'/','','US','Chrome','Desktop')",(site,stamp,day))
c.execute("INSERT INTO import_batches VALUES(1,3,'2025-10-01','2025-10-01',0)")
c.execute("INSERT INTO import_daily VALUES(3,'2025-10-01',1,10,20)")
c.commit()
`,database]);
 // Detached keepalive requests use Chromium's process User-Agent, so a
 // context-only override would let the bot filter discard unload measurements.
 browser=await chromium.launch({headless:true,args:['--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36'],...(process.env.OWLYTICS_BROWSER?{executablePath:process.env.OWLYTICS_BROWSER}:{})});
 const context=await browser.newContext({viewport:{width:1440,height:1050},colorScheme:'light',locale:'en-GB'});const page=await context.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
 page.on('console',message=>{if(message.type()==='error' && /Content Security Policy/.test(message.text()))errors.push(message.text());});
 await page.goto(`${origin}/login`);await page.getByLabel('Password',{exact:true}).fill(password);await page.getByRole('button',{name:'Sign in',exact:true}).click();await page.waitForURL(`${origin}/`);
 await page.goto(`${origin}/sites/1?period=28d&compare=previous&compare_match=date`);
 assert.equal(await page.locator('.metrics .metric').count(),6);
 assert.equal(await page.locator('.report-panel').count(),4);
 assert.ok(await page.getByText('current visitors',{exact:false}).count());
 const screenshot=async name=>{await page.screenshot({path:join(output,`${name}.png`),fullPage:true});assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`${name}: horizontal overflow`);};
 await screenshot('dashboard-light');
 const otherRequests=[];page.on('request',request=>{if(!request.url().startsWith(origin))otherRequests.push(request.url());});
 const hits=page.locator('.chart-hit'), tooltip=page.getByRole('tooltip');
 assert.equal(await page.locator('.chart[data-interactive=true]').count(),1);
 assert.equal(await hits.locator('title').count(),0,'Native tooltips are replaced only after enhancement');
 assert.equal(await page.locator('.chart-hit[tabindex="0"]').count(),1,'Chart has one keyboard tab stop');
 await hits.first().hover();
 assert.equal(await tooltip.isVisible(),true);
 assert.equal(await tooltip.locator('.chart-tooltip-current strong').innerText(),'10');
 assert.equal(await tooltip.locator('.chart-tooltip-previous strong').innerText(),'10');
 assert.equal(await tooltip.locator('small').innerText(),'→ 0%');
 assert.equal(await page.locator('.chart-cursor line').getAttribute('x1'),'0.00');
 await tooltip.hover();
 assert.equal(await tooltip.isVisible(),true,'Details stay open when the pointer enters them');
 await screenshot('chart-tooltip-light');
 await page.mouse.move(0,0);
 assert.equal(await tooltip.isVisible(),false);
 await hits.first().focus();
 await page.keyboard.press('ArrowRight');
 assert.equal(await hits.nth(1).evaluate(point=>point===document.activeElement),true);
 assert.equal(await tooltip.locator('.chart-tooltip-current span').innerText(),await hits.nth(1).getAttribute('data-date'));
 await page.keyboard.press('End');
 assert.equal(await hits.last().evaluate(point=>point===document.activeElement),true);
 await page.keyboard.press('Home');
 assert.equal(await hits.first().evaluate(point=>point===document.activeElement),true);
 await page.keyboard.press('Escape');
 assert.equal(await tooltip.isVisible(),false);
 assert.equal(await page.locator('.chart-cursor').getAttribute('hidden'),'');
 assert.equal(await page.locator('.chart-hit:focus-visible').count(),1);
 assert.notEqual(await page.locator('.chart-hit:focus-visible').evaluate(point=>getComputedStyle(point).stroke),'none','Escape leaves a visible keyboard focus indicator');
 await page.keyboard.press('ArrowRight');
 assert.equal(await tooltip.isVisible(),true);
 await page.setViewportSize({width:320,height:1050});
 await page.emulateMedia({colorScheme:'dark'});
 await screenshot('chart-tooltip-dark-320');
 const inside=await tooltip.evaluate(element=>{const tip=element.getBoundingClientRect(),chart=element.parentElement.querySelector('.chart').getBoundingClientRect();return tip.left>=chart.left-1&&tip.right<=chart.right+1&&tip.top>=chart.top&&tip.bottom<=chart.bottom;});
 assert.ok(inside,'Tooltip stays inside the narrow chart after resizing');
 await page.keyboard.press('Tab');
 assert.equal(await tooltip.isVisible(),false);
 await page.goto(`${origin}/sites/1?compare=off`);
 await hits.first().hover();
 assert.equal(await tooltip.locator('.chart-tooltip-previous').isVisible(),false);
 assert.equal(await tooltip.locator('small').isVisible(),false);
 await page.setViewportSize({width:1440,height:1050});await page.emulateMedia({colorScheme:'light'});
 const touch=await browser.newContext({viewport:{width:390,height:844},hasTouch:true,isMobile:true,storageState:await context.storageState()});
 const touchPage=await touch.newPage();touchPage.on('pageerror',e=>errors.push(e.message));
 await touchPage.goto(`${origin}/sites/1?period=28d&compare=previous&compare_match=date`);
 await touchPage.locator('.chart-hit').first().tap();
 assert.equal(await touchPage.getByRole('dialog',{name:'Chart details'}).isVisible(),true,'Tap opens persistent details');
 assert.equal(await touchPage.locator('.chart-tooltip-current strong').innerText(),'10');
 await touchPage.screenshot({path:join(output,'chart-tooltip-touch.png'),fullPage:true});
 await touchPage.locator('.chart-scale').tap();
 assert.equal(await touchPage.locator('.chart-tooltip').isVisible(),false,'Tap outside closes details');
 await touch.close();
 await page.goto(`${origin}/sites/2?from=2025-11-03&to=2025-11-03&interval=hour&compare=previous&compare_match=date`);
 await hits.last().focus();
 assert.equal(await hits.count(),25);
 assert.equal(await tooltip.locator('.chart-tooltip-current').isVisible(),false,'A comparison-only interval has no invented current date');
 assert.equal(await tooltip.locator('.chart-tooltip-previous span').innerText(),'Nov 2, 23:00 EST');
 assert.equal(await tooltip.locator('.chart-tooltip-previous strong').innerText(),'1');
 assert.equal(await page.locator('.chart-marker').first().getAttribute('d'),'');
 assert.equal(await page.locator('.chart-marker-previous').getAttribute('d'),'M1000.00 20.00h0');
 await page.goto(`${origin}/sites/3?from=2025-10-03&to=2025-10-04&interval=day&compare=previous&compare_match=date&metric=visits`);
 await hits.first().focus();
 assert.deepEqual(await page.locator('.chart-scale span').allTextContents(),['1','0'],'Small counts have distinct integer ticks');
 assert.equal(await tooltip.locator('.chart-tooltip-current strong').innerText(),'1');
 assert.equal(await tooltip.locator('.chart-tooltip-previous strong').innerText(),'—');
 assert.equal(await tooltip.locator('small').isVisible(),false,'Unknown history has no percentage change');
 assert.equal(await page.locator('.chart-marker-previous').getAttribute('d'),'','Unknown history has no marker');
 await screenshot('chart-unknown-history');
 await page.goto(`${origin}/sites/1?period=28d&compare=previous&compare_match=date`);
 const mapCountry=page.locator('.country-map a[data-country="US"]');
 await mapCountry.focus();await page.keyboard.press('Enter');
 await page.waitForURL(url=>url.searchParams.get('country')==='US');
 assert.equal(new URL(page.url()).searchParams.get('country'),'US');
 assert.equal(await page.locator('.panel-region').count(),1);
 await page.locator('.panel-region .data-row').filter({hasText:'California'}).click();
 assert.equal(await page.locator('.panel-city').count(),1);
 assert.match(await page.locator('.panel-city').innerText(),/San Francisco/);
 await screenshot('location-drilldown');
 await page.getByRole('link',{name:'Remove country filter',exact:true}).click();await page.waitForFunction(()=>!document.querySelector('main[aria-busy=true]'));
 assert.equal(new URL(page.url()).searchParams.has('region'),false);
 assert.equal(await page.locator('.country-map').count(),1);
 await page.getByRole('link',{name:'Countries',exact:true}).click();
 assert.equal(await page.locator('.panel-country').count(),1);
 await page.locator('.panel-country .expand-report').click();
 await page.locator('.expanded-report').getByRole('link',{name:'Bounce rate',exact:true}).click();
 assert.equal(new URL(page.url()).searchParams.get('sort'),'bounce_rate');
 await page.getByRole('link',{name:'Close expanded report',exact:true}).click();
 await page.getByRole('link',{name:'Map',exact:true}).click();
 assert.equal(await page.locator('.expanded-report').count(),0);
 assert.equal(await page.locator('.country-map').count(),1);
 assert.deepEqual(otherRequests,[],'Locations must not fetch third-party maps or labels');
 await page.locator('.panel-page .data-row').filter({hasText:'/pricing'}).click();
 await page.getByRole('link',{name:'Show time on page',exact:true}).click();
 assert.match(await page.locator('.chart').getAttribute('aria-label'),/^Time on page/);
 assert.notEqual(await page.locator('.metric[aria-current=true] strong').innerText(),'—');
 await page.getByRole('link',{name:'Show scroll depth',exact:true}).click();
 assert.match(await page.locator('.metric[aria-current=true] strong').innerText(),/%$/);
 await screenshot('page-engagement');
 await page.goto(`${origin}/sites/1?period=28d&compare=previous&compare_match=date`);
 for(const [name,selector] of [['Entry pages','.panel-entry'],['Exit pages','.panel-exit'],['Regions','.panel-region'],['Cities','.panel-city'],['OS','.panel-os'],['Devices','.panel-device']]) {
   await page.getByRole('link',{name,exact:true}).click();assert.equal(await page.locator(selector).count(),1);assert.ok(await page.locator(`${selector} .data-row`).count());
 }
 await screenshot('report-tabs');
 await page.goto(`${origin}/sites/1?period=28d&compare=previous&compare_match=date`);await page.locator('.panel-source .data-row').filter({hasText:'Google'}).click();assert.equal(await page.locator('.panel-referrer').count(),1);assert.ok((await page.locator('.panel-referrer').innerText()).includes('google.com/article'));
 await page.goto(`${origin}/sites/1?period=28d&compare=previous&compare_match=date`);await page.getByRole('link',{name:'Show bounce rate',exact:true}).click();assert.match(await page.locator('.metric[aria-current=true] strong').innerText(),/%$/);assert.match(await page.locator('.chart').getAttribute('aria-label'),/Bounce rate/);
 await page.locator('.date-picker>summary').click();
 const beforeToday=Date.now();
 await page.getByRole('link',{name:'Today',exact:true}).click();
 const todayHours=await page.locator('.chart-hit').count();
 // UTC fixture: include the current hour, but never draw future hours.
 // Account for a request crossing an hour boundary, including midnight.
 const expectedHours=[beforeToday,Date.now()+1000].map(time=>new Date(time).getUTCHours()+1);
 assert.ok(expectedHours.includes(todayHours),`Today has ${todayHours} hours; expected ${expectedHours}`);
 assert.match(await page.locator('.chart-hit').first().getAttribute('aria-label'),/00:00 UTC/);
 await page.locator('.date-picker>summary').click();await page.getByRole('link',{name:'Realtime',exact:true}).click();assert.equal(await page.locator('.chart-hit').count(),30);
 await page.goto(`${origin}/sites/1?period=28d&compare=previous&compare_match=date`);await page.getByRole('link',{name:'Browsers',exact:true}).click();await page.locator('.panel-browser .data-row').filter({hasText:'Chrome'}).click();assert.equal(await page.locator('.panel-browser_version').count(),1);assert.ok((await page.locator('.panel-browser_version').innerText()).includes('130.0'));
 await page.getByRole('link',{name:'OS',exact:true}).click();
 await page.locator('.panel-os .data-row').filter({hasText:'macOS'}).click();
 assert.ok((await page.locator('.panel-os_version').innerText()).includes('14.0'));
 await page.getByRole('link',{name:'Browser versions',exact:true}).click();
 assert.ok((await page.locator('.panel-browser_version').innerText()).includes('130.0'));
 await page.getByRole('link',{name:'Remove os filter'}).click();await page.waitForFunction(()=>!document.querySelector('main[aria-busy=true]'));
 await page.getByRole('link',{name:'Remove browser filter'}).click();await page.waitForFunction(()=>!document.querySelector('main[aria-busy=true]'));await page.getByRole('link',{name:'Map',exact:true}).click();await page.locator('.panel-map .expand-report').click();assert.equal(await page.locator('.expanded-report').count(),1);
 await page.goto(`${origin}/sites/1?detail=page`);
 const table=page.locator('.expanded-report .report-table');
 assert.equal(await table.locator('tbody tr').count(),50);
 await table.getByRole('link',{name:'Pageviews',exact:true}).click();
 assert.equal(new URL(page.url()).searchParams.get('sort'),'pageviews');
 assert.equal(await table.locator('th[aria-sort="descending"]').count(),1);
 await table.getByRole('link',{name:'Pageviews',exact:true}).click();
 assert.equal(new URL(page.url()).searchParams.get('order'),'asc');
 await page.getByRole('link',{name:'Next →',exact:true}).click();
 assert.equal(new URL(page.url()).searchParams.get('offset'),'50');
 await page.getByRole('searchbox',{name:'Search report'}).fill('/article/109');
 await page.getByRole('searchbox',{name:'Search report'}).press('Enter');
 await page.waitForURL(url=>url.searchParams.get('search')==='/article/109');
 await page.waitForFunction(()=>!document.querySelector('main[aria-busy=true]'));
 assert.equal(await table.locator('tbody tr').count(),1);
 assert.equal(new URL(page.url()).searchParams.has('offset'),false);
 assert.match(await table.innerText(),/article\/109/);
 await page.getByRole('link',{name:'Close expanded report',exact:true}).click();
 await page.getByRole('link',{name:'Entry pages',exact:true}).click();
 await page.locator('.panel-entry .expand-report').click();
 assert.equal(await page.locator('.expanded-report.panel-entry').count(),1);
 assert.equal(new URL(page.url()).searchParams.has('search'),false);
 await page.goto(`${origin}/sites/1?detail=page`);
 await screenshot('expanded-pages-desktop');
 await page.setViewportSize({width:390,height:844});await screenshot('expanded-pages-mobile');
 await page.goto(`${origin}/sites/1?period=28d&compare=previous&compare_match=date`);
 await page.getByLabel('Dashboard menu',{exact:true}).click();
 await screenshot('dashboard-menu-mobile');
 assert.equal(await page.getByRole('link',{name:'Days',exact:true}).getAttribute('aria-current'),'true');
 await page.getByRole('link',{name:'Weeks',exact:true}).click();
 assert.equal(new URL(page.url()).searchParams.get('interval'),'week');
 await page.locator('.chart-hit').nth(1).focus();
 assert.match(await tooltip.locator('.chart-tooltip-current span').innerText(),/^Week of \d{1,2} [A-Za-z]{3}(?: \d{2})?$/);
 await screenshot('chart-week-tooltip-mobile');
 await page.keyboard.press('Escape');
 await page.getByLabel('Dashboard menu',{exact:true}).click();
 const downloadPromise=page.waitForEvent('download');
 await page.getByRole('link',{name:'Export stats',exact:true}).click();
 const download=await downloadPromise;const archive=join(folder,'stats.zip');await download.saveAs(archive);
 assert.match(download.suggestedFilename(),/^owlytics-example\.com-.*\.zip$/);
 execFileSync('python3',['-c',`import zipfile,csv,sys,io
with zipfile.ZipFile(sys.argv[1]) as z:
 assert z.testzip() is None
 assert 'summary.csv' in z.namelist()
 rows=list(csv.reader(io.TextIOWrapper(z.open('pages.csv'))))
 assert len(rows)==101
 assert rows[0]==['page','visitors','pageviews','bounce_rate','time_on_page','scroll_depth']
`,archive]);
 await page.goto(`${origin}/sites/1?period=28d&compare=previous&compare_match=date`);
 for(const scheme of ['light','dark']){await page.emulateMedia({colorScheme:scheme});for(const width of [1440,768,390,320]) {await page.setViewportSize({width,height:1050});await screenshot(`dashboard-${scheme}-${width}`);}}
 // Comparison forms preserve the report state; tables open after date selection.
 const date=offset=>new Date(Date.now()+offset*86400000).toISOString().slice(0,10);
 const state={browser:'Chrome',metric:'pageviews',pages_tab:'entry',interval:'day',sort:'name',order:'asc',search:'article'};
 await page.goto(`${origin}/sites/1?${new URLSearchParams({...state,from:date(-7),to:date(-1),compare:'previous',offset:'50'})}`);
 await page.locator('.comparison-picker>summary').click();
 await page.getByText('Custom period',{exact:true}).click();
 await screenshot('custom-comparison-dark-320');
 await selectCalendarRange(page,date(-45),date(-36),true);
 let params=new URL(page.url()).searchParams;
 for(const [key,value] of Object.entries({...state,compare:'custom',compare_from:date(-45),compare_to:date(-36),offset:'50'}))assert.equal(params.get(key),value,key);
 assert.equal(await hits.count(),10,'Custom comparison permits a different number of days');
 await page.locator('.date-picker>summary').click();
 await page.getByText('Custom range',{exact:true}).click();
 await selectCalendarRange(page,date(-14),date(-1));
 params=new URL(page.url()).searchParams;
 for(const [key,value] of Object.entries({...state,compare:'custom',compare_from:date(-45),compare_to:date(-36),from:date(-14),to:date(-1)}))assert.equal(params.get(key),value,key);
 assert.equal(params.has('offset'),false,'Changing the primary dates resets pagination');
 assert.equal(await hits.count(),14);
 await page.getByRole('link',{name:'Top pages',exact:true}).click();
 await page.locator('.panel-page .expand-report').click();
 await page.getByRole('searchbox',{name:'Search report'}).fill('/article/109');
 await page.getByRole('searchbox',{name:'Search report'}).press('Enter');
 await page.waitForURL(url=>url.searchParams.get('search')==='/article/109');
 await page.waitForFunction(()=>!document.querySelector('main[aria-busy=true]'));
 assert.equal(new URL(page.url()).searchParams.get('compare_from'),date(-45));
 assert.equal(await page.locator('.expanded-report tbody tr').count(),1);
 await page.getByRole('link',{name:'Close expanded report',exact:true}).click();
 await page.getByRole('link',{name:'Clear filters',exact:true}).click();await page.waitForFunction(()=>!document.querySelector('main[aria-busy=true]'));
 params=new URL(page.url()).searchParams;
 assert.equal(params.has('browser'),false);
 for(const key of ['compare','compare_from','compare_to','metric','interval'])assert.equal(params.get(key),key==='compare'?'custom':key==='compare_from'?date(-45):key==='compare_to'?date(-36):state[key]);
 await page.locator('.comparison-picker>summary').click();
 await page.getByRole('link',{name:'Year over year',exact:true}).click();
 await page.locator('.comparison-picker>summary').click();
 await page.getByRole('link',{name:'Match day of week',exact:true}).click();
 assert.equal(new URL(page.url()).searchParams.get('compare_match'),'weekday');
 await page.locator('.comparison-picker>summary').click();
 assert.equal(await page.getByRole('link',{name:'Match day of week',exact:true}).getAttribute('aria-current'),'true');
 await page.getByRole('link',{name:'Disable comparison',exact:true}).click();
 assert.equal(await page.locator('.comparison-picker').count(),0);
 assert.equal(await page.locator('.chart-previous').count(),0);
 await page.locator('.date-picker>summary').click();
 await page.getByRole('link',{name:'Compare',exact:true}).click();
 assert.equal(new URL(page.url()).searchParams.get('compare'),'previous');
 await page.goto(`${origin}/sites/1?period=realtime&interval=week&compare=year`);
 assert.equal(await hits.count(),30);
 assert.equal(await page.locator('.comparison-picker').count(),0);
 await page.getByLabel('Dashboard menu',{exact:true}).click();
 assert.equal(await page.getByRole('link',{name:'Minutes',exact:true}).getAttribute('aria-current'),'true');
 await page.locator('.date-picker>summary').click();
 await page.getByRole('link',{name:'Last 7 days',exact:true}).click();
 assert.equal(await hits.count(),7);
 assert.equal(new URL(page.url()).searchParams.get('compare'),'year');
 assert.equal(await hits.last().getAttribute('data-date'),new Date(`${date(-1)}T00:00Z`).toLocaleDateString('en-US',{timeZone:'UTC',month:'short',day:'numeric',year:'numeric'}));
 // Menus fit the viewport even when the toolbar wraps; Escape restores focus.
 for(const scheme of ['light','dark']) {
   await page.emulateMedia({colorScheme:scheme});
   for(const width of [1440,768,390,320]) {
     await page.setViewportSize({width,height:844});
     for(const menu of await page.locator('.dashboard-controls>.dropdown').all()) {
       await menu.locator(':scope>summary').click();
       const bounds=await menu.locator(':scope>.dropdown-content').boundingBox();
       assert.ok(bounds.x>=0&&bounds.x+bounds.width<=width+1,`Menu overflows ${width}px: ${JSON.stringify(bounds)}`);
       if(width===320)await screenshot(`date-menu-${scheme}-${await menu.getAttribute('class')}`);
       await menu.locator('a').first().focus();
       await page.keyboard.press('Escape');
       assert.equal(await menu.getAttribute('open'),null);
       assert.equal(await menu.locator(':scope>summary').evaluate(el=>el===document.activeElement),true);
     }
   }
 }
 await page.locator('.date-picker>summary').click();
 await page.locator('.metrics').click({position:{x:3,y:3}});
 assert.equal(await page.locator('.date-picker').getAttribute('open'),null,'Outside click closes dropdowns');
 const noScript=await browser.newContext({javaScriptEnabled:false,storageState:await context.storageState()});
 const noScriptPage=await noScript.newPage();
 await noScriptPage.goto(`${origin}/sites/1?period=28d&compare=previous&compare_match=date`);
 assert.equal(await noScriptPage.locator('.chart[data-interactive]').count(),0);
 assert.equal(await noScriptPage.locator('.chart-hit title').count(),28);
 await noScriptPage.locator('.chart-hit').first().focus();
 assert.match(await noScriptPage.locator('.chart-hit').first().getAttribute('aria-label'),/10 visitors.*Previous: 10/);
 assert.equal(await noScriptPage.getByRole('tooltip').isVisible(),false);
 await noScriptPage.locator('.country-map a[data-country="US"]').focus();
 await noScriptPage.keyboard.press('Enter');
 await noScriptPage.waitForURL(url=>url.searchParams.get('country')==='US');
 assert.equal(new URL(noScriptPage.url()).searchParams.get('country'),'US');
 assert.equal(await noScriptPage.locator('.panel-region').count(),1);
 await noScriptPage.locator('.comparison-picker>summary').click();
 await noScriptPage.getByText('Custom period',{exact:true}).click();
 await noScriptPage.getByLabel('Compare from',{exact:true}).fill(date(-14));
 await noScriptPage.getByLabel('Compare to',{exact:true}).fill(date(-8));
 await noScriptPage.getByRole('button',{name:'Compare',exact:true}).click();
 assert.equal(new URL(noScriptPage.url()).searchParams.get('compare'),'custom');
 assert.equal(new URL(noScriptPage.url()).searchParams.get('country'),'US');
 assert.equal(await noScriptPage.locator('.chart-hit title').count(),28);
 await noScript.close();
 await page.evaluate(()=>localStorage.clear());
 await checkFilters({page,context,browser,origin,output});
 await checkAcquisition({page,context,browser,origin,database,output});
 await checkImports({page,context,browser,origin,database,binary,output});
 await checkMenus({context,browser,origin,output});
 await checkPreferences({context,browser,origin,output});
 await checkRealtime({context,browser,origin,database,output});
 await checkNavigation({context,browser,origin,database,output});
 await checkChartFormatting({context,browser,origin,database,output});
 await checkSourceIcons({context,browser,origin,database,output});
 await checkRowComparisons({context,browser,origin,database,output});
 await checkSessionMeans({context,browser,origin,database,output});
 await checkVisitAttributes({context,browser,origin,database,output});
 await checkVisitBreakdowns({context,browser,origin,database,output});
 await checkReportPercentages({context,browser,origin,database,output});
 await checkReportOverlay({context,browser,origin,output});
 await checkCalendars({context,browser,origin,database,output});
 await checkHeadlineComparisons({context,browser,origin,database,output});
 await checkReportWindows({context,browser,origin,database,output});
 await checkCrossdayReports({context,browser,origin,database,output});
 await checkEngagementFocus({context,browser,origin,database,output});
 assert.deepEqual(errors,[]);writeFileSync(join(output,'results.json'),JSON.stringify({passed:true,checks:['site-local calendar selection, leap days, reversed ranges, comparison dates and no-JavaScript forms','previous headline totals and dates with missing imported metrics across themes, mobile and no JavaScript','expanded report overlay, live search, scroll containment and closing during a pending search','browser focus/blur handling and three-second engagement buffering with controlled focus state','cross-date visitor and visit counts, calendar engagement, comparisons and interval CSVs in light/dark and without JavaScript','local-midnight visit overlap, exact date bounds and full-visit metrics across UTC, fractional offsets and DST','event-based percentage audiences, unavailable denominators and known-location tables/CSVs in light/dark and without JavaScript','visit-based report counts and matching expanded columns, CSVs, light/dark and no JavaScript',
  'inherited visit location and software filters, page intersections, dark mobile and no JavaScript','rolling-window traffic and complete-visit averages in light/dark and without JavaScript','row comparisons with exact prior values, missing history, keyboard and touch details','known-zero growth and empty-period charts; unavailable row duration and bounce changes','local source and software SVGs, alias identity, safe referrer hosts and site icons','responsive site picker, zero external icon requests and no-script site navigation','unfinished intervals and historical partial weeks/months','browser 12/24-hour clocks with preserved site timezone and DST distinctions','rounded SVG axis ticks at desktop, 390px and 360px, including no JavaScript','saved per-site metrics and report tabs; per-period intervals with calendar aliases and URL precedence','minute charts, DST offsets, imported-history fallback and custom interval defaults','realtime five/thirty-minute counts and empty audiences','realtime refresh timing, edit/visibility pauses, recovery, session expiry and navigation races','import switch across reports, suggestions and CSVs, including queued filters and no JavaScript','per-site date/comparison preferences with explicit URL overrides and optional storage','date, custom-range, interval and comparison shortcuts','comparison defaults and additional date presets','grouped filter chips, operators and immediate multi-selection','queued filter changes preserve search and chart navigation','filter suggestions and pagination retain selections','filter editing without JavaScript','Escape priority and filter history navigation','six metrics','report tabs','local country map and keyboard navigation without JavaScript','country/region/city drilldown and filter removal','location bounce-rate sorting and compact visitor percentages','no third-party location requests','chart comparison tooltip and crosshair','arrow/Home/End/Escape/Tab chart controls','touch details and narrow-screen positioning','native no-JavaScript chart descriptions','hourly and realtime','browser version drilldown','expanded report sorting/search/pagination','interval menu','custom comparisons and weekday matching','comparison and primary date forms preserve filters and report state','preset ranges exclude today','dropdown bounds and keyboard dismissal','comparison forms without JavaScript','ZIP export','responsive light/dark']},null,2));
 if(process.env.OWLYTICS_PARITY_DEMO) execFileSync(binary,['backup',resolve(process.env.OWLYTICS_PARITY_DEMO)],{env:{...process.env,OWLYTICS_DATABASE:database}});
 console.log('Report parity browser checks passed.');
} finally {await browser?.close();if(child){child.kill('SIGTERM');await new Promise(resolve=>{child.once('exit',resolve);setTimeout(resolve,3000).unref();});}rmSync(folder,{recursive:true,force:true});}
