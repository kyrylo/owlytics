import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

export async function checkReportPercentages({context,browser,origin,database,output}) {
  execFileSync('python3',['-c',`
import datetime,sqlite3,sys,time
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
c.execute("INSERT INTO sites VALUES(93,'locations.example','UTC',0),(94,'gap.example','UTC',0),(95,'overlap.example','UTC',0)")
for visitor,offset,country,region,city in [(1,0,'US','US: US-CA','US: 5391959'),(1,3600,'PH','PH: PH-00','PH: 1701668'),(1,7200,'','',''),(2,10,'US','',''),(3,20,'','',''),(4,30,'ZZ','',''),(5,40,'AQ','',''),(6,50,'US','US: Uncharted','US: Uncharted')]:
 c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,region,city,browser,device) VALUES(93,1793059200+?,'2026-10-27',?,randomblob(16),'/','Google',?,?,?,'Chrome','Desktop')",(offset,visitor.to_bytes(16,'big'),country,region,city))
now=int(time.time())
# A controlled gap spanning the report boundary, without waiting five minutes.
for site in [94,95]:
 for offset in [-400,1000]:
  stamp=now+offset;day=str(datetime.datetime.fromtimestamp(stamp,datetime.timezone.utc).date())
  c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(?,?,?,zeroblob(16),randomblob(16),'/','Google','US','Chrome','Desktop')",(site,stamp,day))
stamp=now-30;day=str(datetime.datetime.fromtimestamp(stamp,datetime.timezone.utc).date())
c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(95,?,?,randomblob(16),randomblob(16),'/','GitHub','PH','Safari','Mobile')",(stamp,day))
c.commit()
`,database]);
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  const errors=[];
  for(const [scheme,width,javaScriptEnabled] of [['light',1440,true],['dark',390,true],['light',1440,false]]) {
    const owner=await browser.newContext({storageState,colorScheme:scheme,viewport:{width,height:1000},javaScriptEnabled});
    try {
      const page=await owner.newPage();page.on('pageerror',e=>errors.push(e.message));
      const base=`${origin}/sites/93?from=2026-10-27&to=2026-10-27&compare=off`;
      for(const [tab,label,percentage] of [['country','United States','75.0%'],['region','California','50.0%'],['city','San Francisco','50.0%']]) {
        await page.goto(base+'&locations_tab='+tab);
        const panel=page.locator('.report-panel').filter({has:page.locator('.data-row').filter({hasText:label})}).first();
        const rows=panel.locator('.data-row');
        assert.equal(await rows.count(),3,tab);
        assert.doesNotMatch(await rows.allTextContents().then(s=>s.join(' ')),/not set|ZZ/);
        const row=rows.filter({has:page.locator('.row-label').filter({hasText:label})});
        assert.equal(await row.locator('.row-percentage').textContent(),percentage);
        await row.hover();
        if(tab==='country' && javaScriptEnabled) await page.screenshot({path:join(output,`location-percentages-${scheme}.png`),fullPage:true});
        await page.goto(base+'&detail='+tab);
        assert.equal(await page.locator('.expanded-report tbody tr').count(),3,tab);
      }
      await page.goto(`${origin}/sites/94?period=realtime&compare=off`);
      const source=page.locator('.panel-source .data-row');
      assert.equal(await source.count(),1);
      assert.equal(await source.locator('.row-percentage').textContent(),'—');
      await source.hover();
      if(javaScriptEnabled) await page.screenshot({path:join(output,`unavailable-percentage-${scheme}.png`),fullPage:true});
      await page.goto(`${origin}/sites/95?period=realtime&compare=off`);
      assert.deepEqual(await page.locator('.panel-source .row-percentage').allTextContents(),['100.0%','100.0%']);
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),true);
    } finally {await owner.close();}
  }
  const response=await context.request.get(`${origin}/sites/93/export?from=2026-10-27&to=2026-10-27&compare=off`);
  assert.equal(response.status(),200);
  const exported=JSON.parse(execFileSync('python3',['-c',`
import csv,io,json,sys,zipfile
z=zipfile.ZipFile(io.BytesIO(sys.stdin.buffer.read()))
print(json.dumps({name:list(csv.reader(io.StringIO(z.read(name).decode()))) for name in ['countries.csv','regions.csv','cities.csv']}))
`],{input:await response.body(),encoding:'utf8'}));
  for(const [name,rows] of Object.entries(exported)) {
    assert.equal(rows.length,4,name);
    assert.ok(rows.slice(1).every(row=>row[0] && !['ZZ','0','(not set)'].includes(row[0])),name);
  }
  assert.ok(exported['countries.csv'].some(row=>row[0]==='Antarctica'));
  assert.deepEqual(errors,[]);
}
