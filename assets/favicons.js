// Let browser-cached icons load together. Only queue discovery retries: a slow
// website or a missing icon must never hold up the rest of the page.
(() => {
  const requests = new Map();
  let retries = Promise.resolve();
  const request = async url => {
    const response = await fetch(url);
    // Finish populating the HTTP cache before the image consumes the same URL.
    await response.arrayBuffer();
    return response.status;
  };
  const discover = async url => {
    let status = await request(url);
    if (status === 503) {
      const retry = async () => {
        for (let attempt = 0; attempt < 8 && status === 503; attempt++) {
          await new Promise(resolve => setTimeout(resolve, 2000));
          status = await request(url);
        }
        return status === 200;
      };
      const result = retries.then(retry);
      retries = result.catch(() => false);
      return result;
    }
    return status === 200;
  };
  const load = async image => {
    const url = image.dataset.favicon;
    if (!requests.has(url)) requests.set(url, discover(url).catch(() => false));
    if (!await requests.get(url) || !image.isConnected) return;
    image.onload = () => { image.hidden = false; };
    image.src = url;
    if (image.complete && image.naturalWidth > 0) image.hidden = false;
  };
  const scan = () => document.querySelectorAll('img[data-favicon]:not([data-queued])').forEach(image => {
    // A closed site picker should not download every website's icon.
    for (let menu = image.closest('details'); menu; menu = menu.parentElement?.closest('details')) {
      if (!menu.open && !menu.querySelector(':scope > summary')?.contains(image)) return;
    }
    image.dataset.queued = 'true';
    load(image);
  });
  document.addEventListener('toggle', scan, true);
  scan();
  new MutationObserver(scan).observe(document.body, {childList:true,subtree:true});
})();
