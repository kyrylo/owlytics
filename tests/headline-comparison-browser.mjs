import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

export async function checkHeadlineComparisons({context,browser,origin,database,output}) {
  execFileSync('python3',['-c',`
import datetime,sqlite3,sys
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
c.execute("INSERT INTO sites VALUES(110,'headlines.example','UTC',0),(111,'headline-import.example','UTC',0)")
for site,day,person,steps in [(110,'2025-01-31',1,2),(110,'2025-02-01',1,1),(110,'2025-02-01',2,2),(111,'2025-02-01',1,1)]:
 start=int(datetime.datetime.fromisoformat(day+'T12:00:00+00:00').timestamp())
 for step in range(steps):
  c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(?,?,?,?,randomblob(16),?,'Google','US','Chrome','Desktop')",(site,start+step*60,day,person.to_bytes(16,'big'),'/' if step==0 else '/done'))
c.execute("INSERT INTO import_batches VALUES(110,111,'2025-01-31','2025-01-31',0)")
c.execute("INSERT INTO import_daily VALUES(111,'2025-01-31',110,7,9)")
c.commit()
`,database]);
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  for(const [scheme,width,javaScriptEnabled] of [['light',1280,true],['dark',390,true],['light',320,true],['dark',768,true],['light',1280,false]]) {
    const owner=await browser.newContext({storageState,colorScheme:scheme,viewport:{width,height:900},javaScriptEnabled});
    try {
      const page=await owner.newPage();
      const base=`${origin}/sites/110?from=2025-02-01&to=2025-02-01&compare=previous&compare_match=date&interval=day`;
      await page.goto(base);
      for(const [name,current,previous] of [['visitors','2','1'],['total visits','2','1'],['pageviews','3','2'],['views per visit','1.50','2.00'],['bounce rate','50%','0%'],['visit duration','30s','1m 00s']]) {
        const card=page.getByRole('link',{name:`Show ${name}`,exact:true});
        assert.equal(await card.locator('strong').innerText(),current);
        assert.equal(await card.locator('.metric-previous>b').innerText(),previous);
        assert.deepEqual(await card.locator('.metric-date').allTextContents(),['Sat, 01 Feb','Fri, 31 Jan']);
      }
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),true);
      const overflow=await page.locator('.metric-date,.metric-previous>b').evaluateAll(elements=>elements.filter(e=>{
        const r=e.getBoundingClientRect(),card=e.closest('.metric').getBoundingClientRect();
        return r.right>card.right+1||r.bottom>card.bottom+1;
      }).length);
      assert.equal(overflow,0,'Dates and previous values stay inside their metric cards');
      await page.screenshot({path:join(output,`headline-comparison-${scheme}-${width}-${javaScriptEnabled}.png`),fullPage:true});
      await page.goto(base.replace('/110?','/111?'));
      assert.equal(await page.getByRole('link',{name:'Show visitors',exact:true}).locator('.metric-previous>b').innerText(),'7');
      for(const name of ['total visits','views per visit','bounce rate','visit duration']) {
        const card=page.getByRole('link',{name:`Show ${name}`,exact:true});
        assert.equal(await card.locator('.metric-previous>b').innerText(),'—','Missing imported metrics are not zero');
        assert.equal(await card.locator('.metric-change').count(),0);
      }
      await page.goto(base.replace('compare=previous','compare=off'));
      assert.equal(await page.locator('.metric-previous,.metric-date').count(),0);
      await page.goto(`${origin}/sites/110?period=realtime&compare=previous`);
      assert.equal(await page.locator('.metric-previous,.metric-date').count(),0);
    } finally { await owner.close(); }
  }
}
