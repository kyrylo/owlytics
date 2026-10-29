#!/usr/bin/env node
// Real-browser checks with an isolated database, owner, visitor and local website.
// Install Playwright separately; it is never part of the application runtime.
import assert from 'node:assert/strict';
import {selectCalendarRange} from '../tests/calendar-helpers.mjs';
import {spawn, execFileSync} from 'node:child_process';
import {mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync} from 'node:fs';
import {createServer} from 'node:http';
import {createServer as socketServer} from 'node:net';
import {tmpdir} from 'node:os';
import {resolve, join, extname, sep} from 'node:path';
import {createRequire} from 'node:module';
import {randomBytes} from 'node:crypto';

const {chromium} = createRequire(import.meta.url)('playwright');
const binary = resolve(process.env.OWLYTICS_BINARY || 'target/release/owlytics');
const output = resolve(process.env.OWLYTICS_BROWSER_OUTPUT || 'tmp/browser-checks');
const fixture = mkdtempSync(join(tmpdir(), 'owlytics-browser-'));
const database = join(fixture, 'test.sqlite');
const password = 'disposable browser passphrase';
const failures = [];
const checks = [];
let child, browser, website;
mkdirSync(output, {recursive: true});
rmSync(join(output, 'results.json'), {force: true});

const listen = server => new Promise((done, fail) => {
  server.once('error', fail);
  server.listen(0, '127.0.0.1', () => done(server.address().port));
});
const close = server => new Promise(done => server.close(done));
const date = offset => new Date(Date.now() + offset * 86400000).toISOString().slice(0, 10);
const compact = value => value.replaceAll('-', '');
function observe(page) {
  page.on('pageerror', error => failures.push(error.message));
  page.on('console', message => {
    if (message.type() === 'error' && /Content Security Policy/.test(message.text())) failures.push(message.text());
  });
}
async function screenshot(page, name) {
  await page.screenshot({path: join(output, `${name}.png`), fullPage: true, animations: 'disabled'});
  const sizes = await page.evaluate(() => ({viewport: innerWidth, content: document.documentElement.scrollWidth}));
  assert.ok(sizes.content <= sizes.viewport + 1, `${name}: horizontal overflow ${JSON.stringify(sizes)}`);
  const broken = await page.locator('img').evaluateAll(images => images.filter(image => !image.hidden && (!image.complete || image.naturalWidth === 0)).map(image => image.src));
  assert.deepEqual(broken, [], `${name}: broken images`);
  const overflowingTargets = await page.locator('.chart-hit').evaluateAll(targets => targets.filter(target => {
    const bounds=target.getBoundingClientRect(), plot=target.ownerSVGElement.getBoundingClientRect();
    return bounds.left < plot.left-1 || bounds.right > plot.right+1;
  }).length);
  assert.equal(overflowingTargets,0,`${name}: chart hover targets extend outside the plot`);
}
async function metric(page, name) {
  return Number(await page.locator('.metrics > div').filter({hasText: name}).locator('.metric').getAttribute('data-value'));
}
async function signIn(page, origin) {
  await page.goto(`${origin}/login`);
  await page.getByLabel('Password', {exact: true}).fill(password);
  await page.getByRole('button', {name: 'Sign in', exact: true}).click();
  await page.waitForURL(`${origin}/`);
}
async function assertTheme(page, scheme) {
  await page.emulateMedia({colorScheme: scheme});
  await page.waitForFunction(expected => {
    const body = getComputedStyle(document.body).backgroundColor;
    return (body === 'rgba(0, 0, 0, 0)' ? getComputedStyle(document.documentElement).backgroundColor : body) === expected;
  },
    scheme === 'dark' ? 'rgb(17, 19, 29)' : 'rgb(247, 248, 252)');
  assert.equal(await page.locator('.theme-menu, [data-theme]').count(), 0);
}
async function addSite(page, origin, domain) {
  await page.goto(origin);
  await page.getByRole('link', {name: 'Add website', exact: true}).click();
  await page.getByLabel('Domain', {exact: true}).fill(domain);
  await page.getByLabel('Timezone', {exact: true}).fill('UTC');
  await page.getByRole('button', {name: 'Add website'}).click();
  await page.waitForURL(/\/sites\/\d+\/settings$/);
  assert.ok((await page.locator('pre code').innerText()).includes(`data-domain="${domain}"`));
  return page.url().match(/\/sites\/(\d+)/)[1];
}

