import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

export async function checkSessionMeans({context,browser,origin,database,output}) {
  execFileSync('python3',['-c',`
import datetime,sqlite3,sys,time,zoneinfo
now=int(time.time())
utc=datetime.datetime.fromtimestamp(now,datetime.timezone.utc)
name='UTC' if 1<=utc.hour<23 else 'Pacific/Honolulu'
zone=zoneinfo.ZoneInfo(name)
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
c.execute("INSERT INTO sites VALUES(90,'visit-means.example',?,0)",(name,))
for person,offset in [(0,-120),(0,120),(0,240),(1,300)]:
 stamp=now-86400+offset
 day=str(datetime.datetime.fromtimestamp(stamp,zone).date())
 c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(90,?,?,?,randomblob(16),'/','Google','US','Chrome','Desktop')",(stamp,day,person.to_bytes(16,'big')))
c.commit()
`,database]);
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  const owner=await browser.newContext({storageState});
  const errors=[];
  const base=`${origin}/sites/90?period=24h&metric=views_per_visit&compare=off`;
  try {
    const page=await owner.newPage();
    page.on('pageerror',error=>errors.push(error.message));
    for(const [scheme,width] of [['light',1440],['dark',390]]) {
      await page.emulateMedia({colorScheme:scheme});
      await page.setViewportSize({width,height:950});
      for(const filter of ['', '&source=Google']) {
        await page.goto(base+filter);
        for(const [label,value] of [['Visitors','2'],['Total visits','2'],['Pageviews','3'],['Views per visit','2'],['Bounce rate','50'],['Visit duration','180']]) {
          assert.equal(await page.getByRole('link',{name:`Show ${label.toLowerCase()}`,exact:true}).getAttribute('data-value'),value,label);
        }
        assert.ok(await page.locator('.chart-hit').count()>0);
        assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),true);
      }
      await page.screenshot({path:join(output,`session-means-${scheme}.png`),fullPage:true});
    }
    assert.deepEqual(errors,[]);
  } finally {await owner.close();}
  const plain=await browser.newContext({storageState,javaScriptEnabled:false});
  try {
    const page=await plain.newPage();
    await page.goto(base);
    assert.equal(await page.getByRole('link',{name:'Show views per visit',exact:true}).getAttribute('data-value'),'2');
    assert.ok(await page.locator('.chart-hit title').count()>0);
  } finally {await plain.close();}
}
