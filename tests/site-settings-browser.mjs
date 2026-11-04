import assert from 'node:assert/strict';
import {join} from 'node:path';

export async function verifySiteDeletion(browser, origin, storageState, output) {
  for (const javaScriptEnabled of [true, false]) {
    const context = await browser.newContext({storageState, javaScriptEnabled});
    const errors = [];
    try {
      const page = await context.newPage();
      page.on('pageerror', error => errors.push(error.message));
      await page.goto(origin);
      const initialCount = await page.locator('.site-card').count();
      const domain = `delete-${javaScriptEnabled ? 'js' : 'noscript'}.example`;
      await page.getByRole('link', {name:'Add website', exact:true}).click();
      await page.getByLabel('Domain', {exact:true}).fill(domain);
      await page.getByRole('button', {name:'Add website'}).click();
      await page.waitForURL(/\/sites\/\d+\/settings$/);
      const settingsUrl = page.url();
      const dashboardUrl = settingsUrl.replace(/\/settings$/, '');
      await page.goto(origin);
      await page.getByRole('link', {name:`Settings for ${domain}`, exact:true}).click();
      assert.equal(await page.getByRole('heading', {name:'Website settings', exact:true}).count(), 1);
      assert.ok((await page.locator('pre code').innerText()).includes(`data-domain="${domain}"`));
      assert.equal(await page.getByLabel(`Type ${domain} to confirm`).isVisible(), false);
      await page.locator('.delete-confirmation>summary').focus();
      await page.keyboard.press('Enter');
      await page.getByLabel(`Type ${domain} to confirm`).fill('wrong.example');
      await page.getByRole('link', {name:'Cancel', exact:true}).click();
      assert.equal(page.url(), settingsUrl);
      assert.equal(await page.getByLabel(`Type ${domain} to confirm`).isVisible(), false);

      // The website dropdown in the dashboard leads to the same setting.
      await page.getByRole('link', {name:'Open dashboard →', exact:true}).click();
      await page.locator('.site-picker>summary').click();
      await page.getByRole('link', {name:'Website settings', exact:true}).click();
      assert.equal(page.url(), settingsUrl);
      await page.locator('.delete-confirmation>summary').click();
      await page.getByLabel(`Type ${domain} to confirm`).fill('wrong.example');
      const rejected = page.waitForResponse(response => response.url().endsWith('/delete') && response.request().method() === 'POST');
      await page.getByRole('button', {name:'Permanently delete website', exact:true}).click();
      assert.equal((await rejected).status(), 400);
      assert.match(await page.getByRole('alert').innerText(), /domain doesn’t match/);
      assert.equal(await page.getByLabel(`Type ${domain} to confirm`).inputValue(), 'wrong.example');
      assert.equal(await page.getByLabel(`Type ${domain} to confirm`).getAttribute('aria-invalid'), 'true');

      for (const [width, height] of [[1440, 1000], [320, 780]]) {
        for (const colorScheme of ['light', 'dark']) {
          await page.setViewportSize({width, height});
          await page.emulateMedia({colorScheme});
          assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1));
          await page.screenshot({path:join(output, `delete-${javaScriptEnabled ? 'js' : 'noscript'}-${colorScheme}-${width}.png`), fullPage:true});
        }
      }
      await page.getByLabel(`Type ${domain} to confirm`).fill(domain);
      await page.getByRole('button', {name:'Permanently delete website', exact:true}).click();
      await page.waitForURL(`${origin}/`);
      assert.equal(await page.getByRole('status').count(), 0);
      await page.reload();
      assert.equal(page.url(), `${origin}/`);
      assert.equal(await page.getByRole('status').count(), 0);
      assert.equal(await page.locator('.site-card').count(), initialCount);
      assert.equal(await page.getByRole('link', {name:`Settings for ${domain}`, exact:true}).count(), 0);
      assert.equal((await context.request.get(dashboardUrl)).status(), 404);
      assert.deepEqual(errors, []);
    } finally {
      await context.close();
    }
  }
}
