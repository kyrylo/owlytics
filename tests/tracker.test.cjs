const {test} = require('node:test');
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const vm = require('node:vm');
const code = readFileSync(new URL('../assets/script.js', `file://${__filename}`), 'utf8');

function browser(options = {}) {
  const calls = [], timers = [], events = new Map(), documentEvents = new Map();
  let next = 1, now = 0;
  const location = {hostname: options.hostname || 'example.com', pathname: '/first', search: options.search || ''};
  const document = {
    currentScript: {src: 'https://stats.example.com/script.js', getAttribute: () => 'example.com', hasAttribute: () => !!options.local},
    visibilityState: 'visible', prerendering: !!options.prerender,
    focused: options.focused ?? true,
    hasFocus() { return this.focused; },
    referrer: options.referrer ?? 'https://search.example/private?q=secret#fragment',
    addEventListener: (name, handler) => documentEvents.set(name, handler),
    documentElement: {scrollHeight: 3000, clientHeight: 600, scrollTop: 0},
    body: {scrollHeight: 3000},
  };
  const context = {document, location, navigator: options.navigator || {}, URL, URLSearchParams, TextEncoder, Uint8Array,
    innerHeight: 600, scrollY: 0, performance: {now: () => now},
    crypto: {getRandomValues: bytes => { bytes.fill(next++); return bytes; }},
    history: {pushState: () => 42, replaceState: () => 43},
    addEventListener: (name, handler) => events.set(name, handler),
    setTimeout: callback => timers.push(callback),
    fetch: (url, request) => { calls.push({url, ...request}); return options.fetch ? options.fetch() : Promise.resolve({status: 204}); },
  };
  context.window = context;
  vm.createContext(context);
  vm.runInContext(code, context);
  return {context, calls, timers, events, documentEvents, advance: ms => now += ms,
    views: () => calls.filter(c => c.url.endsWith('/api/event')),
    engagement: () => calls.filter(c => c.url.endsWith('/api/engagement')).map(c => JSON.parse(c.body)),
    run: () => vm.runInContext(code, context)};
}

test('sends paths without query strings, fragments, credentials or storage', () => {
  const {calls, context} = browser();
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, 'https://stats.example.com/api/event');
  assert.equal(calls[0].credentials, 'omit');
  assert.equal(calls[0].referrerPolicy, 'no-referrer');
  assert.equal(calls[0].headers['Content-Type'], 'text/plain');
  assert.deepEqual(JSON.parse(calls[0].body), {domain: 'example.com', path: '/first', referrer: 'https://search.example/private', id: '01'.repeat(16), campaign: {}});
  assert.equal(context.localStorage, undefined);
  assert.equal(context.document.cookie, undefined);
});

test('counts real path changes and back navigation, deduplicating Turbo and repeated installation', () => {
  const b = browser();
  b.run();
  b.documentEvents.get('turbo:load')();
  b.context.history.replaceState();
  assert.equal(b.calls.length, 1);
  b.context.location.pathname = '/second';
  assert.equal(b.context.history.pushState(), 42);
  b.documentEvents.get('turbo:load')();
  assert.equal(b.views().length, 2);
  b.context.location.pathname = '/first';
  b.events.get('popstate')();
  assert.equal(b.views().length, 3);
  b.events.get('pageshow')({persisted: true});
  assert.equal(b.views().length, 4);
  assert.equal(new Set(b.views().map(c => JSON.parse(c.body).id)).size, 4);
});

test('honors DNT, GPC, automated browsers and local development defaults', () => {
  for (const navigator of [{doNotTrack: '1'}, {globalPrivacyControl: true}, {webdriver: true}]) assert.equal(browser({navigator}).calls.length, 0);
  for (const hostname of ['localhost', '127.0.0.1', '127.1.2.3', '[::1]', 'test.localhost']) assert.equal(browser({hostname}).calls.length, 0, hostname);
  assert.equal(browser({hostname: 'localhost', local: true}).calls.length, 1);
});

test('waits for prerender activation', () => {
  const b = browser({prerender: true});
  assert.equal(b.calls.length, 0);
  b.context.document.prerendering = false;
  b.documentEvents.get('prerenderingchange')();
  assert.equal(b.calls.length, 1);
});

test('retries temporary failures at most twice using the same request ID', async () => {
  const b = browser({fetch: () => Promise.resolve({status: 503})});
  await new Promise(setImmediate);
  assert.equal(b.timers.length, 1);
  b.timers.shift()();
  await new Promise(setImmediate);
  b.timers.shift()();
  await new Promise(setImmediate);
  assert.equal(b.calls.length, 3);
  assert.equal(b.timers.length, 0);
  assert.equal(new Set(b.calls.map(c => c.body)).size, 1);
  const invalid = browser({fetch: () => Promise.resolve({status: 400})});
  await new Promise(setImmediate);
  assert.equal(invalid.timers.length, 0);
});

