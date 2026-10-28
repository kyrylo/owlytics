import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';

export async function checkAcquisition({page, context, browser, origin, database, output}) {
  const day = new Date(Date.now() - 86400000).toISOString().slice(0, 10);
  const campaign = '<Launch & "quote">';
  execFileSync('python3', ['-c', `
import sqlite3,sys,datetime,hashlib
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA foreign_keys=ON')
day=sys.argv[2];stamp=int(datetime.datetime.fromisoformat(day+'T08:00:00+00:00').timestamp())
c.execute("INSERT INTO sites VALUES(4,'campaign.example','UTC',0)")
rows=[(1,'Facebook','Paid Social','FB-Ads','CPC',sys.argv[3]),(1,'Facebook','Paid Social','fb-ads','cpc','B'),(2,'Google','Paid Search','Google','cpc','B'),(3,'','Direct','','',''),(4,'Google',None,None,None,None)]
for i,(person,source,channel,utm_source,medium,campaign) in enumerate(rows):
 visitor=hashlib.sha256(f'campaign-{person}'.encode()).digest()[:16]
 request=hashlib.sha256(f'campaign-request-{i}'.encode()).digest()[:16]
 c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,referrer,country,browser,device) VALUES(4,?,?,?,?,?,?,?,'US','Chrome','Desktop')",(stamp+i*3600,day,visitor,request,'/'+str(i),source,'google.com/search' if source else ''))
 if channel is not None:
  session=c.execute('SELECT session_id FROM events WHERE site_id=4 AND request_id=?',(request,)).fetchone()[0]
  c.execute('INSERT INTO session_acquisition(session_id,channel,utm_source,utm_medium,utm_campaign,utm_content,utm_term) VALUES(?,?,?,?,?,?,?)',(session,channel,utm_source,medium,campaign,'=1+1' if medium else '', 'owl' if medium else ''))
c.commit()
`, database, day, campaign]);
  const base = `${origin}/sites/4?from=${day}&to=${day}&compare=off`;
  const idle = () => page.waitForFunction(() => !document.querySelector('main[aria-busy=true]'));
  const filters = () => JSON.parse(new URL(page.url()).searchParams.get('filters') || '[]');
  const menu = () => page.locator('.campaign-menu');
  const options = [['UTM mediums','utm_medium'], ['UTM sources','utm_source'], ['UTM campaigns','utm_campaign'], ['UTM contents','utm_content'], ['UTM terms','utm_term']];
  await page.goto(`${base}&sources_tab=channel`);
  await page.locator('.panel-channel .data-row').filter({hasText:'Paid Social'}).click();
  assert.equal(await page.locator('.panel-source').count(), 1);
  assert.deepEqual(filters(), [{key:'channel',op:'is',values:['Paid Social']}]);
  await page.locator('.panel-source .data-row').filter({hasText:'Facebook'}).click();
  assert.equal(await page.locator('.panel-referrer').count(), 1);
  assert.match(await page.locator('.panel-referrer').innerText(), /google.com\/search/);
  await page.getByRole('link', {name:'Clear filters',exact:true}).click();
  await idle();
  assert.equal(await page.locator('.panel-source').count(), 1);
  for (const [label, key] of options) {
    await menu().locator('summary').click();
    await menu().getByRole('link', {name:label,exact:true}).click();
    assert.equal(new URL(page.url()).searchParams.get('sources_tab'), key);
    assert.equal(await menu().locator('summary').getAttribute('aria-current'), 'page');
    await page.locator(`.panel-${key}`).getByRole('link', {name:'Expand report',exact:true}).click();
    assert.equal(await page.locator(`.panel-${key}.expanded-report`).count(), 1);
    await page.getByRole('link', {name:'Close expanded report',exact:true}).click();
  }
  await page.goto(`${base}&sources_tab=utm_campaign`);
  const launch = page.locator('.panel-utm_campaign .data-row').filter({hasText:campaign});
  assert.equal(await launch.locator('.row-percentage').innerText(), '50.0%');
  assert.equal(await page.locator('.panel-utm_campaign .data-row').filter({hasText:/^B/}).locator('.row-percentage').innerText(), '100.0%');
  await launch.click();
  assert.deepEqual(filters(), [{key:'utm_campaign',op:'is',values:[campaign]}]);
  assert.equal(await page.locator('.metrics .metric').first().getAttribute('data-value'), '1');
  await page.getByRole('link', {name:'Remove utm_campaign filter',exact:true}).click();
  await idle();
  assert.equal(new URL(page.url()).searchParams.get('sources_tab'), 'utm_campaign');
  assert.equal(await page.locator('.metrics .metric').first().getAttribute('data-value'), '4');
  await page.goto(`${base}&sources_tab=utm_medium&filter=utm_medium`);
  await page.locator('.filter-values input[type=checkbox][value="CPC"]').check();
  await idle();
  await page.locator('.filter-values input[type=checkbox][value="cpc"]').check();
  await page.waitForFunction(() => JSON.parse(new URL(location.href).searchParams.get('filters'))[0].values.length === 2);
  await idle();
  assert.deepEqual(filters()[0].values.sort(), ['CPC','cpc']);
  assert.equal(await page.locator('.metrics .metric').first().getAttribute('data-value'), '2');

  for (const [width, scheme] of [[1440,'light'],[768,'light'],[390,'dark'],[320,'light']]) {
    await page.setViewportSize({width,height:1050});
    await page.emulateMedia({colorScheme:scheme});
    await page.goto(`${base}&sources_tab=utm_campaign`);
    await menu().locator('summary').click();
    assert.equal(await menu().getByRole('link').count(), 5);
    assert.ok(await menu().locator('.dropdown-content').evaluate(el => {const r=el.getBoundingClientRect();return r.left>=0 && r.right<=innerWidth+1;}), `${width}: campaign menu bounds`);
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth+1), `${width}: horizontal overflow`);
    await page.screenshot({path:join(output,`campaigns-${width}-${scheme}.png`),fullPage:true});
    await page.keyboard.press('Escape');
    assert.equal(await menu().getAttribute('open'), null);
  }
  const noScript = await browser.newContext({javaScriptEnabled:false,storageState:await context.storageState()});
  try {
    const p = await noScript.newPage();
    await p.goto(base);
    await p.locator('.campaign-menu>summary').click();
    await p.locator('.campaign-menu').getByRole('link', {name:'UTM campaigns',exact:true}).click();
    await p.locator('.panel-utm_campaign .data-row').filter({hasText:campaign}).click();
    assert.equal(await p.locator('.metrics .metric').first().getAttribute('data-value'), '1');
    assert.equal(await p.getByRole('link', {name:'Remove utm_campaign filter',exact:true}).count(), 1);
  } finally { await noScript.close(); }
  await page.setViewportSize({width:1440,height:1050});
  await page.emulateMedia({colorScheme:'light'});
  await page.goto(`${origin}/sites/1`);
}
