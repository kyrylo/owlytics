#!/usr/bin/env node
// Disposable image smoke test. Requires Docker and an already-built image.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {randomBytes} from 'node:crypto';
import {mkdtempSync, writeFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';

const image = process.argv[2] || 'owlytics:test';
const name = `owlytics-test-${randomBytes(6).toString('hex')}`;
const volume = `${name}-data`;
const password = 'disposable container passphrase';
const origin = 'http://localhost:3000';
const docker = (...args) => execFileSync('docker', args, {encoding: 'utf8', timeout: 120000}).trim();
let address, exportDirectory;
function locate() {
  const port = JSON.parse(docker('inspect', '--format', '{{json .NetworkSettings.Ports}}', name))['3000/tcp'][0].HostPort;
  address = `http://127.0.0.1:${port}`;
}

async function healthy() {
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${address}/up`, {signal: AbortSignal.timeout(1000)});
      if (response.status === 200 && await response.text() === 'ok') return;
    } catch {}
    await new Promise(done => setTimeout(done, 200));
  }
  throw new Error('Container did not become healthy');
}
const csrf = html => {
  const match = /name="csrf" value="([^"]+)"/.exec(html);
  assert.ok(match, 'Expected a CSRF token');
  return match[1];
};
const form = (path, cookie, values) => fetch(`${address}${path}`, {
  method: 'POST', redirect: 'manual',
  headers: {Origin: origin, Cookie: cookie, 'Content-Type': 'application/x-www-form-urlencoded'},
  body: new URLSearchParams(values),
});
function start(hash) {
  docker('run', '--detach', '--name', name, '--read-only', '--cap-drop=ALL',
    '--tmpfs', '/tmp:rw,noexec,nosuid,size=32m',
    '--publish', '127.0.0.1::3000', '--volume', `${volume}:/data`,
    ...(hash === undefined ? [] : ['--env', `OWLYTICS_PASSWORD_HASH=${hash}`]),
    '--env', `OWLYTICS_URL=${origin}`, image);
  locate();
}

async function attemptPassword(password) {
  const page = await fetch(`${address}/login`);
  assert.equal(page.status, 200);
  const cookie = page.headers.getSetCookie()[0].split(';')[0];
  return form('/login', cookie, {csrf: csrf(await page.text()), password});
}

docker('image', 'inspect', image);
const hash = execFileSync('docker', ['run', '--rm', '-i', image, 'hash-password'], {
  input: password, encoding: 'utf8', timeout: 20000,
}).trim();
docker('volume', 'create', volume);
try {
  start(hash);
  await healthy();
  assert.equal(docker('exec', name, 'id', '-u'), '10001');
  assert.equal(docker('exec', name, 'printenv', 'SQLITE_TMPDIR'), '/data');
  assert.equal(docker('exec', name, 'stat', '-c', '%a', '/data/owlytics.sqlite'), '600');
  assert.match(docker('exec', name, 'cat', '/usr/share/doc/owlytics/LICENSE'), /MIT License/);
  assert.match(docker('exec', name, 'cat', '/usr/share/doc/owlytics/licenses/Unicode-3.0.txt'), /UNICODE LICENSE V3/);
  const geographyNotice = docker('exec', name, 'cat', '/usr/share/doc/owlytics/licenses/geography.md');
  assert.match(geographyNotice, /GeoNames contributors/);
  assert.match(geographyNotice, /assets\/city-keys\.tsv/);
  assert.match(geographyNotice, /assets\/region-keys\.tsv/);
  assert.match(docker('exec', name, 'cat', '/usr/share/doc/owlytics/licenses/Simple-Icons-CC0.txt'), /CC0 1.0 Universal/);
  assert.match(docker('exec', name, 'cat', '/usr/share/doc/owlytics/licenses/icons.md'), /90646e4bf7f3b89df6c37e65baf3e7bc759a517c/);

  const login = await fetch(`${address}/login`);
  assert.equal(login.status, 200);
  const cookie = login.headers.getSetCookie()[0].split(';')[0];
  const signedIn = await form('/login', cookie, {csrf: csrf(await login.text()), password});
  assert.equal(signedIn.status, 303);
  const session = signedIn.headers.getSetCookie().find(value => value.startsWith('owlytics_session=')).split(';')[0];
  const home = await fetch(`${address}/`, {headers: {Cookie: session}});
  const added = await form('/sites', session, {csrf: csrf(await home.text()), domain: 'container.example', timezone: 'UTC'});
  assert.equal(added.status, 303);
  assert.equal(added.headers.get('location'), '/sites/1/settings');
  const install = await fetch(`${address}/sites/1/settings`, {headers: {Cookie: session}});
  assert.match(await install.text(), /data-domain=&quot;container.example&quot;/);
  for (const path of ['/script.js', '/assets/app.css', '/assets/dashboard.js', '/assets/owl.svg']) {
    const response = await fetch(`${address}${path}`);
    assert.equal(response.status, 200, path);
    assert.equal(response.headers.get('cache-control'), 'public, no-cache');
    const cached = await fetch(`${address}${path}`, {headers:{'If-None-Match':response.headers.get('etag')}});
    assert.equal(cached.status,304,path);
    assert.equal(await cached.text(),'');
    assert.ok((await response.text()).length > 100, path);
  }

  const payload = JSON.stringify({domain: 'container.example', path: '/hello?private=omitted', referrer: '', id: randomBytes(16).toString('hex'), campaign:{source:'Launch',medium:'email',name:'October'}});
  for (let retry = 0; retry < 2; retry++) {
    const response = await fetch(`${address}/api/event`, {
      method: 'POST', headers: {Origin: 'https://container.example', 'Content-Type': 'text/plain', 'User-Agent': 'Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36'}, body: payload,
    });
    assert.equal(response.status, 204);
  }
  const engagementPayload=JSON.stringify({domain:'container.example',id:JSON.parse(payload).id,time_ms:0,scroll_depth:70});
  for(let retry=0;retry<2;retry++) {
    const response=await fetch(`${address}/api/engagement`,{method:'POST',headers:{Origin:'https://container.example','Content-Type':'text/plain','User-Agent':'Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36'},body:engagementPayload});
    assert.equal(response.status,204);
  }
  async function checkReport() {
    const response = await fetch(`${address}/sites/1?period=day`, {headers: {Cookie: session}});
    assert.equal(response.status, 200);
    const html = await response.text();
    assert.ok(html.includes('<span>Total pageviews</span><strong>1</strong>'));
    assert.ok(html.includes('/hello'));
    assert.ok(!html.includes('private=omitted'));
    const beforeMinute=Date.now();
    const minute=await fetch(`${address}/sites/1?period=day&interval=minute&metric=pageviews&compare=off`,{headers:{Cookie:session}});
    assert.equal(minute.status,200);
    const minuteHtml=await minute.text();
    assert.ok(minuteHtml.includes('<span>Total pageviews</span><strong>1</strong>'));
    assert.match(minuteHtml,/<a data-preference="interval" aria-current="true"[^>]+>Minutes/);
    const expectedMinutes=[beforeMinute,Date.now()+1000].map(value=>{const time=new Date(value);return time.getUTCHours()*60+time.getUTCMinutes()+1;});
    assert.ok(expectedMinutes.includes((minuteHtml.match(/class="chart-hit"/g)||[]).length),'Minute buckets include the current UTC minute');
    const campaign=await fetch(`${address}/sites/1?period=day&source=Launch`,{headers:{Cookie:session}});
    assert.equal(campaign.status,200);
    assert.ok((await campaign.text()).includes('<span>Total pageviews</span><strong>1</strong>'));
    const engagement=await fetch(`${address}/sites/1?period=day&page=%2Fhello`,{headers:{Cookie:session}});
    const engagementHtml=await engagement.text();
    assert.equal(engagement.status,200);
    assert.ok(engagementHtml.includes('<span>Scroll depth</span><strong>70%</strong>'));
    assert.ok(engagementHtml.includes('<span>Time on page</span><strong>0s</strong>'));
    for(const [filter,count] of [['','1'],['&source=Missing','0']]) {
      const realtime=await fetch(`${address}/sites/1?period=realtime${filter}`,{headers:{Cookie:session}});
      assert.equal(realtime.status,200);
      const live=await realtime.text();
      assert.ok(live.includes('class="metric realtime-current" data-value="1"'));
      assert.ok(live.includes(`<span>Unique visitors (last 30 min)</span><strong>${count}</strong>`));
      assert.ok(live.includes(`<span>Total pageviews (last 30 min)</span><strong>${count}</strong>`));
      assert.equal((live.match(/class="chart-hit"/g)||[]).length,30);
    }
  }
  await checkReport();
  const exported = await fetch(`${address}/sites/1/export`, {headers:{Cookie:session}});
  assert.equal(exported.status,200);
  assert.equal(exported.headers.get('content-type'),'application/zip');
  assert.equal(exported.headers.get('cache-control'),'no-store');
  const exportedBytes = Buffer.from(await exported.arrayBuffer());
  assert.equal(exportedBytes.subarray(0,4).toString('hex'),'504b0304');
  assert.ok(exportedBytes.includes(Buffer.from('summary.csv')));
  const pageviewReport = await fetch(`${address}/sites/1?metric=pageviews`, {headers:{Cookie:session}});
  assert.equal(pageviewReport.status,200);
  const pageviewHtml=await pageviewReport.text();
  assert.ok(pageviewHtml.includes('aria-label="Pageviews from'));
  assert.ok(pageviewHtml.includes('aria-label="Show visitors"'));
  const projects = await fetch(`${address}/`, {headers:{Cookie:session}});
  assert.equal(projects.status,200);
  const projectHtml = await projects.text();
  assert.ok(projectHtml.includes('site-sparkline'));
  assert.ok(projectHtml.includes('title="1 visitors">1</strong>'));
  const write = (path, input) => execFileSync('docker', ['exec', '-i', name, 'sh', '-c', 'cat > "$1"', 'sh', path], {input, timeout: 20000});
  exportDirectory = mkdtempSync(join(tmpdir(), 'owlytics-container-export-'));
  for (const [table,csv] of [
    ['visitors','date,visitors,pageviews\n2020-01-01,3,8\n'],
    ['pages','date,hostname,page,pageviews\n2020-01-01,container.example,/archive,8\n'],
    ['sources','date,source,referrer,utm_source,utm_medium,utm_campaign,utm_content,utm_term,pageviews\n2020-01-01,Search,search.example,Search,cpc,Launch,,,9\n'],
    ['browsers','date,browser,browser_version,visitors,pageviews\n2020-01-01,Chrome,1,2,5\n2020-01-01,Firefox,1,1,3\n'],
    ['custom_props','outside scope\n'],
  ]) writeFileSync(join(exportDirectory,`imported_${table}_20200101_20200101.csv`),csv,{mode:0o600});
  const archive=execFileSync('tar',['-C',exportDirectory,'-cf','-','.']);
  execFileSync('docker',['exec','-i',name,'sh','-c','mkdir /data/plausible-import && tar -xf - -C /data/plausible-import'],{input:archive,timeout:20000});
  const preview=docker('exec',name,'owlytics','import-plausible','container.example','/data/plausible-import','--dry-run');
  assert.match(preview,/Would import 3 visitors and 8 pageviews/);
  assert.match(preview,/Outside v1 scope: custom_props/);
  assert.match(preview,/session date/);
  const beforeImport=await fetch(`${address}/sites/1?from=2020-01-01&to=2020-01-01`,{headers:{Cookie:session}});
  assert.ok((await beforeImport.text()).includes('<span>Total pageviews</span><strong>0</strong>'));
  assert.match(docker('exec', name, 'owlytics', 'import-plausible', 'container.example', '/data/plausible-import'), /Imported 3 visitors and 8 pageviews/);
  const historical = await fetch(`${address}/sites/1?from=2020-01-01&to=2020-01-01`, {headers: {Cookie: session}});
  assert.equal(historical.status, 200);
  const historicalHtml=await historical.text();
  assert.ok(historicalHtml.includes('<span>Total pageviews</span><strong>8</strong>'));
  assert.ok(historicalHtml.includes('Search · 9 pageviews'));
  async function checkImportedFilter() {
    const calendar=await fetch(`${address}/sites/1?period=month&date=2020-01-31&compare=off`,{headers:{Cookie:session}});
    assert.equal(calendar.status,200);
    const calendarHtml=await calendar.text();
    assert.ok(calendarHtml.includes('<span>January 2020</span>'));
    assert.ok(calendarHtml.includes('<span>Total pageviews</span><strong>8</strong>'));
    assert.ok(calendarHtml.includes('aria-disabled="true" aria-label="Previous month"'));
    assert.ok(calendarHtml.includes('<a class="chart-hit"'));
    const result=await fetch(`${address}/sites/1?from=2020-01-01&to=2020-01-01&browser=Chrome&browser_version=1`,{headers:{Cookie:session}});
    assert.equal(result.status,200);
    const html=await result.text();
    assert.ok(html.includes('<span>Unique visitors</span><strong>2</strong>'));
    assert.ok(html.includes('<span>Total pageviews</span><strong>5</strong>'));
    assert.ok(html.includes('Some historical data is unavailable with these filters.'));
    const hidden=await fetch(`${address}/sites/1?from=2020-01-01&to=2020-01-01&browser=Chrome&browser_version=1&with_imported=false`,{headers:{Cookie:session}});
    assert.equal(hidden.status,200);
    const hiddenHtml=await hidden.text();
    assert.ok(hiddenHtml.includes('<span>Total pageviews</span><strong>0</strong>'));
    assert.ok(hiddenHtml.includes('role="switch" aria-checked="false"'));
    assert.ok(!hiddenHtml.includes('Some historical data is unavailable with these filters.'));
  }
  await checkImportedFilter();
  write('/data/geo.csv', '203.0.113.1,203.0.113.9,PH\n');
  assert.match(docker('exec', name, 'owlytics', 'import-geo', '/data/geo.csv'), /Imported 1 country ranges/);
  docker('exec', name, 'owlytics', 'backup', '/data/check.sqlite');
  docker('exec', '--env', 'OWLYTICS_DATABASE=/data/restored.sqlite', name, 'owlytics', 'restore', '/data/check.sqlite');
  docker('restart', '--time', '5', name);
  locate(); // Docker can assign a new ephemeral host port on restart.
  await healthy();
  await checkReport();
  await checkImportedFilter();

  // Recover with the README's disposable-container command, without an env hash.
  const recovered = 'recovered container owner password';
  const reset = execFileSync('docker', ['run', '--rm', '-i', '--volume', `${volume}:/data`, image, 'reset-password'], {
    input: recovered, encoding: 'utf8', timeout: 20000,
  });
  assert.match(reset, /Password reset\. All sessions signed out\./);
  const revoked = await fetch(`${address}/`, {headers: {Cookie: session}, redirect: 'manual'});
  assert.equal(revoked.status, 303);
  assert.equal((await attemptPassword(password)).status, 401);
  assert.equal((await attemptPassword(recovered)).status, 303);
  for (const fallback of [undefined, 'invalid-hash', hash]) {
    docker('rm', '--force', name);
    start(fallback);
    await healthy();
    assert.equal((await attemptPassword(password)).status, 401);
    assert.equal((await attemptPassword(recovered)).status, 303);
  }
  console.log('Container passes: non-root, license notices, health, login, collection, engagement, deduplication, assets, imports, backup/restore, restart persistence, and password recovery without an environment hash.');
} catch (error) {
  console.error(docker('logs', name));
  throw error;
} finally {
  if (exportDirectory) rmSync(exportDirectory,{recursive:true,force:true});
  try { docker('rm', '--force', name); } finally { docker('volume', 'rm', volume); }
}