test('new privacy preferences stop navigation events and pending retries', async () => {
  for (const preference of ['doNotTrack', 'globalPrivacyControl']) {
    const b = browser({fetch: () => Promise.resolve({status:503})});
    await new Promise(setImmediate);
    assert.equal(b.timers.length,1);
    b.context.navigator[preference] = preference === 'doNotTrack' ? '1' : true;
    b.context.location.pathname = '/private';
    b.context.history.pushState();
    b.documentEvents.get('turbo:load')();
    b.timers.shift()();
    await new Promise(setImmediate);
    assert.equal(b.calls.length,1);
    assert.equal(b.timers.length,0);
  }
});

test('a synchronous fetch failure cannot interrupt website navigation', () => {
  const b = browser({fetch: () => { throw new TypeError('Request blocked'); }});
  b.context.location.pathname='/next';
  assert.equal(b.context.history.pushState(),42);
  assert.equal(b.views().length,2);
  assert.equal(b.timers.length,0);
});

test('engagement counts foreground time, includes the viewport and pauses in background tabs', () => {
  const b=browser();
  b.advance(1200);
  b.context.scrollY=900;b.events.get('scroll')();
  b.context.document.visibilityState='hidden';b.documentEvents.get('visibilitychange')();
  b.events.get('pagehide')();
  assert.deepEqual(b.engagement(),[{domain:'example.com',id:'01'.repeat(16),time_ms:1200,scroll_depth:50}]);
  b.advance(9000);
  b.context.scrollY=0;b.events.get('scroll')();
  b.context.document.visibilityState='visible';b.documentEvents.get('visibilitychange')();
  b.advance(3000);b.events.get('pagehide')();
  assert.equal(b.engagement()[1].time_ms,4200);
  assert.equal(b.engagement()[1].scroll_depth,50);
  assert.equal(b.views().length,1);
});

test('SPA navigation flushes the old page and starts independent engagement measurements', () => {
  const b=browser();
  b.advance(5000);b.context.location.pathname='/next';b.context.history.pushState();
  assert.equal(b.engagement()[0].id,'01'.repeat(16));
  assert.equal(b.engagement()[0].time_ms,5000);
  assert.equal(b.engagement()[0].scroll_depth,20);
  b.context.document.documentElement.scrollHeight=600;b.context.document.body.scrollHeight=600;
  b.documentEvents.get('turbo:load')();b.advance(1000);b.events.get('pagehide')();
  assert.equal(b.engagement()[1].id,'02'.repeat(16));
  assert.equal(b.engagement()[1].time_ms,1000);
  assert.equal(b.engagement()[1].scroll_depth,100);
  assert.equal(b.views().length,2);
});

test('engagement retries preserve a cumulative snapshot when a later update has already been sent', async () => {
  let requests=0;
  const b=browser({fetch:()=>Promise.resolve({status:++requests===1?204:503})});
  b.advance(1000);b.context.document.visibilityState='hidden';b.documentEvents.get('visibilitychange')();
  await new Promise(setImmediate);
  b.context.document.visibilityState='visible';b.documentEvents.get('visibilitychange')();
  b.advance(3000);b.context.document.visibilityState='hidden';b.documentEvents.get('visibilitychange')();
  await new Promise(setImmediate);
  b.timers.shift()();
  await new Promise(setImmediate);
  assert.deepEqual(b.engagement().map(e=>e.time_ms),[1000,4000,1000]);
  assert.equal(new Set(b.engagement().map(e=>e.id)).size,1);
  b.context.navigator.globalPrivacyControl=true;
  while(b.timers.length)b.timers.shift()();
  b.events.get('pagehide')();
  await new Promise(setImmediate);
  assert.equal(b.engagement().length,3);
});

test('background or prerendered pages do not collect time until they become visible', () => {
  const b=browser({prerender:true});
  b.context.document.visibilityState='hidden';b.context.document.prerendering=false;b.documentEvents.get('prerenderingchange')();
  b.advance(60000);b.events.get('pagehide')();
  assert.equal(b.engagement().length,0);
  b.context.document.visibilityState='visible';b.documentEvents.get('visibilitychange')();
  b.advance(250);b.events.get('pagehide')();
  assert.equal(b.engagement()[0].time_ms,250);
  assert.equal(b.views().length,1);
});

test('focus pauses visible pages and short revisits accumulate until three seconds or deeper scrolling', () => {
  const b=browser({focused:false});
  b.advance(9000);b.events.get('pagehide')();
  assert.equal(b.engagement().length,0);
  const focus=()=>{b.context.document.focused=true;b.events.get('focus')();};
  const blur=()=>{b.context.document.focused=false;b.events.get('blur')();};
  focus();b.advance(1000);blur();
  assert.equal(b.engagement()[0].time_ms,1000);
  b.advance(9000);focus();b.advance(1000);blur();
  assert.equal(b.engagement().length,1,'A short revisit with unchanged scroll is buffered');
  b.advance(9000);focus();b.advance(2000);blur();
  assert.equal(b.engagement()[1].time_ms,4000,'Only focused time is accumulated');
  focus();b.advance(100);b.context.scrollY=900;b.events.get('scroll')();blur();
  assert.deepEqual(b.engagement().map(e=>[e.time_ms,e.scroll_depth]),[[1000,20],[4000,20],[4100,50]]);
});

