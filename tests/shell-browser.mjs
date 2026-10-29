import assert from 'node:assert/strict';
import {join} from 'node:path';

// Shared chrome must not jump when navigating between the website list and a report.
export async function checkShell({browser,context,origin,output}) {
  const storageState={cookies:(await context.storageState()).cookies,origins:[]};
  for(const [width,scheme,javaScriptEnabled] of [[1440,'light',true],[1440,'dark',true],[1024,'light',true],[768,'dark',true],[390,'dark',true],[320,'light',true],[390,'light',false]]) {
    const owner=await browser.newContext({storageState,viewport:{width,height:900},colorScheme:scheme,javaScriptEnabled});
    try {
      const page=await owner.newPage(),errors=[];
      page.on('pageerror',error=>errors.push(error.message));
      const header=()=>page.locator('.nav').evaluate(nav=>{
        const box=e=>{const b=e.getBoundingClientRect();return {x:b.x,y:b.y,width:b.width,height:b.height};};
        const style=getComputedStyle(nav);
        return {nav:box(nav),logo:box(nav.querySelector('.brand')),actions:box(nav.querySelector('.nav-actions')),background:style.backgroundColor,border:style.borderBottomWidth};
      });
      await page.goto(origin+'/');const indexHeader=await header();
      await page.screenshot({path:join(output,`shell-index-${scheme}-${width}-${javaScriptEnabled}.png`)});
      await page.goto(origin+'/settings');
      assert.deepEqual(await header(),indexHeader,`Settings header consistency at ${width}px`);
      await page.goto(origin+'/sites/1?period=28d&compare=previous');
      assert.deepEqual(await header(),indexHeader,`Index/report header consistency at ${width}px`);
      if(javaScriptEnabled) await page.waitForFunction(()=>document.documentElement.hasAttribute('data-dashboard-enhanced'));
      for(const name of ['site-picker','date-picker','comparison-picker','dashboard-menu','filter-menu-toggle','campaign-menu']) {
        const menu=page.locator(`.${name}`).first();
        await menu.locator(':scope > summary').click();
        const content=menu.locator(':scope > .dropdown-content');
        await content.waitFor({state:'visible'});
        const check=()=>content.evaluate((element,enhanced)=>{
          const trigger=element.parentElement.querySelector(':scope > summary').getBoundingClientRect(),box=element.getBoundingClientRect();
          const expected=Math.max(12,Math.min(trigger.x+trigger.width/2-box.width/2,innerWidth-box.width-12));
          return {offset:Math.abs(box.x-expected),inside:box.x>=11&&box.right<=innerWidth-11&&box.bottom<=innerHeight-10,aligned:box.y>=trigger.bottom+3&&box.y<=trigger.bottom+9,enhanced};
        },javaScriptEnabled);
        if(javaScriptEnabled) await page.waitForFunction(element=>element.hasAttribute('data-positioned'),await content.elementHandle());
        const position=await check();
        assert.ok(position.inside,`${name} stays in viewport at ${width}px: ${JSON.stringify(position)}`);
        if(javaScriptEnabled) assert.ok(position.offset<=1&&position.aligned,`${name} is centered on its trigger, clamped only at the viewport edge: ${JSON.stringify(position)}`);
        if(name==='site-picker') {
          const summary=menu.locator(':scope > summary');
          const alignment=await summary.evaluate(element=>{
            const center=e=>{const b=e.getBoundingClientRect();return b.y+b.height/2;};
            return [center(element.querySelector('.site-favicon')),center(element.querySelector('.site-name')),center(element.querySelector('.chevron'))];
          });
          assert.ok(Math.max(...alignment)-Math.min(...alignment)<=1,'Website icon, label and chevron share one center line');
          assert.equal(await content.locator('.site-option[aria-current=page]').count(),1,'Current website is marked in the selector');
          await page.screenshot({path:join(output,`shell-selector-${scheme}-${width}-${javaScriptEnabled}.png`)});
          if(javaScriptEnabled&&width===1440) {
            await page.setViewportSize({width:320,height:700});
            await page.waitForFunction(element=>element.getBoundingClientRect().right<=innerWidth-11,await content.elementHandle());
            assert.ok((await check()).inside,'An open selector repositions after resize');
            await page.setViewportSize({width,height:900});
            await page.mouse.wheel(0,300);
            await page.waitForFunction(element=>{
              const trigger=element.parentElement.querySelector('summary').getBoundingClientRect(),menu=element.getBoundingClientRect();
              return Math.abs(menu.y-trigger.bottom-6)<=1;
            },await content.elementHandle());
            assert.ok((await check()).inside,'An open selector follows the sticky toolbar');
          }
        }
        await menu.locator(':scope > summary').click();
        if(javaScriptEnabled&&width===1440) {
          await page.setViewportSize({width:768,height:900});
          const reopened=await menu.evaluate(async element=>{
            element.querySelector(':scope > summary').click();
            await Promise.resolve();
            const box=element.querySelector(':scope > .dropdown-content').getBoundingClientRect();
            return {left:box.left,right:box.right};
          });
          assert.ok(reopened.left>=11&&reopened.right<=757,`${name} must not reopen with stale desktop coordinates: ${JSON.stringify(reopened)}`);
          await menu.locator(':scope > summary').click();
          await page.setViewportSize({width,height:900});
        }
      }
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
      assert.deepEqual(errors,[]);
    } finally {await owner.close();}
  }
}
