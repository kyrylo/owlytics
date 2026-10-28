import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

export async function checkVisitBreakdowns({context,browser,origin,database,output}) {
  execFileSync('python3',['-c',`
import datetime,sqlite3,sys,time,zoneinfo
now=int(time.time())
name='UTC' if 1<=datetime.datetime.fromtimestamp(now,datetime.timezone.utc).hour<23 else 'Pacific/Honolulu'
zone=zoneinfo.ZoneInfo(name)
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
c.execute("INSERT INTO sites VALUES(92,'visit-reports.example',?,0)",(name,))
for person,offset,path in [(0,-120,'/entry'),(0,120,'/seen'),(0,240,'/finish'),(1,300,'/seen')]:
 stamp=now-86400+offset
 day=str(datetime.datetime.fromtimestamp(stamp,zone).date())
 labels=['Google','US','US: US-CA','US: 5391959','Chrome','130','macOS','14','Desktop'] if person==0 else ['GitHub','PH','PH: PH-00','PH: 1701668','Safari','18','iOS','18','Mobile']
 c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,region,city,browser,browser_version,os,os_version,device) VALUES(92,?,?,?,randomblob(16),?,?,?,?,?,?,?,?,?,?)",(stamp,day,person.to_bytes(16,'big'),path,*labels))
c.execute("INSERT INTO session_acquisition SELECT id,'Referral',source,'referral','seed','','',NULL FROM sessions WHERE site_id=92")
c.commit()
`,database]);
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  const owner=await browser.newContext({storageState});
  const base=`${origin}/sites/92?period=24h&compare=off`;
  const columns={
    source:['Visitors','Bounce rate','Visit duration'],
    referrer:['Visitors','Bounce rate','Visit duration'],
    channel:['Visitors','Bounce rate','Visit duration'],
    utm_campaign:['Visitors','Bounce rate','Visit duration'],
    country:['Visitors','Bounce rate','Visit duration'],region:['Visitors','Bounce rate','Visit duration'],city:['Visitors','Bounce rate','Visit duration'],
    browser:['Visitors','Bounce rate','Visit duration'],
    browser_version:['Visitors','Bounce rate','Visit duration'],
    os:['Visitors','Bounce rate','Visit duration'],
    os_version:['Visitors','Bounce rate','Visit duration'],
    device:['Visitors','Bounce rate','Visit duration'],
    entry:['Unique entrances','Total entrances','Bounce rate','Visit duration'],
    exit:['Unique exits','Total exits','Exit rate'],
    page:['Visitors','Pageviews','Bounce rate','Time on page','Scroll depth']
  };
  const errors=[];
  try {
    const page=await owner.newPage();page.on('pageerror',error=>errors.push(error.message));
    for(const [scheme,width] of [['light',1440],['dark',390]]) {
      await page.emulateMedia({colorScheme:scheme});await page.setViewportSize({width,height:1000});
      for(const [detail,expected] of Object.entries(columns)) {
        const parent=detail==='browser_version'?'&browser=Chrome':detail==='os_version'?'&os=macOS':'';
        await page.goto(base+'&detail='+detail+parent);
        const table=page.locator('.expanded-report .report-table');
        const headers=(await table.locator('thead th').allTextContents()).slice(1).map(s=>s.replace(/[↑↓]/g,'').trim());
        assert.deepEqual(headers,expected,detail);
        assert.equal(await page.getByRole('link',{name:'Show pageviews',exact:true}).getAttribute('data-value'),parent?'2':'3');
        if(detail==='source') {
          const google=table.locator('tbody tr').filter({has:page.getByRole('link',{name:'Google',exact:true})});
          assert.deepEqual(await google.locator('td').allTextContents(),['1','0%','6m 00s']);
          await page.screenshot({path:join(output,`visit-breakdown-source-${scheme}.png`),fullPage:true});
        }
        if(detail==='browser') {
          const chrome=table.locator('tbody tr').filter({has:page.getByRole('link',{name:'Chrome',exact:true})});
          assert.deepEqual(await chrome.locator('td').allTextContents(),['1','0%','6m 00s']);
          await table.getByRole('link',{name:'Bounce rate',exact:true}).click();
          await page.waitForURL(url=>url.searchParams.get('sort')==='bounce_rate');
          assert.equal(await table.locator('thead th[aria-sort="descending"]').innerText(),'Bounce rate ↓');
          await page.screenshot({path:join(output,`visit-breakdown-browser-${scheme}.png`),fullPage:true});
        }
        if(detail==='entry') {
          const entry=table.locator('tbody tr').filter({has:page.getByRole('link',{name:'/entry',exact:true})});
          assert.deepEqual(await entry.locator('td').allTextContents(),['1','1','0%','6m 00s']);
        }
        assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),true,detail);
      }
    }
    const response=await owner.request.get(base.replace('/sites/92?','/sites/92/export?'));
    assert.equal(response.status(),200);
    const exported=JSON.parse(execFileSync('python3',['-c',`
import csv,io,json,sys,zipfile
z=zipfile.ZipFile(io.BytesIO(sys.stdin.buffer.read()))
print(json.dumps({name:list(csv.reader(io.StringIO(z.read(name).decode()))) for name in ['sources.csv','countries.csv','browsers.csv','entry_pages.csv','pages.csv']}))
`],{input:await response.body(),encoding:'utf8'}));
    assert.deepEqual(exported['sources.csv'][0],['source','visitors','bounce_rate','visit_duration']);
    assert.deepEqual(exported['sources.csv'].find(r=>r[0]==='Google'),['Google','1','0.00','360.00']);
    assert.deepEqual(exported['countries.csv'][0],['country','visitors','bounce_rate','visit_duration']);
    assert.deepEqual(exported['browsers.csv'][0],['browser','visitors','bounce_rate','visit_duration']);
    assert.deepEqual(exported['entry_pages.csv'][0],['entry','visitors','visits','bounce_rate','visit_duration']);
    assert.equal(exported['pages.csv'][0].includes('pageviews'),true);
    assert.deepEqual(errors,[]);
  } finally {await owner.close();}
  const plain=await browser.newContext({storageState,javaScriptEnabled:false});
  try {
    const page=await plain.newPage();await page.goto(base+'&detail=browser');
    assert.deepEqual((await page.locator('.report-table thead th').allTextContents()).slice(1).map(s=>s.replace(/[↑↓]/g,'').trim()),columns.browser);
    await page.locator('.report-table').getByRole('link',{name:'Bounce rate',exact:true}).click();
    await page.waitForURL(url=>url.searchParams.get('sort')==='bounce_rate');
    assert.equal(await page.locator('.report-table tbody tr').count(),2);
  } finally {await plain.close();}
}