try {
  const reservation = socketServer();
  const port = await listen(reservation);
  await close(reservation);
  const origin = `http://127.0.0.1:${port}`;
  const hash = execFileSync(binary, ['hash-password'], {input: password, encoding: 'utf8'}).trim();
  child = spawn(binary, [], {env: {...process.env, OWLYTICS_DATABASE: database, OWLYTICS_PASSWORD_HASH: hash, OWLYTICS_BIND: `127.0.0.1:${port}`, OWLYTICS_URL: origin, OWLYTICS_TRUSTED_PROXIES: ''}, stdio: ['ignore', 'pipe', 'pipe']});
  await new Promise((done, fail) => {
    const timeout = setTimeout(() => fail(new Error('Server startup timed out')), 15000);
    child.stdout.on('data', data => { if (String(data).includes('Owlytics listening')) { clearTimeout(timeout); done(); } });
    child.stderr.on('data', data => process.stderr.write(data));
    child.once('error', fail);
    child.once('exit', code => { clearTimeout(timeout); fail(new Error(`Server exited: ${code}`)); });
  });
  browser = await chromium.launch({headless: true, ...(process.env.OWLYTICS_BROWSER ? {executablePath: process.env.OWLYTICS_BROWSER} : {})});
  const owner = await browser.newContext({viewport: {width: 1440, height: 1050}, colorScheme: 'light'});
  const page = await owner.newPage();
  const network = await owner.newCDPSession(page);
  const requests = new Map(), assetResponses = new Map(), revalidated = new Set();
  await network.send('Network.enable');
  network.on('Network.requestWillBeSent', event => requests.set(event.requestId,event.request.url));
  network.on('Network.responseReceivedExtraInfo', event => {
    assetResponses.set(event.requestId, event.statusCode);
    // Resolve URLs after collecting events, not inside the CDP callback.
    if (event.statusCode === 304) revalidated.add(event.requestId);
  });
  observe(page);
  await page.goto(`${origin}/login`);
  await screenshot(page, 'login-light');
  await assertTheme(page, 'dark');
  await screenshot(page, 'login-dark');
  await assertTheme(page, 'light');
  assert.equal(await page.locator('script').count(), 1);
  assert.deepEqual(await page.evaluate(() => [localStorage.length, sessionStorage.length]), [0,0]);
  checks.push('Automatic light-dark CSS responds to system changes without scripts or storage');
  await page.reload();
  await page.waitForLoadState('networkidle');
  const cssRequests = [...requests].filter(([, url]) => url === `${origin}/assets/app.css`);
  assert.ok(cssRequests.some(([id]) => revalidated.has(id)),
    `Browser should revalidate cached CSS; observed statuses: ${cssRequests.map(([id]) => assetResponses.get(id)).join(', ')}`);
  await network.detach();
  checks.push('Public assets revalidate with HTTP 304 while page responses remain uncached');
  await page.getByLabel('Password', {exact: true}).fill('deliberately incorrect');
  await page.getByRole('button', {name: 'Sign in', exact: true}).click();
  await page.locator('.error').waitFor();
  await signIn(page, origin);
  assert.ok(await page.getByRole('heading', {name: 'No websites yet'}).isVisible());
  const site = await addSite(page, origin, 'example.com');
  await screenshot(page, 'install-light');
  await page.getByRole('link', {name: 'Open dashboard'}).click();
  assert.equal(await metric(page, 'Pageviews'), 0);
  assert.ok(await page.getByRole('heading', {name:'Waiting for your first pageview'}).isVisible());
  assert.ok(await page.getByRole('link', {name:'Install tracker', exact:true}).isVisible());
  assert.equal(await page.locator('.report-panel').count(), 4);
  await screenshot(page, 'empty-dashboard');
  checks.push('Owner login, rejected password, empty state, website creation, install snippet');

  const from = date(-14), to = date(-1);
  const imported = ['date,visitors,pageviews'];
  const history = [38, 52, 45, 68, 64, 80, 73, 105, 94, 112, 125, 118, 141, 153];
  history.forEach((visitors, index) => imported.push(`${date(index-59)},${Math.round(visitors/2)},${visitors}`));
  history.forEach((visitors, index) => imported.push(`${date(index-14)},${visitors},${visitors*2}`));
  const directory = join(fixture, 'export');
  mkdirSync(directory);
  writeFileSync(join(directory, `imported_visitors_${compact(date(-59))}_${compact(to)}.csv`), `${imported.join('\n')}\n`);
  execFileSync(binary, ['import-plausible', 'example.com', directory], {env: {...process.env, OWLYTICS_DATABASE: database}, encoding: 'utf8'});
  const paths = ['/', '/pricing', '/docs', '/about', '/changelog', '/blog', '/features', '/privacy', '/contact', '/download', '/guides', '/terms'];
  let nativeViews = 0;
  for (let index = 0; index < paths.length; index++) {
    for (let view = 0; view <= index; view++) {
      const response = await fetch(`${origin}/api/event`, {method: 'POST', headers: {Origin: 'https://example.com', 'Content-Type': 'text/plain', 'User-Agent': `Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36 OwlyticsFixture/${view}`}, body: JSON.stringify({domain: 'example.com', path: paths[index], referrer: view % 2 ? 'https://github.com/kyrylo' : '', id: randomBytes(16).toString('hex')})});
      assert.equal(response.status, 204);
      nativeViews++;
    }
  }
  await page.reload();
  assert.equal(await metric(page, 'Pageviews'), history.reduce((a,b) => a + b*2, 0));
  const studio = await addSite(page, origin, 'studio.example');
  const studioDir = join(fixture, 'studio-export');
  mkdirSync(studioDir);
  const studioHistory = ['date,visitors,pageviews'];
  for (let day=-59; day<0; day++) {
    const visitors = day < -29 ? 110 : Math.round(85 + Math.sin(day*1.3)*20 + day/2);
    studioHistory.push(`${date(day)},${visitors},${visitors*3}`);
  }
  writeFileSync(join(studioDir, `imported_visitors_${compact(date(-59))}_${compact(to)}.csv`), `${studioHistory.join('\n')}\n`);
  execFileSync(binary, ['import-plausible', 'studio.example', studioDir], {env:{...process.env, OWLYTICS_DATABASE: database}, encoding:'utf8'});
  await addSite(page, origin, 'quiet.example');
  await page.goto(origin);
  assert.equal(await page.locator('.site-card').count(), 3);
  assert.equal(await page.locator('.site-card .site-sparkline').count(), 3);
  const exampleCard = page.locator('.site-card').filter({has:page.getByRole('heading',{name:'example.com',exact:true})});
  assert.equal(await exampleCard.locator('.site-visitors strong').getAttribute('title'), '12 visitors');
  assert.equal(await page.locator('.site-card').filter({hasText:'studio.example'}).locator('.site-visitors strong').innerText(), '0');
  assert.equal(await page.getByText('visitors in last 24h', {exact:true}).count(), 3);
  for (const [name,width,height,scheme] of [
    ['projects-desktop-light',1440,1000,'light'], ['projects-desktop-dark',1440,1000,'dark'],
    ['projects-mobile-light',390,844,'light'], ['projects-mobile-dark',390,844,'dark'],
    ['projects-small-mobile',320,740,'light'], ['projects-tablet',768,1024,'light'],
  ]) {
    await page.setViewportSize({width,height});
    await page.emulateMedia({colorScheme:scheme});
    await assertTheme(page, scheme);
    await screenshot(page,name);
  }
  await exampleCard.getByRole('link', {name:'Settings for example.com'}).click();
  assert.ok(page.url().endsWith(`/sites/${site}/settings`));
  await page.goto(origin);
  await exampleCard.locator('.site-card-main').click();
  assert.equal(await metric(page,'Pageviews'), nativeViews);
  checks.push('Rolling 24-hour project counts excluding daily imports, card/report/settings navigation, responsive grid');
  await page.goto(`${origin}/sites/${site}?period=28d`);
  for (const [name, width, height, scheme] of [
    ['dashboard-desktop-light', 1440, 1050, 'light'],
    ['dashboard-desktop-dark', 1440, 1050, 'dark'],
    ['dashboard-mobile-light', 390, 844, 'light'],
    ['dashboard-mobile-dark', 390, 844, 'dark'],
    ['dashboard-small-mobile', 320, 740, 'light'],
    ['dashboard-tablet', 768, 1024, 'light'],
  ]) {
    await page.setViewportSize({width, height});
    await page.emulateMedia({colorScheme: scheme});
    const chartSize = await page.locator('.chart-line').evaluate(line => line.getBoundingClientRect().height);
    const labelSize = await page.locator('.chart-labels').evaluate(labels => parseFloat(getComputedStyle(labels).fontSize));
    assert.ok(chartSize >= 120, `${name}: compressed traffic chart`);
    assert.ok(labelSize >= 10.5, `${name}: unreadable date labels`);
    await screenshot(page, name);
  }
  await page.getByRole('link', {name:'Show pageviews', exact:true}).focus();
  await page.keyboard.press('Enter');
  assert.equal(await page.getByRole('link', {name:'Show pageviews', exact:true}).getAttribute('aria-current'), 'true');
  assert.ok((await page.locator('.chart-hit').last().getAttribute('aria-label')).includes(`${history.at(-1)*2} pageviews`));
  for (const [name,width,height,scheme] of [['pageviews-desktop',1440,1050,'light'],['pageviews-mobile',390,844,'dark']]) {
    await page.setViewportSize({width,height});
    await assertTheme(page,scheme);
    await screenshot(page,name);
  }
  await page.locator('.date-picker > summary').click();
  await page.getByRole('link', {name: 'Today', exact: true}).click();
  assert.equal(new URL(page.url()).searchParams.get('metric'),'pageviews');
  assert.equal(await metric(page, 'Pageviews'), nativeViews);
  await screenshot(page, 'dashboard-single-day');
  await page.getByText('View all', {exact: true}).click();
  const pricing = page.locator('.expanded-report').getByRole('link',{name:'/pricing',exact:true});
  await pricing.click();
  assert.equal(await metric(page, 'Pageviews'), 2);
  assert.equal(new URL(page.url()).searchParams.get('metric'),'pageviews');
  await page.getByRole('link', {name: 'Remove page filter', exact: true}).click();await page.waitForFunction(()=>!document.querySelector('main[aria-busy=true]'));
  assert.equal(await metric(page, 'Pageviews'), nativeViews);
  await page.locator('.date-picker > summary').click();
  await page.getByText('Custom range', {exact:true}).click();
  await selectCalendarRange(page,from,to);
  assert.equal(new URL(page.url()).searchParams.get('metric'),'pageviews');
  assert.equal(await metric(page, 'Pageviews'), history.reduce((a,b) => a + b*2, 0));
  await page.locator('.date-picker > summary').click();
  await page.getByText('Custom range', {exact:true}).click();
  await selectCalendarRange(page,date(-1),date(0));
  assert.equal(await metric(page,'Pageviews'), nativeViews + history.at(-1)*2);
  await screenshot(page,'dashboard-two-days');
  const originalCounts = await page.locator('.metrics strong').allTextContents();
  await page.setViewportSize({width:320,height:740});
  await page.locator('.metrics strong').evaluateAll(values => values.forEach((value,index) => value.textContent=index ? '5,000,000' : '1,100,000'));
  await screenshot(page,'dashboard-large-counts');
  const countBounds = await page.locator('.metrics strong').evaluateAll(values => values.map(value => ({right:value.getBoundingClientRect().right,panel:value.closest('.panel').getBoundingClientRect().right})));
  assert.ok(countBounds.every(value => value.right < value.panel), 'Million-scale counts must fit inside their panel');
  await page.locator('.metrics strong').evaluateAll((values,originals) => values.forEach((value,index) => value.textContent=originals[index]), originalCounts);
  await page.setViewportSize({width:768,height:1024});
  checks.push('Historical import, native collection, chart, More rows, date range, filter and removal');
  await page.goto(`${origin}/sites/${site}?from=${date(0)}&to=${date(0)}&metric=pageviews&page=%2Fmissing&browser=Chrome`);
  assert.ok(await page.getByRole('heading',{name:'No matching pageviews'}).isVisible());
  assert.equal(await page.locator('.report-panel').count(),4);
  await screenshot(page,'empty-filtered-report');
  await page.getByRole('link',{name:'Clear filters',exact:true}).click();await page.waitForFunction(()=>!document.querySelector('main[aria-busy=true]'));
  assert.equal(await metric(page,'Pageviews'),nativeViews);
  assert.equal(new URL(page.url()).searchParams.get('metric'),'pageviews');
  assert.equal(new URL(page.url()).searchParams.get('from'),date(0));
  assert.equal(new URL(page.url()).searchParams.has('browser'),false);
  assert.equal(new URL(page.url()).searchParams.has('page'),false);
  await page.goto(`${origin}/sites/${site}?from=${date(0)}&to=${date(0)}&metric=pageviews&page=%2Fpricing&browser=Chrome`);
  assert.equal(await metric(page,'Pageviews'),2);
  await page.getByRole('link',{name:'Clear filters',exact:true}).click();await page.waitForFunction(()=>!document.querySelector('main[aria-busy=true]'));
  assert.equal(await metric(page,'Pageviews'),nativeViews);
  await page.goto(`${origin}/sites/${site}?from=2020-01-01&to=2020-01-02&metric=pageviews`);
  assert.ok(await page.getByRole('heading',{name:'No pageviews in this period'}).isVisible());
  await page.getByRole('link',{name:'Last 28 days',exact:true}).click();
  assert.equal(new URL(page.url()).searchParams.get('metric'),'pageviews');
  await page.getByRole('link',{name:'Show visitors',exact:true}).click();
  assert.equal(new URL(page.url()).searchParams.get('metric'),'visitors');
  assert.ok((await page.locator('.chart-hit').last().getAttribute('aria-label')).includes(`${history.at(-1)} visitors`));
  const invalidRange = await page.goto(`${origin}/sites/${site}?from=invalid`);
  assert.equal(invalidRange.status(),400);
  assert.ok(await page.getByRole('alert').isVisible());
  await screenshot(page,'invalid-date-range');
  await page.getByRole('link',{name:'Back to dashboard'}).click();
  assert.equal(await metric(page,'Pageviews'),history.reduce((a,b)=>a+b*2,0));
  checks.push('Keyboard metric switching, chart units, rankings, retained dates/filters, clear-all, contextual empty states and recoverable date errors');
  if (process.env.OWLYTICS_BROWSER_DEMO) {
    execFileSync(binary, ['backup', resolve(process.env.OWLYTICS_BROWSER_DEMO)], {env:{...process.env, OWLYTICS_DATABASE:database}, encoding:'utf8'});
  }

  const trackedSite = await addSite(page, origin, '127.0.0.1');
  website = createServer((request, response) => {
    if (request.url?.startsWith('/owlytics') && process.env.OWLYTICS_MARKETING_DIR) {
      const root = resolve(process.env.OWLYTICS_MARKETING_DIR);
      const pathname = new URL(request.url, 'http://localhost').pathname;
      const file = resolve(root, `.${pathname.endsWith('/') ? `${pathname}index.html` : pathname}`);
      if (!file.startsWith(root + sep)) { response.writeHead(403).end(); return; }
      try {
        response.setHeader('Content-Type', ({'.html':'text/html', '.css':'text/css', '.svg':'image/svg+xml', '.js':'text/javascript'})[extname(file)] || 'application/octet-stream');
        response.end(readFileSync(file));
      } catch { response.writeHead(404).end(); }
      return;
    }
    response.setHeader('Content-Type', 'text/html');
    response.end(`<!doctype html><html><head><title>Tracker fixture</title><script defer data-local data-domain="127.0.0.1" src="${origin}/script.js"></script></head><body style="min-height:2400px"><h1>Local website</h1><button onclick="history.pushState({},'', '/second?private=discard');document.dispatchEvent(new Event('turbo:load'))">Next page</button></body></html>`);
  });
  const websitePort = await listen(website);
  const websiteOrigin = `http://127.0.0.1:${websitePort}`;
  const ua = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36';
  const visitor = await browser.newContext({userAgent: ua});
  // The production tracker intentionally skips automation. Simulate a normal
  // visitor in this isolated context to exercise real fetch/CORS/navigation.
  await visitor.addInitScript(() => Object.defineProperty(navigator, 'webdriver', {get: () => false}));
  await visitor.addCookies([{name:'fixture_cookie', value:'must-not-reach-collector', url: websiteOrigin}]);
  const tracked = await visitor.newPage();
  observe(tracked);
  const collected = [], engagements=[];
  tracked.on('request', request => {
    if (request.url() === `${origin}/api/event`) collected.push(request);
    if (request.url() === `${origin}/api/engagement`) engagements.push(request);
  });
  const nextEvent = () => tracked.waitForResponse(response => response.url() === `${origin}/api/event` && response.request().method() === 'POST');
  const nextEngagement = () => tracked.waitForResponse(response => response.url() === `${origin}/api/engagement` && response.request().method() === 'POST');
  let event = nextEvent();
  await tracked.goto(`${websiteOrigin}/first?private=discard&utm_source=FB-Ads&utm_medium=CPC&utm_campaign=Autumn&gclid=secret-click#secret`);
  assert.equal((await event).status(), 204);
  await tracked.evaluate(()=>window.scrollTo(0,1200));
  await tracked.waitForFunction(()=>window.scrollY>=1200);
  await tracked.waitForTimeout(250);
  let engagement=nextEngagement();
  event = nextEvent();
  await tracked.getByRole('button', {name:'Next page'}).click();
  assert.equal((await event).status(), 204);
  assert.equal((await engagement).status(),204);
  engagement=nextEngagement();
  event = nextEvent();
  await tracked.goBack();
  assert.equal((await event).status(), 204);
  assert.equal((await engagement).status(),204);
  assert.equal(collected.length, 3);
  assert.deepEqual(collected.map(request => request.postDataJSON().path), ['/first','/second','/first']);
  assert.deepEqual(collected.map(request => request.postDataJSON().campaign), [
    {source:'FB-Ads',medium:'CPC',name:'Autumn',click_id:'gclid'}, {},
    {source:'FB-Ads',medium:'CPC',name:'Autumn',click_id:'gclid'},
  ]);
  assert.equal(engagements.length,2);
  assert.equal(engagements[0].postDataJSON().id,collected[0].postDataJSON().id);
  assert.ok(engagements[0].postDataJSON().time_ms>=200);
  assert.ok(engagements[0].postDataJSON().scroll_depth>=50);
  for (const request of [...collected,...engagements]) {
    const headers = await request.allHeaders();
    assert.equal(headers.cookie, undefined);
    assert.equal(headers.referer, undefined);
    assert.ok(!request.postData().includes('private'));
    assert.ok(!request.postData().includes('secret-click'));
  }
  assert.deepEqual(await tracked.evaluate(() => [localStorage.length, sessionStorage.length]), [0,0]);
  assert.equal((await visitor.cookies()).length, 1);
  await tracked.evaluate(() => Object.defineProperty(navigator, 'doNotTrack', {get: () => '1'}));
  await tracked.getByRole('button', {name:'Next page'}).click();
  await tracked.waitForLoadState('networkidle');
  assert.equal(collected.length, 3, 'A privacy preference enabled after loading must stop collection');
  assert.equal(engagements.length,2,'New privacy preferences must stop engagement as well');
  await page.goto(`${origin}/sites/${trackedSite}`);
  assert.equal(await metric(page, 'Pageviews'), 3);
  assert.equal(await metric(page, 'Visitors'), 1);
  await page.goto(`${origin}/sites/${trackedSite}?source=Facebook`);
  assert.equal(await metric(page, 'Pageviews'), 3, 'Campaign source stays attached throughout the visit');
  await page.goto(`${origin}/sites/${trackedSite}?page=%2Ffirst&metric=time_on_page`);
  assert.ok(await metric(page,'Time on page')>=0.2);
  assert.ok(await metric(page,'Scroll depth')>=50);
  assert.equal(await metric(page,'Pageviews'),2);
  checks.push('Real scroll and foreground engagement requests use CORS without cookies, keep pageview counts unchanged and appear in page-filter metrics');
  const privateVisitor = await browser.newContext({userAgent: ua});
  await privateVisitor.addInitScript(() => {
    Object.defineProperty(navigator, 'webdriver', {get: () => false});
    Object.defineProperty(navigator, 'doNotTrack', {get: () => '1'});
  });
  const privatePage = await privateVisitor.newPage();
  let privateRequests = 0;
  privatePage.on('request', request => { if (request.url().endsWith('/api/event')) privateRequests++; });
  await privatePage.goto(`${websiteOrigin}/private`);
  await privatePage.waitForLoadState('networkidle');
  assert.equal(privateRequests, 0);
  checks.push('Real browser tracker, allowlisted campaign tags and first-touch source, click-ID value omission, cross-origin collection, SPA/back navigation, cookie omission, no storage, DNT including preference changes after loading');

  if (process.env.OWLYTICS_MARKETING_DIR) {
    const marketing = await browser.newContext();
    // Keep preview checks local; block external analytics requests.
    await marketing.route('**/*', route => new URL(route.request().url()).origin === websiteOrigin ? route.continue() : route.abort());
    const marketingPage = await marketing.newPage();
    observe(marketingPage);
    await marketingPage.goto(`${websiteOrigin}/owlytics/`);
    for (const [name,width,height,scheme] of [['marketing-desktop-light',1440,1000,'light'],['marketing-desktop-dark',1440,1000,'dark'],['marketing-mobile-light',390,844,'light'],['marketing-mobile-dark',390,844,'dark']]) {
      await marketingPage.setViewportSize({width,height});
      await marketingPage.emulateMedia({colorScheme:scheme});
      await assertTheme(marketingPage, scheme);
      await screenshot(marketingPage,name);
    }
    checks.push('Marketing page: desktop/mobile, light/dark, automatic themes, matching assets, project preview, no overflow');
  }
  const noScripts = await browser.newContext({javaScriptEnabled:false,colorScheme:'dark'});
  await noScripts.addCookies(await owner.cookies());
  const fallback = await noScripts.newPage();
  await fallback.goto(origin);
  assert.equal(await fallback.locator('.site-card .site-sparkline').count(),4);
  assert.equal(await fallback.locator('.theme-menu').count(),0);
  assert.equal(await fallback.locator('body').evaluate(body => getComputedStyle(body).backgroundColor),'rgb(17, 19, 29)');
  await fallback.goto(`${origin}/sites/${site}`);
  await fallback.getByRole('link',{name:'Show pageviews',exact:true}).click();
  assert.ok((await fallback.locator('.chart').getAttribute('aria-label')).startsWith('Pageviews'));
  checks.push('Server-rendered project charts and automatic dark mode work without JavaScript');
  const noStorage = await browser.newContext({colorScheme:'dark'});
  await noStorage.addInitScript(() => Object.defineProperty(window,'localStorage',{get:()=>{throw new Error('Storage disabled');}}));
  const restricted = await noStorage.newPage();
  observe(restricted);
  await restricted.goto(`${origin}/login`);
  await assertTheme(restricted, 'dark');
  await assertTheme(restricted, 'light');
  checks.push('Automatic themes work when storage is blocked');
  await page.getByRole('button', {name: 'Sign out', exact: true}).click();
  await page.waitForURL(`${origin}/login`);
  await page.goto(`${origin}/sites/${site}`);
  await page.waitForURL(`${origin}/login`);
  checks.push('Sign-out revokes access');
  assert.deepEqual(failures, [], 'Browser runtime or CSP errors');
  writeFileSync(join(output, 'results.json'), JSON.stringify({passed: true, browser: await browser.version(), checks}, null, 2));
  console.log(`Browser checks pass (${await browser.version()}). Screenshots: ${output}`);
  checks.forEach(check => console.log(`- ${check}`));
} catch (error) {
  writeFileSync(join(output, 'results.json'), JSON.stringify({passed: false, checks, error: error.stack}, null, 2));
  throw error;
} finally {
  if (browser) await browser.close();
  if (website) await close(website);
  if (child && child.exitCode === null) {
    child.kill('SIGTERM');
    await new Promise(done => {
      const timeout = setTimeout(() => { child.kill('SIGKILL'); done(); }, 5000);
      child.once('exit', () => { clearTimeout(timeout); done(); });
    });
  }
  rmSync(fixture, {recursive: true, force: true});
}
