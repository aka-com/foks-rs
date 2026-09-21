/* global document */
/** Browser regression checks. Requires VITE_FOKS_MOCK=1 npm run build:frontend. */
import assert from 'node:assert/strict';
import { chromium } from 'playwright-core';
import { serve } from './run.mjs';

const site = await serve(new URL('../../dist/', import.meta.url).pathname);
let browser;
try {
  browser = await chromium.launch({
    executablePath: process.env.FOKS_CHROMIUM ?? chromium.executablePath(),
    args: ['--no-sandbox'],
  });
  for (const width of [1280, 960]) {
    const page = await browser.newPage({ viewport: { width, height: 860 } });
    page.setDefaultTimeout(10000);
    await page.goto(`${site.origin}/?state=group-channels`);
    await page
      .getByRole('button', { name: 'Open #general in Chat', exact: true })
      .click();
    await page
      .getByRole('button', { name: 'New channel', exact: true })
      .first()
      .click();
    await page.getByRole('button', { name: 'Team', exact: true }).click();
    await page.getByRole('option', { name: /^Household/ }).click();
    await page
      .getByRole('textbox', { name: 'Channel name', exact: true })
      .fill('w'.repeat(32));
    await page
      .getByRole('textbox', { name: 'Channel description', exact: true })
      .fill('w'.repeat(128));
    await page
      .getByRole('button', { name: 'Create channel', exact: true })
      .click();
    await page.getByRole('dialog').waitFor({ state: 'hidden' });
    await page
      .getByRole('button', { name: 'Channel info', exact: true })
      .click();
    const panel = page.getByRole('complementary', {
      name: 'Channel info',
      exact: true,
    });
    await panel.waitFor();
    await panel.evaluate(async (element) => {
      await Promise.all(
        element.getAnimations().map((animation) => animation.finished),
      );
    });
    assert.equal(
      await panel.locator('.chat-info-name').textContent(),
      'w'.repeat(32),
    );
    for (const theme of ['light', 'dark']) {
      await page.evaluate((value) => {
        document.documentElement.dataset.theme = value;
      }, theme);
      const fits = await panel.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        return (
          element.scrollWidth <= element.clientWidth &&
          [
            ...element.querySelectorAll(
              '.chat-info-name, .chat-info-identity p',
            ),
          ].every((text) => {
            const rect = text.getBoundingClientRect();
            return rect.left >= bounds.left && rect.right <= bounds.right;
          })
        );
      });
      assert.ok(
        fits,
        `Channel identity overflows at ${width}px in ${theme} mode`,
      );
    }
    await page.close();
  }
  console.log(
    'Channel identity fits at both desktop widths in light and dark mode.',
  );
} finally {
  await browser?.close();
  await site.close();
}
