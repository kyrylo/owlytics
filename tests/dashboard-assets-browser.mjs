#!/usr/bin/env node
// Focused browser regression checks for cacheable maps and deferred menu icons.
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {spawn,execFileSync} from 'node:child_process';
import {mkdtempSync,rmSync} from 'node:fs';
import {createServer} from 'node:net';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
const {chromium}=createRequire(import.meta.url)('playwright');
const directory=mkdtempSync(join(tmpdir(),'owlytics-assets-'));
const binary=resolve(process.env.OWLYTICS_BINARY||'target/debug/owlytics');
let child,browser;
try {
  const socket=createServer();
  await new Promise(done=>socket.listen(0,'127.0.0.1',done));
  const port=socket.address().port;await new Promise(done=>socket.close(done));
  const origin=`http://127.0.0.1:${port}`;
  const hash=execFileSync(binary,['hash-password'],{input:'temporary asset test password',encoding:'utf8'}).trim();
  child=spawn(binary,[],{env:{...process.env,OWLYTICS_BIND:`127.0.0.1:${port}`,OWLYTICS_URL:origin,OWLYTICS_DATABASE:join(directory,'test.sqlite'),OWLYTICS_PASSWORD_HASH:hash},stdio:['ignore','pipe','pipe']});
  await new Promise((done,fail)=>{
    const timer=setTimeout(()=>fail(new Error('Startup timeout')),15000);
    child.stdout.on('data',data=>{if(String(data).includes('Owlytics listening')){clearTimeout(timer);done();}});
    child.once('error',fail);
  });
  browser=await chromium.launch({headless:true,...(process.env.OWLYTICS_BROWSER?{executablePath:process.env.OWLYTICS_BROWSER}:{})});
  const version=(await fetch(origin+'/assets/country-shapes.svg')).headers.get('etag').slice(1,17);
  const sprite=`/assets/country-shapes.${version}.svg`;
  const context=await browser.newContext(),page=await context.newPage();
  const requested=[],errors=[];page.on('pageerror',e=>errors.push(e.message));
  await context.route('**/sites/*/favicon',async route=>{
    requested.push(new URL(route.request().url()).pathname);
    await route.fulfill({contentType:'image/svg+xml',body:'<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><rect width="20" height="20" fill="purple"/></svg>'});
  });
  await context.route('**/__assets',route=>route.fulfill({contentType:'text/html',body:`<!doctype html><html><head><link rel="stylesheet" href="/assets/app.css"><script defer src="/assets/favicons.js"></script></head><body>
    <details id="sites"><summary>Sites <span class="site-favicon"><img hidden data-favicon="/sites/1/favicon"></span></summary>
    <span class="site-favicon"><img hidden data-favicon="/sites/2/favicon"></span></details>
    <figure class="country-map"><svg viewBox="0 0 720 300" style="width:720px"><a href="#country" data-country="US"><title>United States</title><use class="map-country map-level-5" href="${sprite}#US"/></a><use class="map-country map-level-0" href="${sprite}#FR"/></svg></figure>
    </body></html>`}));
  await page.goto(origin+'/__assets');
  await page.waitForFunction(()=>document.querySelector('img').naturalWidth>0);
  assert.deepEqual(requested,['/sites/1/favicon'],'Closed menus must not load hidden icons');
  const map=page.locator('use').first();
  await page.waitForFunction(()=>document.querySelector('use').getBBox().width>0);
  assert.ok(await map.evaluate(e=>e.getBBox().width>50),'External country geometry renders');
  for(const scheme of ['light','dark']) {
    await page.emulateMedia({colorScheme:scheme});
    assert.notEqual(await map.evaluate(e=>getComputedStyle(e).fill),await page.locator('use').last().evaluate(e=>getComputedStyle(e).fill),'Visitor colors survive external SVG references');
  }
  await page.locator('summary').click();
  await page.waitForFunction(()=>[...document.images].every(image=>!image.hidden&&image.naturalWidth>0));
  assert.deepEqual(requested,['/sites/1/favicon','/sites/2/favicon']);
  await page.locator('summary').click();await page.locator('summary').click();
  assert.equal(requested.length,2,'Reopening a menu does not enqueue its icons again');
  const href=await page.locator('[data-country=US]').getAttribute('href');assert.equal(href,'#country');
  assert.deepEqual(errors,[]);
  console.log('PASS: external country geometry, light/dark colors, and deferred menu favicons');
} finally {
  if(browser)await browser.close();
  if(child){child.kill();await new Promise(done=>child.once('exit',done));}
  rmSync(directory,{recursive:true,force:true});
}
