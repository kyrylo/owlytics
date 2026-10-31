#!/usr/bin/env node
// Use a real HTTP server: Playwright route interception disables browser caching.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
const {chromium} = createRequire(import.meta.url)('playwright');
const script = readFileSync(process.env.OWLYTICS_FAVICONS_SCRIPT || new URL('../assets/favicons.js', import.meta.url));
const counts = new Map();
const pending = [];
let releaseSlow = false;
const icon = (response, name) => {
  if (name === 'missing') response.writeHead(404, {'Cache-Control':'private, max-age=3600'}).end();
  else response.writeHead(200, {'Content-Type':'image/svg+xml', 'Cache-Control':'private, max-age=604800'})
    .end('<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><rect width="20" height="20" fill="purple"/></svg>');
};
const img = name => `<span class="site-favicon"><span class="report-icon">S</span><img id="${name}" hidden data-favicon="/icons/${name}"></span>`;
const server = createServer((request, response) => {
  const url = new URL(request.url, 'http://localhost');
  if (url.pathname === '/favicons.js') return response.writeHead(200, {'Content-Type':'text/javascript'}).end(script);
  if (url.pathname.startsWith('/icons/')) {
    const name = url.pathname.slice(7);
    counts.set(name, (counts.get(name) || 0) + 1);
    if (name === 'slow' && !releaseSlow) { pending.push(response); return; }
    if (name === 'busy' && counts.get(name) === 1) return response.writeHead(503, {'Cache-Control':'no-store','Retry-After':'2'}).end();
    return icon(response, name);
  }
  response.writeHead(200, {'Content-Type':'text/html', 'Cache-Control':'no-store',
    'Content-Security-Policy':"default-src 'self'; script-src 'self'; img-src 'self'"})
    .end(`<!doctype html><script defer src="/favicons.js"></script><body>${img('slow')}${img('missing')}${img('fast')}${img('busy')}<details><summary>Websites</summary>${img('menu')}</details><a href="/next">Next page</a></body>`);
});
let browser;
try {
  await new Promise(done => server.listen(0, '127.0.0.1', done));
  const origin = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({headless:true, ...(process.env.OWLYTICS_BROWSER ? {executablePath:process.env.OWLYTICS_BROWSER} : {})});
  const context = await browser.newContext();
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(origin, {waitUntil:'domcontentloaded'});
  await page.waitForFunction(() => !document.querySelector('#fast').hidden, null, {timeout:1500});
  assert.equal(counts.get('slow'), 1, 'Slow discovery is still in progress');
  assert.equal(counts.get('fast'), 1, 'fetch and img share the HTTP cache');
  assert.equal(counts.get('menu'), undefined, 'Closed menus do not load icons');
  await page.evaluate(() => {
    const duplicate = document.querySelector('#fast').cloneNode();
    duplicate.id = 'duplicate'; duplicate.hidden = true; duplicate.removeAttribute('src'); delete duplicate.dataset.queued;
    document.body.append(duplicate);
  });
  await page.waitForFunction(() => !document.querySelector('#duplicate').hidden);
  assert.equal(counts.get('fast'), 1, 'Inserted duplicates reuse the same request');
  await page.waitForFunction(() => !document.querySelector('#busy').hidden, null, {timeout:5000});
  assert.equal(counts.get('busy'), 2, 'Only temporary saturation is retried');
  assert.equal(counts.get('missing'), 1, 'A missing icon is never retried');
  assert.equal(await page.locator('#missing').evaluate(image => image.hidden), true);
  releaseSlow = true;
  pending.splice(0).forEach(response => icon(response, 'slow'));
  await page.waitForFunction(() => !document.querySelector('#slow').hidden);
  await page.locator('summary').click();
  await page.waitForFunction(() => !document.querySelector('#menu').hidden);
  assert.equal(counts.get('menu'), 1);
  const before = Object.fromEntries(counts);
  await page.getByRole('link', {name:'Next page'}).click();
  await page.waitForFunction(() => ['slow','fast','busy'].every(id => !document.getElementById(id).hidden));
  await page.locator('summary').click();
  await page.waitForFunction(() => !document.querySelector('#menu').hidden);
  assert.deepEqual(Object.fromEntries(counts), before, 'Navigation makes no network requests for cached successes or misses');
  assert.deepEqual(errors, []);
  console.log('PASS: independent icons, duplicate coalescing, cache reuse across navigation, cached misses, deferred menus, and busy retries');
} finally {
  pending.splice(0).forEach(response => response.destroy());
  if (browser) await browser.close();
  await new Promise(done => server.close(done));
}
