import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';

// Browser DOM focus handling with a controlled clock and focus state. Headless
// Chrome keeps tabs focused even after bringToFront; collection uses real fetch.
export async function checkEngagementFocus({browser,origin,database}) {
  execFileSync('python3',['-c',"import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.execute(\"INSERT INTO sites VALUES(101,'127.0.0.1','UTC',0)\");c.commit()",database]);
  const context=await browser.newContext();
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
    await page.route(origin+'/focus-fixture',route=>route.fulfill({contentType:'text/html',body:`<!doctype html><title>Focus fixture</title><body style="height:3000px"><script defer src="${origin}/script.js" data-domain="127.0.0.1" data-local></script>`}));
    await page.clock.install({time:new Date('2026-10-28T11:00:00Z')});
    await page.clock.pauseAt(new Date('2026-10-28T12:00:00Z'));
    const view=page.waitForResponse(response=>response.url()===origin+'/api/event' && response.request().method()==='POST');
    await page.goto(origin+'/focus-fixture');assert.equal((await view).status(),204);
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
    assert.deepEqual(errors,[]);
  } finally {await context.close();}
}
