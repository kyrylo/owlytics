// Keep the letter fallback when a website has no usable icon. Stagger cold-cache
// requests so a page with many websites uses only one discovery request at once.
(() => {
  const load = async image => {
    for (let attempt = 0; attempt < 2; attempt++) {
      const ok = await new Promise(resolve => {
        image.onload = () => resolve(true);
        image.onerror = () => resolve(false);
        image.src = image.dataset.favicon;
      });
      if (ok) { image.hidden = false; return; }
      await new Promise(resolve => setTimeout(resolve, 2000));
    }
  };
  let queue = Promise.resolve();
  const scan = () => document.querySelectorAll('img[data-favicon]:not([data-queued])').forEach(image => {
    image.dataset.queued = 'true';
    queue = queue.then(() => load(image));
  });
  scan();
  new MutationObserver(scan).observe(document.body, {childList:true,subtree:true});
})();
