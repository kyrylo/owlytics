import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

export async function checkReportWindows({context,browser,origin,database,output}) {
  const sites=[[96,'UTC','2026-10-27'],[97,'Asia/Kathmandu','2026-10-27'],[98,'America/New_York','2026-03-08']];
  execFileSync('python3',['-c',`
import datetime,json,sqlite3,sys,zoneinfo
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
for site,name,date in json.loads(sys.argv[2]):
 zone=zoneinfo.ZoneInfo(name);day=datetime.date.fromisoformat(date)
 start=int(datetime.datetime.combine(day,datetime.time(),zone).timestamp())
 end=int(datetime.datetime.combine(day+datetime.timedelta(days=1),datetime.time(),zone).timestamp())
 c.execute('INSERT INTO sites VALUES(?,?,?,0)',(site,'midnight-'+str(site)+'.example',name))
 for person,source,path,stamp in [(1,'Google','/entry',start-60),(1,'Google','/seen',start+60),(2,'GitHub','/seen',start+120),(3,'Outside','/seen',start-61),(4,'After','/seen',end)]:
  local_day=str(datetime.datetime.fromtimestamp(stamp,zone).date())
  c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(?,?,?,?,randomblob(16),?,?,'US','Chrome','Desktop')",(site,stamp,local_day,person.to_bytes(16,'big'),path,source))
c.commit()
`,database,JSON.stringify(sites)]);
  const storageState={cookies:(await context.storageState()).cookies,origins:[]},errors=[];
  for(const [scheme,width,javaScriptEnabled] of [['light',1440,true],['dark',390,true],['light',1440,false]]) {
    const owner=await browser.newContext({storageState,colorScheme:scheme,viewport:{width,height:1000},javaScriptEnabled});
    try {
      const page=await owner.newPage();page.on('pageerror',e=>errors.push(e.message));
      for(const [site,zone,day] of sites) {
        const base=`${origin}/sites/${site}?from=${day}&to=${day}&browser=Chrome&compare=off&interval=day&metric=views_per_visit`;
        for(const filter of ['', '&page=%2Fseen']) {
          await page.goto(base+filter);
          for(const [label,value] of [['visitors','2'],['total visits','2'],['pageviews','2'],['bounce rate',filter?'100':'50'],...(filter?[]:[['views per visit','1.5'],['visit duration','60']])]) {
            assert.equal(await page.getByRole('link',{name:`Show ${label}`,exact:true}).getAttribute('data-value'),value,`${zone}/${filter}/${label}`);
          }
          await page.goto(base+filter+'&detail=source');
          const rows=page.locator('.expanded-report tbody tr');assert.equal(await rows.count(),2);
          const google=rows.filter({has:page.getByRole('link',{name:'Google',exact:true})});
          assert.deepEqual(await google.locator('td').allTextContents(),['1','0%','2m 00s']);
          assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),true);
        }
        await page.goto(base+'&source=Google');
        assert.equal(await page.getByRole('link',{name:'Show views per visit',exact:true}).getAttribute('data-value'),'2');
        assert.equal(await page.getByRole('link',{name:'Show visit duration',exact:true}).getAttribute('data-value'),'120');
        if(site===97 && javaScriptEnabled) await page.screenshot({path:join(output,`midnight-report-${scheme}.png`),fullPage:true});
      }
    } finally {await owner.close();}
  }
  for(const [site,,day] of sites) {
    const response=await context.request.get(`${origin}/sites/${site}/export?from=${day}&to=${day}&browser=Chrome&compare=off`);
    assert.equal(response.status(),200);
    const rows=JSON.parse(execFileSync('python3',['-c',`
import csv,io,json,sys,zipfile
z=zipfile.ZipFile(io.BytesIO(sys.stdin.buffer.read()))
print(json.dumps(list(csv.DictReader(io.StringIO(z.read('sources.csv').decode())))))
`],{input:await response.body(),encoding:'utf8'}));
    assert.equal(rows.length,2);
    const google=rows.find(row=>row.source==='Google');
    assert.equal(google.visitors,'1');
    assert.equal(google.bounce_rate,'0.00');
    assert.equal(google.visit_duration,'120.00');
  }
  assert.deepEqual(errors,[]);
}
