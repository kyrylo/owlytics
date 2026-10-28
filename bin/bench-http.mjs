#!/usr/bin/env node
// Run against a disposable database created by examples/benchmark.rs.
import {spawn, execFileSync} from 'node:child_process';
import {randomBytes} from 'node:crypto';
import {performance} from 'node:perf_hooks';
import {resolve, basename} from 'node:path';

const database = process.argv[2];
if (!database || !basename(database).startsWith('owlytics-benchmark-')) throw new Error('Pass an owlytics-benchmark-*.sqlite disposable fixture');
const binary = resolve('target/release/owlytics');
const run = randomBytes(6).toString('hex');
function pageviews(html) {
  const metric = /<a class="metric"[^>]*data-value="(\d+)"[^>]*aria-label="Show pageviews"/.exec(html);
  if (!metric) throw new Error('Missing pageview metric');
  return Number(metric[1]);
}
const hash = await new Promise((done, fail) => {
  const child = spawn(binary, ['hash-password'], {stdio: ['pipe', 'pipe', 'inherit']});
  let result = '';
  child.stdout.on('data', data => result += data);
  child.on('error', fail);
  child.on('exit', code => code === 0 ? done(result.trim()) : fail(new Error('Hash CLI failed')));
  child.stdin.end('disposable benchmark passphrase\n');
});
const child = spawn(binary, [], {env: {...process.env, OWLYTICS_PASSWORD_HASH: hash, OWLYTICS_DATABASE: resolve(database), OWLYTICS_BIND: '127.0.0.1:0', OWLYTICS_URL: 'http://localhost:3000', OWLYTICS_TRUSTED_PROXIES: '127.0.0.1/32'}, stdio: ['ignore', 'pipe', 'inherit']});
let sampler, maxRss = 0;
function rss() {
  const value = Number(execFileSync('ps', ['-o', 'rss=', '-p', String(child.pid)], {encoding: 'utf8'}).trim());
  maxRss = Math.max(maxRss, value);
  return value;
}
try {
  const address = await new Promise((done, fail) => {
    let text = '';
    const timeout = setTimeout(() => fail(new Error('Startup deadline exceeded')), 15000);
    child.stdout.on('data', data => { text += data; const match = /Owlytics listening on (http:\/\/[^\s]+)/.exec(text); if (match) { clearTimeout(timeout); done(match[1]); } });
    child.on('error', fail);
    child.on('exit', code => { clearTimeout(timeout); fail(new Error(`Server exited: ${code}`)); });
  });
  console.log(`idle_rss_kib=${rss()}`);
  sampler = setInterval(() => { try { rss(); } catch {} }, 100);
  const login = await fetch(`${address}/login`);
  const cookie = login.headers.getSetCookie()[0].split(';')[0];
  const csrf = /name="csrf" value="([^"]+)"/.exec(await login.text())[1];
  const signedIn = await fetch(`${address}/login`, {method: 'POST', redirect: 'manual', headers: {Origin: 'http://localhost:3000', Cookie: cookie, 'Content-Type': 'application/x-www-form-urlencoded'}, body: new URLSearchParams({csrf, password: 'disposable benchmark passphrase'})});
  if (signedIn.status !== 303) throw new Error(`Login failed: ${signedIn.status}`);
  const session = signedIn.headers.getSetCookie().find(value => value.startsWith('owlytics_session=')).split(';')[0];
  for (let i = 1; i <= 3; i++) {
    const started = performance.now();
    const response = await fetch(`${address}/sites/1?from=2025-10-01&to=2025-10-30`, {headers: {Cookie: session}});
    const html = await response.text();
    if (response.status !== 200 || pageviews(html) !== 1100000) throw new Error('Historical report failed');
    console.log(`http_report_trial=${i} milliseconds=${(performance.now()-started).toFixed(1)}`);
  }
  const today = new Date().toISOString().slice(0, 10);
  const currentUrl = `${address}/sites/1?from=${today}&to=${today}&source=news.example`;
  const before = await fetch(currentUrl, {headers:{Cookie:session}});
  if (before.status !== 200) throw new Error('Could not read initial count');
  const initialViews = pageviews(await before.text());
  const indexStarted = performance.now();
  const index = await fetch(address, {headers:{Cookie:session}});
  if (index.status !== 200 || !(await index.text()).includes('site-sparkline')) throw new Error('Project index failed');
  console.log(`http_index_milliseconds=${(performance.now()-indexStarted).toFixed(1)}`);
  const latencies = [], payloads = [];
  let next = 0, retries = 0;
  const total = 2000;
  const started = performance.now();
  async function send(i, body) {
    const response = await fetch(`${address}/api/event`, {method: 'POST', headers: {Origin: 'https://example.com', 'Content-Type': 'text/plain', 'User-Agent': `Mozilla/5.0 (Macintosh; Intel Mac OS X) AppleWebKit/537.36 Chrome/130.0.0.0 Safari/537.36 OwlyticsBenchmark/${run}`, 'X-Forwarded-For': `203.0.113.${i%200+1}`}, body});
    if (response.status !== 204) throw new Error(`Collection failed: ${response.status}`);
  }
  await Promise.all(Array.from({length: 8}, async () => {
    while (next < total) {
      const i = next++;
      const body = JSON.stringify({domain: 'example.com', path: `/benchmark/${i%40}`, referrer: 'https://news.example/', id: randomBytes(16).toString('hex'), campaign:{source:'news.example',medium:'email',name:'Benchmark',content:`link-${i%4}`,term:'Rust SQLite'}});
      payloads[i] = body;
      const began = performance.now();
      await send(i, body);
      latencies.push(performance.now()-began);
    }
  }));
  const elapsed = performance.now()-started;
  await send(0, payloads[0]); // An intentional retry must not create another view.
  const result = await fetch(currentUrl, {headers: {Cookie: session}});
  const html = await result.text();
  if (result.status !== 200 || pageviews(html) !== initialViews + total) throw new Error('Collector count/deduplication mismatch');
  latencies.sort((a,b) => a-b);
  console.log(`collected=${total} concurrency=8 events_per_second=${(total*1000/elapsed).toFixed(1)} p50_ms=${latencies[Math.floor(total*.5)].toFixed(1)} p95_ms=${latencies[Math.floor(total*.95)].toFixed(1)} retries=${retries}`);
  let engagementNext=0;
  const engagementStarted=performance.now(), engagementLatencies=[];
  async function sendEngagement(i) {
    const response=await fetch(`${address}/api/engagement`,{method:'POST',headers:{Origin:'https://example.com','Content-Type':'text/plain','User-Agent':'Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36','X-Forwarded-For':`203.0.113.${i%200+1}`},body:JSON.stringify({domain:'example.com',id:JSON.parse(payloads[i]).id,time_ms:1000,scroll_depth:70})});
    if(response.status!==204) throw new Error(`Engagement collection failed: ${response.status}`);
  }
  await Promise.all(Array.from({length:8},async()=>{
    while(engagementNext<total) {
      const i=engagementNext++, began=performance.now();await sendEngagement(i);engagementLatencies.push(performance.now()-began);
    }
  }));
  const engagementElapsed=performance.now()-engagementStarted;
  await sendEngagement(0);
  const afterEngagement=await fetch(currentUrl,{headers:{Cookie:session}});
  if(afterEngagement.status!==200 || pageviews(await afterEngagement.text())!==initialViews+total) throw new Error('Engagement changed pageview counts');
  engagementLatencies.sort((a,b)=>a-b);
  console.log(`engagement_updates=${total} concurrency=8 updates_per_second=${(total*1000/engagementElapsed).toFixed(1)} p50_ms=${engagementLatencies[Math.floor(total*.5)].toFixed(1)} p95_ms=${engagementLatencies[Math.floor(total*.95)].toFixed(1)}`);
  console.log(`after_collection_rss_kib=${rss()} sampled_peak_rss_kib=${maxRss}`);
} finally {
  clearInterval(sampler);
  child.kill('SIGTERM');
  await new Promise(done => { if (child.exitCode !== null) return done(); const timeout = setTimeout(() => { child.kill('SIGKILL'); done(); }, 5000); child.once('exit', () => { clearTimeout(timeout); done(); }); });
}
