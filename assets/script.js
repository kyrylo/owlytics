/* Owlytics — MIT. No cookies or browser storage. */
(() => {
  "use strict";
  // Replaced by the server at startup; the standalone tracker keeps the default.
  const respectGpc = true;
  const script = document.currentScript;
  if (!script || window.__owlytics) return;
  window.__owlytics = true;
  const domain = script.getAttribute("data-domain");
  const endpoint = new URL("/api/event", script.src).href;
  const engagementEndpoint = new URL("/api/engagement", script.src).href;
  const local = /^(localhost|127(?:\.\d{1,3}){3}|0\.0\.0\.0|\[?::1\]?)$/.test(location.hostname) || location.hostname.endsWith(".localhost");
  const disabled = () => navigator.doNotTrack === "1" || (respectGpc && navigator.globalPrivacyControl) || navigator.webdriver;
  if (!domain || disabled() || (local && !script.hasAttribute("data-local"))) return;
  let previous, current;
  function allowed() {
    if (!disabled()) return true;
    current = undefined;
    previous = undefined;
    return false;
  }
  function send(url, payload) {
    const body = JSON.stringify(payload);
    const attempt = number => {
      if (!allowed()) return;
      const retry = () => { if (number < 2 && allowed()) setTimeout(() => attempt(number + 1), (number + 1) * 1000); };
      try {
        fetch(url, {method: "POST", headers: {"Content-Type": "text/plain"}, body, keepalive: true, credentials: "omit", referrerPolicy: "no-referrer"})
          .then(response => { if (response.status === 503) retry(); }).catch(retry);
      } catch (_) { /* Analytics must not interrupt the host website. */ }
    };
    attempt(0);
  }
  function scroll() {
    if (!allowed() || !current || current.started === null) return;
    const root = document.documentElement, body = document.body;
    const viewport = window.innerHeight || root?.clientHeight || 0;
    const height = Math.max(root?.scrollHeight || 0, body?.scrollHeight || 0, viewport);
    const bottom = Math.max(window.scrollY || 0, root?.scrollTop || 0, 0) + viewport;
    if (height > 0) current.scroll = Math.max(current.scroll, Math.min(100, Math.round(bottom / height * 100)));
  }
  const foreground = () => document.visibilityState === "visible" && document.hasFocus();
  function flush(final = false) {
    if (!allowed() || !current || !current.seen) return;
    scroll();
    if (current.started !== null) {
      current.time = Math.min(86400000, current.time + Math.max(0, performance.now() - current.started));
      current.started = null;
    }
    const time = Math.round(current.time);
    // Short focus changes may be buffered, but leaving the page must not lose
    // that final foreground time. Identical final snapshots still stay silent.
    if (current.sentScroll >= current.scroll && (time <= current.sentTime || (!final && time - current.sentTime < 3000))) return;
    current.sentTime = time;
    current.sentScroll = current.scroll;
    // Cumulative values make retries and out-of-order tab-hide requests safe.
    send(engagementEndpoint, {domain, id: current.id, time_ms: time, scroll_depth: current.scroll});
  }
  function resume() {
    if (!allowed() || !current || !foreground()) return;
    if (current.started === null) current.started = performance.now();
    current.seen = true;
    scroll();
  }
  function campaign() {
    const params = new URLSearchParams(location.search || "");
    const last = key => params.getAll(key).pop();
    const tags = {};
    // Presence matters: an explicitly empty source overrides the referrer.
    const source = last("utm_source") ?? last("source") ?? last("ref");
    if (source !== undefined) tags.source = source;
    for (const [key, param] of [["medium", "utm_medium"], ["name", "utm_campaign"], ["content", "utm_content"], ["term", "utm_term"]]) {
      const value = last(param);
      if (value !== undefined) tags[key] = value;
    }
    // Send parameter names only; never send a click ID or the full query.
    if (params.has("gclid")) tags.click_id = "gclid";
    else if (params.has("msclkid")) tags.click_id = "msclkid";
    if (Object.values(tags).some(value => /[\u0000-\u001f\u007f-\u009f]/.test(value)) || new TextEncoder().encode(JSON.stringify(tags)).length > 2048) return;
    return tags;
  }
  function track() {
    if (!allowed() || document.visibilityState === "prerender" || document.prerendering) return;
    const path = location.pathname;
    if (path === previous) return;
    flush(true);
    previous = path;
    const bytes = new Uint8Array(16);
    crypto.getRandomValues(bytes);
    let referrer = "";
    try {
      const url = new URL(document.referrer);
      if (["https:", "http:"].includes(url.protocol)) referrer = url.origin + url.pathname;
      else if (url.protocol === "android-app:") referrer = "android-app://" + url.hostname + url.pathname;
    } catch (_) { /* No referrer. */ }
    const id = Array.from(bytes, b => b.toString(16).padStart(2, "0")).join("");
    const visible = foreground();
    current = {id, time: 0, scroll: 0, started: visible ? performance.now() : null, seen: visible, sentTime: -1, sentScroll: -1};
    send(endpoint, {domain, path, referrer, id, campaign: campaign()});
  }
  for (const method of ["pushState", "replaceState"]) {
    const original = history[method];
    history[method] = function () { const result = original.apply(this, arguments); track(); return result; };
  }
  addEventListener("popstate", track);
  addEventListener("pageshow", event => { if (event.persisted) { previous = undefined; track(); } resume(); });
  addEventListener("pagehide", () => flush(true));
  addEventListener("scroll", scroll, {passive: true});
  addEventListener("resize", scroll, {passive: true});
  // Hidden is the last reliable lifecycle signal on mobile; pagehide may never
  // follow if the browser is terminated. Visible blur can still be buffered.
  const visibility = () => { if (foreground()) resume(); else flush(document.visibilityState === "hidden"); };
  document.addEventListener("visibilitychange", visibility);
  addEventListener("blur", visibility);
  addEventListener("focus", visibility);
  document.addEventListener("turbo:load", () => { track(); resume(); });
  document.addEventListener("DOMContentLoaded", scroll);
  document.addEventListener("prerenderingchange", track);
  track();
})();
