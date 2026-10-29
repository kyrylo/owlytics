import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createServer} from 'node:http';

// Browser DOM focus handling with a controlled clock and focus state. Headless
// Chrome keeps tabs focused even after bringToFront; collection uses real fetch.
export async function checkEngagementFocus({browser,origin,database}) {
  execFileSync('python3',['-c',"import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.execute(\"INSERT INTO sites VALUES(101,'127.0.0.1','UTC',0)\");c.commit()",database]);
  const context=await browser.newContext({userAgent:'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36'});
  const website=createServer((request,response)=>{
    response.writeHead(200,{'Content-Type':'text/html'});
    response.end(request.url==='/' ? `<!doctype html><title>Focus fixture</title><body style="height:3000px"><script defer src="${origin}/script.js" data-domain="127.0.0.1" data-local></script>` : '<!doctype html><title>Finished</title>Finished');
  });
  await new Promise((resolve,reject)=>{website.once('error',reject);website.listen(0,'127.0.0.1',resolve);});
  try {
    await context.addInitScript(()=>Object.defineProperty(navigator,'webdriver',{get:()=>false}));
    const page=await context.newPage(),updates=[],errors=[];
    await page.addInitScript(()=>{
      window.fixtureFocused=true;
      document.hasFocus=()=>window.fixtureFocused;
    });
    const focus=focused=>page.evaluate(value=>{
      window.fixtureFocused=value;
      window.dispatchEvent(new Event(value?'focus':'blur'));
    },focused);
    page.on('pageerror',error=>errors.push(error.message));
    page.on('request',request=>{if(request.url()===origin+'/api/engagement')updates.push(request.postDataJSON());});
    await page.clock.install({time:new Date('2026-10-28T11:00:00Z')});
    await page.clock.pauseAt(new Date('2026-10-28T12:00:00Z'));
    const view=page.waitForResponse(response=>response.url()===origin+'/api/event' && response.request().method()==='POST');
    await page.goto(`http://127.0.0.1:${website.address().port}/`);assert.equal((await view).status(),204);
    await page.waitForFunction(()=>document.hasFocus());
    await page.clock.fastForward(1000);
    let update=page.waitForRequest(request=>request.url()===origin+'/api/engagement');
    await focus(false);await update;
    assert.equal(updates[0].time_ms,1000);
    await page.clock.fastForward(9000);await focus(true);
    await page.waitForFunction(()=>document.hasFocus());await page.clock.fastForward(1000);
    await focus(false);await page.waitForTimeout(50);
    assert.equal(updates.length,1,'A sub-three-second revisit remains buffered');
    await page.clock.fastForward(9000);await focus(true);
    await page.waitForFunction(()=>document.hasFocus());await page.clock.fastForward(2000);
    update=page.waitForRequest(request=>request.url()===origin+'/api/engagement');
    await focus(false);await update;
    assert.deepEqual(updates.map(event=>event.time_ms),[1000,4000]);
    assert.equal(new Set(updates.map(event=>event.id)).size,1);
    await focus(true);await page.clock.fastForward(1000);await focus(false);
    assert.equal(updates.length,2,'The final short revisit is buffered until departure');
    // Let server wall time cover the simulated five seconds of foreground time.
    // Chrome may stop reporting old-renderer requests after navigation, so prove
    // delivery in SQLite instead of relying on Playwright's request event.
    await new Promise(resolve=>setTimeout(resolve,5000));
    await page.goto(`http://127.0.0.1:${website.address().port}/finished`);
    const stored=()=>JSON.parse(execFileSync('python3',['-c',`
import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1])
print(json.dumps(c.execute('SELECT COUNT(*),MAX(g.time_ms) FROM events e LEFT JOIN page_engagement g ON g.event_id=e.id WHERE e.site_id=101').fetchone()))
`,database],{encoding:'utf8'}));
    const deadline=Date.now()+5000;
    let actual=stored();
    while(actual[1]!==5000 && Date.now()<deadline) {
      await new Promise(resolve=>setTimeout(resolve,100));actual=stored();
    }
    assert.deepEqual(actual,[1,5000],'Final navigation delivers buffered foreground time without adding a pageview');
    assert.deepEqual(errors,[]);
  } finally {await context.close();await new Promise(resolve=>website.close(resolve));}
}
