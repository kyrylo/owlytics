import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

export async function checkCrossdayReports({context,browser,origin,database,output}) {
  execFileSync('python3',['-c',`
import datetime,sqlite3,sys
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
c.execute("INSERT INTO sites VALUES(99,'crossday.example','UTC',0),(100,'engagement-days.example','UTC',0)")
before=int(datetime.datetime(2026,10,20,tzinfo=datetime.timezone.utc).timestamp())
for site,person,stamp,path,source in [(99,1,1793059140,'/before','Google'),(99,1,1793059260,'/seen','Google'),(99,2,1793059320,'/seen','GitHub'),(99,1,1793149200,'/seen','Google'),(99,3,before-60,'/before','Google'),(99,3,before+60,'/seen','Google'),(100,1,1793059140,'/same','Google'),(100,1,1793059260,'/same','Google')]:
 day=str(datetime.datetime.fromtimestamp(stamp,datetime.timezone.utc).date())
 c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(?,?,?,?,randomblob(16),?,?,'US','Chrome','Desktop')",(site,stamp,day,person.to_bytes(16,'big'),path,source))
 if site==100:c.execute('INSERT INTO page_engagement VALUES(last_insert_rowid(),?,?)',(1000 if stamp==1793059140 else 2000,20 if stamp==1793059140 else 80))
c.commit()
`,database]);
  const storageState={cookies:(await context.storageState()).cookies,origins:[]},errors=[];
  const base=`${origin}/sites/99?from=2026-10-26&to=2026-10-28&compare=off&metric=visitors`;
  for(const [scheme,width,javaScriptEnabled] of [['light',1440,true],['dark',390,true],['light',1440,false]]) {
    const owner=await browser.newContext({storageState,colorScheme:scheme,viewport:{width,height:1000},javaScriptEnabled});
    try {
      const page=await owner.newPage();page.on('pageerror',e=>errors.push(e.message));
      const points=()=>page.locator('.chart-hit').evaluateAll(es=>es.map(e=>e.dataset.value));
      for(const filter of ['', '&browser=Chrome']) {
        await page.goto(base+'&interval=day'+filter);
        for(const [label,value] of [['visitors','2'],['total visits','3'],['pageviews','4']]) {
          assert.equal(await page.getByRole('link',{name:`Show ${label}`,exact:true}).getAttribute('data-value'),value);
        }
        assert.deepEqual(await points(),['0','2','1']);
        await page.goto(base.replace('metric=visitors','metric=visit_duration')+'&interval=day'+filter);
        assert.deepEqual(await points(),['—','1m 00s','0s']);
        for(const interval of ['week','month']) {
          await page.goto(base+'&interval='+interval+filter);assert.deepEqual(await points(),['2']);
        }
      }
      await page.goto(base.replace('compare=off','compare=custom&compare_from=2026-10-19&compare_to=2026-10-21')+'&interval=week');
      const point=page.locator('.chart-hit');
      assert.equal(await point.getAttribute('data-value'),'2');
      assert.equal(await point.getAttribute('data-previous-value'),'1');
      if(javaScriptEnabled) await page.screenshot({path:join(output,`crossday-week-${scheme}.png`),fullPage:true});
      for(const [metric,value] of [['time_on_page','3s'],['scroll_depth','80%']]) {
        await page.goto(`${origin}/sites/100?from=2026-10-26&to=2026-10-27&page=%2Fsame&compare=off&interval=week&metric=${metric}`);
        assert.deepEqual(await points(),[value]);
      }
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),true);
    } finally {await owner.close();}
  }
  for(const [from,interval,label,views] of [['2026-10-26','week','2026-10-26','4'],['2026-10-26','month','2026-10-01','4'],['2026-10-27','week','2026-10-27','3']]) {
    const response=await context.request.get(`${origin}/sites/99/export?from=${from}&to=2026-10-28&compare=off&interval=${interval}`);
    assert.equal(response.status(),200);
    const rows=JSON.parse(execFileSync('python3',['-c',`
import csv,io,json,sys,zipfile
z=zipfile.ZipFile(io.BytesIO(sys.stdin.buffer.read()))
print(json.dumps(list(csv.reader(io.StringIO(z.read('visitors.csv').decode())))))
`],{input:await response.body(),encoding:'utf8'}));
    assert.equal(rows.length,2);
    assert.deepEqual(rows[1],[label,'2','3',views,'1.33','66.67','40.0']);
  }
  assert.deepEqual(errors,[]);
}
