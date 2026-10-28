import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

export async function checkVisitAttributes({context,browser,origin,database,output}) {
  execFileSync('python3',['-c',`
import datetime,sqlite3,sys,time,zoneinfo
now=int(time.time())
name='UTC' if 1<=datetime.datetime.fromtimestamp(now,datetime.timezone.utc).hour<23 else 'Pacific/Honolulu'
zone=zoneinfo.ZoneInfo(name)
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
c.execute("INSERT INTO sites VALUES(91,'visit-labels.example',?,0)",(name,))
first=['US','US: US-CA','US: 5391959','Chrome','130','macOS','14','Desktop']
later=['PH','PH: PH-00','PH: 1701668','Firefox','131','Linux','1','Mobile']
for person,offset,path,labels in [(0,-600,'/first',first),(0,-540,'/later',later),(1,-400,'/other',later)]:
 stamp=now+offset
 day=str(datetime.datetime.fromtimestamp(stamp,zone).date())
 c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,region,city,browser,browser_version,os,os_version,device) VALUES(91,?,?,?,randomblob(16),?,'Google',?,?,?,?,?,?,?,?)",(stamp,day,person.to_bytes(16,'big'),path,*labels))
c.commit()
`,database]);
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  const owner=await browser.newContext({storageState});
  const base=`${origin}/sites/91?period=24h&compare=off`;
  const attributes={country:'US',region:'US: US-CA',city:'US: 5391959',browser:'Chrome',browser_version:'130',os:'macOS',os_version:'14',device:'Desktop'};
  const errors=[];
  const expect=async(page,views)=>{
    assert.equal(await page.getByRole('link',{name:'Show pageviews',exact:true}).getAttribute('data-value'),String(views));
    assert.equal(await page.getByRole('link',{name:'Show visitors',exact:true}).getAttribute('data-value'),views?'1':'0');
    assert.equal(await page.getByRole('link',{name:'Show total visits',exact:true}).getAttribute('data-value'),views?'1':'0');
  };
  try {
    const page=await owner.newPage();
    page.on('pageerror',error=>errors.push(error.message));
    for(const [key,value] of Object.entries(attributes)) {
      await page.goto(base+'&'+new URLSearchParams({[key]:value}));
      await expect(page,2);
    }
    for(const [scheme,width] of [['light',1440],['dark',390]]) {
      await page.emulateMedia({colorScheme:scheme});
      await page.setViewportSize({width,height:950});
      await page.goto(base+'&'+new URLSearchParams({...attributes,page:'/later'}));
      await expect(page,1);
      assert.equal(await page.getByRole('link',{name:'Show bounce rate',exact:true}).getAttribute('data-value'),'0');
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),true);
      await page.screenshot({path:join(output,`visit-attributes-${scheme}.png`),fullPage:true});
      await page.goto(base+'&country=PH&page=%2Flater');
      await expect(page,0);
    }
    assert.deepEqual(errors,[]);
  } finally {await owner.close();}
  const plain=await browser.newContext({storageState,javaScriptEnabled:false});
  try {
    const page=await plain.newPage();
    await page.goto(base+'&'+new URLSearchParams(attributes));
    await expect(page,2);
  } finally {await plain.close();}
}