test('overlapping focus and visibility events neither restart hidden clocks nor duplicate measurements', () => {
  const b=browser();b.advance(4000);
  b.context.document.focused=false;b.events.get('blur')();
  b.context.document.visibilityState='hidden';b.documentEvents.get('visibilitychange')();
  b.advance(60000);b.events.get('focus')();
  b.context.document.visibilityState='visible';b.documentEvents.get('visibilitychange')();
  b.advance(5000);b.events.get('pagehide')();
  assert.equal(b.engagement().length,1);
  b.context.document.focused=true;b.events.get('focus')();
  b.advance(3000);b.events.get('pagehide')();
  assert.deepEqual(b.engagement().map(e=>e.time_ms),[4000,7000]);
});

test('leaving a page sends buffered short revisits without duplicating the final measurement', () => {
  for (const leave of ['pagehide', 'navigation', 'hidden']) {
    const b=browser();b.advance(1000);
    b.context.document.focused=false;b.events.get('blur')();
    b.advance(9000);
    b.context.document.focused=true;b.events.get('focus')();
    b.advance(1000);
    // Browsers can hide/blur before pagehide; that short increment is buffered.
    b.context.document.focused=false;b.events.get('blur')();
    if (leave==='pagehide') {
      b.events.get('pagehide')();b.events.get('pagehide')();
    } else if (leave==='hidden') {
      b.context.document.visibilityState='hidden';
      b.documentEvents.get('visibilitychange')();
      assert.deepEqual(b.engagement().map(e=>e.time_ms),[1000,2000],'Hidden pages may terminate without pagehide');
      b.events.get('pagehide')();
    } else {
      b.context.location.pathname='/next';b.context.history.pushState();
    }
    assert.deepEqual(b.engagement().map(e=>e.time_ms),[1000,2000],leave);
    assert.equal(b.engagement()[1].id,'01'.repeat(16));
  }
});

test('allowlists campaign labels with source precedence and never sends click ID values', () => {
  const b = browser({search: '?ref=old&source=middle&utm_source=first&utm_source=News%20Letter&utm_medium=Email&utm_campaign=Autumn&utm_content=CTA&utm_term=Rust+SQLite&gclid=secret-click&msclkid=another-secret&fbclid=private-id&email=private@example.com&token=private-token'});
  assert.deepEqual(JSON.parse(b.views()[0].body).campaign, {source: 'News Letter', medium: 'Email', name: 'Autumn', content: 'CTA', term: 'Rust SQLite', click_id: 'gclid'});
  assert.doesNotMatch(b.views()[0].body, /secret-click|another-secret|private-id|private@example.com|private-token/);
  for (const [search, expected] of [['?ref=Referral', {source:'Referral'}], ['?ref=old&source=New', {source:'New'}], ['?source=old&utm_source=', {source:''}], ['?msclkid=secret', {click_id:'msclkid'}], ['?utm_source=%E7%8C%AB&utm_campaign=%F0%9F%A6%89', {source:'猫', name:'🦉'}]]) {
    assert.deepEqual(JSON.parse(browser({search}).views()[0].body).campaign, expected);
  }
  b.context.location.search = '?utm_source=Later';
  b.context.history.replaceState();
  assert.equal(b.views().length, 1, 'query-only changes do not add a pageview');
  b.context.location.pathname = '/second';
  b.context.history.pushState();
  assert.equal(JSON.parse(b.views()[1].body).campaign.source, 'Later');
});

test('campaign limits omit unknown attribution without losing pageviews or leaking other parameters', () => {
  for (const search of ['?utm_source=' + '🦉'.repeat(600), '?utm_source=' + '"'.repeat(1100), '?utm_term=private%00data', '?utm_campaign=a%7Fb', '?utm_source='+'a'.repeat(1100)+'&utm_medium='+'b'.repeat(1100)]) {
    const b = browser({search});
    assert.equal(b.views().length, 1);
    assert.equal(Object.hasOwn(JSON.parse(b.views()[0].body), 'campaign'), false);
  }
  const valid = browser({search:'?utm_source='+'a'.repeat(2000)});
  assert.equal(JSON.parse(valid.views()[0].body).campaign.source.length, 2000);
});

test('campaign retries retain the original labels and Android referrals strip private URL parts', async () => {
  const b = browser({search:'?utm_source=first', referrer:'android-app://com.google.android.googlequicksearchbox/search?q=private#fragment', fetch:() => Promise.resolve({status:503})});
  assert.equal(JSON.parse(b.views()[0].body).referrer, 'android-app://com.google.android.googlequicksearchbox/search');
  b.context.location.search = '?utm_source=changed';
  await new Promise(setImmediate);
  b.timers.shift()();
  assert.equal(b.views()[0].body, b.views()[1].body);
});
