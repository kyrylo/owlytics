import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';
import {selectCalendarDate,selectCalendarRange} from './calendar-helpers.mjs';

export async function checkCalendars({context,browser,origin,database,output}) {
  const today=execFileSync('python3',['-c',`
import datetime,sqlite3,sys,zoneinfo
z=zoneinfo.ZoneInfo('Pacific/Kiritimati');c=sqlite3.connect(sys.argv[1])
c.execute("INSERT INTO sites VALUES(120,'calendar-range.example','Pacific/Kiritimati',0)")
for day in ['2010-01-15','2023-12-31','2024-01-01','2024-02-28','2024-02-29','2024-03-10','2024-03-11']:
 stamp=int(datetime.datetime.fromisoformat(day+'T12:00:00').replace(tzinfo=z).timestamp())
 c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(120,?,?,randomblob(16),randomblob(16),'/calendar','Google','US','Chrome','Desktop')",(stamp,day))
c.commit();print(datetime.datetime.now(z).date())
`,database],{encoding:'utf8'}).trim();
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  const base=`${origin}/sites/120?period=month&compare=previous&compare_match=date&country=US&metric=pageviews&interval=week&with_imported=false`;
  for(const [colorScheme,width,timezoneId] of [['light',1280,'America/Los_Angeles'],['dark',768,'Asia/Tokyo'],['light',390,'Pacific/Honolulu'],['dark',320,'America/New_York']]) {
    const height=width<=390?844:900;
    const owner=await browser.newContext({storageState,colorScheme,timezoneId,viewport:{width,height}});
    try {
      const page=await owner.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));
      await page.clock.install({time:new Date('2035-07-01T12:00:00Z')});
      await page.goto(base);
      const open=async(comparison=false)=>{
        await page.locator(comparison?'.comparison-picker>summary':'.date-picker>summary').click();
        const menuBounds=await page.locator(comparison?'.comparison-picker>.dropdown-content':'.date-picker>.dropdown-content').boundingBox();
        assert.ok(menuBounds.y+menuBounds.height<=height,`Date menu fits ${width}×${height} with comparison controls`);
        await page.getByText(comparison?'Custom period':'Custom range',{exact:true}).click();
        await page.locator(comparison?'.custom-comparison .calendar-days button':'.custom-range:not(.custom-comparison) .calendar-days button').first().waitFor();
      };
      const calendar=page.locator('.date-picker .range-calendar');
      await open();
      assert.equal(await calendar.locator('.calendar-days button').count(),42);
      assert.equal(await calendar.locator('[data-today]').getAttribute('data-date'),today,'Site-local today comes from Rust, not the browser clock or timezone');
      assert.equal(await calendar.locator('[data-today]').isEnabled(),true);
      assert.equal(await calendar.getByRole('button',{name:'Next calendar month'}).isEnabled(),false);
      const currentMonth=await calendar.getByLabel('Month',{exact:true}).inputValue(),currentYear=await calendar.getByLabel('Year',{exact:true}).inputValue();
      await calendar.getByRole('button',{name:'Previous calendar month'}).click();
      assert.equal(Number(await calendar.getByLabel('Year',{exact:true}).inputValue())*12+Number(await calendar.getByLabel('Month',{exact:true}).inputValue()),Number(currentYear)*12+Number(currentMonth)-1);
      await calendar.getByRole('button',{name:'Next calendar month'}).click();
      assert.equal(await calendar.getByLabel('Month',{exact:true}).inputValue(),currentMonth);
      const bounds=await calendar.boundingBox();assert.equal(bounds.width,288);assert.equal(bounds.height,312);
      assert.ok(bounds.x>=0&&bounds.x+bounds.width<=width+1&&bounds.y+bounds.height<=height);
      assert.equal(await page.getByRole('button',{name:'Apply',exact:true}).isVisible(),false);
      await page.screenshot({path:join(output,`calendar-picker-${colorScheme}-${width}.png`)});
      // Reversed selection crosses February's leap day and US DST, while this
      // site is ahead of UTC. Both dates must retain their calendar identity.
      await selectCalendarDate(page,'2024-03-10');
      assert.equal(await calendar.locator('[data-start]').getAttribute('data-date'),'2024-03-10','The first day click survives the year field blur');
      const old=page.url();assert.match(old,/period=month/);
      await selectCalendarDate(page,'2024-02-29');
      await page.waitForURL(url=>url.searchParams.get('from')==='2024-02-29'&&url.searchParams.get('to')==='2024-03-10');
      let params=new URL(page.url()).searchParams;
      for(const [key,value] of Object.entries({country:'US',metric:'pageviews',compare:'previous',compare_match:'date',with_imported:'false'}))assert.equal(params.get(key),value,key);
      assert.equal(params.has('period'),false);assert.equal(params.has('interval'),false,'Custom dates do not inherit the preset interval');
      assert.equal(await page.getByRole('link',{name:'Show pageviews',exact:true}).getAttribute('data-value'),'2');
      await open(true);
      await selectCalendarRange(page,'2023-12-31','2024-01-01',true);
      params=new URL(page.url()).searchParams;
      assert.equal(params.get('compare'),'custom');assert.equal(params.get('from'),'2024-02-29');
      assert.equal(await page.getByRole('link',{name:'Show pageviews',exact:true}).locator('.metric-previous>b').innerText(),'2');
      await open();
      await selectCalendarDate(page,'2024-03-11');
      await page.keyboard.press('Escape');
      await open();
      await calendar.locator('[data-start][data-date="2024-02-29"]').waitFor();
      assert.equal(await calendar.getByLabel('Year',{exact:true}).inputValue(),'2024','Existing custom dates reopen in their month');
      assert.equal(await calendar.locator('[data-start]').getAttribute('data-date'),'2024-02-29','Closing restores the applied range');
      await page.screenshot({path:join(output,`calendar-selected-${colorScheme}-${width}.png`)});
      await selectCalendarRange(page,'2024-02-29','2024-02-29');
      assert.equal(await page.getByRole('link',{name:'Show pageviews',exact:true}).getAttribute('data-value'),'1');
      await open();
      await calendar.getByLabel('Year',{exact:true}).fill('2010');await calendar.getByLabel('Year',{exact:true}).press('Enter');
      await calendar.getByLabel('Month',{exact:true}).selectOption('0');
      assert.equal(await calendar.locator('[data-date="2010-01-14"]').isEnabled(),false);
      assert.equal(await calendar.locator('[data-date="2010-01-15"]').isEnabled(),true);
      assert.equal(await calendar.getByRole('button',{name:'Previous calendar month'}).isEnabled(),false);
      await calendar.locator('[data-date="2010-01-15"]').click();
      await calendar.getByLabel('Year',{exact:true}).fill('2024');await calendar.getByLabel('Year',{exact:true}).press('Enter');
      await calendar.getByLabel('Month',{exact:true}).selectOption('1');
      assert.equal(await calendar.locator('[data-date="2024-02-29"]').isEnabled(),false,'The calendar enforces the server range limit');
      await page.keyboard.press('Escape');
      assert.deepEqual(errors,[]);
    } finally {await owner.close();}
  }
  // Calendar submission must merge a filter that has not reached the server yet.
  const queued=await browser.newContext({storageState});
  let release;
  try {
    const page=await queued.newPage();await page.goto(`${origin}/sites/120?from=2024-02-28&to=2024-03-11&filter=browser`);
    const gate=new Promise(resolve=>{release=resolve;});
    await queued.route('**/sites/120/filter?**',async route=>{await gate;await route.continue();});
    await page.locator('.filter-values input[value="Chrome"]').check();
    await page.locator('.date-picker>summary').click();
    const initialYear=await page.locator('.date-picker .custom-range-toggle>summary').evaluate(summary=>{
      summary.click();return summary.parentElement.querySelector('[aria-label="Year"]').value;
    });
    assert.equal(initialYear,'2024','The calendar is populated before its year field becomes editable');
    await selectCalendarDate(page,'2024-02-29');await selectCalendarDate(page,'2024-03-10');
    release();
    await page.waitForURL(url=>url.searchParams.get('from')==='2024-02-29'&&url.searchParams.get('to')==='2024-03-10');
    assert.deepEqual(JSON.parse(new URL(page.url()).searchParams.get('filters')),[{key:'browser',op:'is',values:['Chrome']}]);
    assert.equal(await page.getByRole('link',{name:'Show pageviews',exact:true}).getAttribute('data-value'),'2');
  } finally {release?.();await queued.close();}
  const owner=await browser.newContext({storageState,javaScriptEnabled:false});
  try {
    const page=await owner.newPage();await page.goto(base);
    await page.locator('.date-picker>summary').click();await page.getByText('Custom range',{exact:true}).click();
    await page.getByLabel('From',{exact:true}).fill('2024-02-29');await page.getByLabel('To',{exact:true}).fill('2024-03-10');
    await page.getByRole('button',{name:'Apply',exact:true}).click();
    assert.equal(await page.getByRole('link',{name:'Show pageviews',exact:true}).getAttribute('data-value'),'2');
  } finally {await owner.close();}
}
