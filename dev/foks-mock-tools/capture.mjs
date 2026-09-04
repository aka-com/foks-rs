#!/usr/bin/env node
/**
 * Capture a FOKS desktop state from the mocked dev server (port 1431) or the
 * mockup page (port 8133): a PNG plus the outerHTML of the app root.
 *
 *   node dev/foks-mock-tools/capture.mjs '<url>' <out-prefix> [steps.json]
 *
 * steps.json is an optional array of actions run after load, in order:
 *   { "click": "<css selector>" }            click the first match
 *   { "clickText": "Button label" }          click by text INSIDE the app frame
 *                                            ("deck": true to aim at the control
 *                                            deck above it instead)
 *   { "fill": "<css>", "value": "..." }      type into an input
 *   { "press": "Enter" }                     keyboard press on the page
 *   { "hover": "<css>" }
 *   { "wait": 300 }                          milliseconds
 *   { "waitFor": "<css>" }
 *   { "eval": "document.querySelector(...)" } run JS in the page
 *   { "hash": "#..." } / { "goto": "<url>" }
 *
 * Writes <out-prefix>.png and <out-prefix>.html (the DOM). The PNG is the
 * #frame element on the mockup page — the app window alone, without the control
 * deck — or the viewport elsewhere. Console errors and page errors are printed
 * to stderr.
 *
 * Environment: FULLPAGE=1 shoots the whole mockup page (deck included), which is
 * how the deck itself is reviewed — pair it with HEIGHT=1600 so nothing below
 * the fold is cut. WIDTH / HEIGHT set the viewport (default 1280x860, the
 * design's window size). ROOT=".frame" dumps the frame alone, which is what
 * diffs against a /tmp/foks-mock/cap/** capture of the real app.
 * FOKS_CHROMIUM overrides the browser binary.
 *
 * Dev server (the real app, mocked): run from repo root
 *   VITE_FOKS_MOCK=1 pnpm exec vite --config foks-ui/vite.config.ts --port 1431
 * Mockup page: python3 -m http.server -d . 8133
 */
import { chromium } from 'playwright-core';
import { readFile, writeFile } from 'node:fs/promises';

const [, , url, out, stepsFile] = process.argv;
if (!url || !out) {
  console.error('usage: capture.mjs <url> <out-prefix> [steps.json]');
  process.exit(2);
}
const CHROME =
  process.env.FOKS_CHROMIUM ??
  '/Applications/Brave Browser.app/Contents/MacOS/Brave Browser';
const steps = stepsFile ? JSON.parse(await readFile(stepsFile, 'utf8')) : [];
/* The mockup page pads the body 20px each side, so a 1320px viewport gives
   its #frame the app's exact 1280×860; the app itself is shot at 1280. */
const width = Number(
  process.env.WIDTH ?? (url.includes('app-foks') ? 1320 : 1280),
);
const height = Number(process.env.HEIGHT ?? 860);

const browser = await chromium.launch({
  executablePath: CHROME,
  args: ['--no-sandbox'],
});
const page = await browser.newPage({ viewport: { width, height } });
const errors = [];
page.on('console', (m) => {
  if (m.type() === 'error') errors.push('console: ' + m.text());
});
page.on('pageerror', (e) => errors.push('pageerror: ' + e.message));
await page.goto(url, { waitUntil: 'load' });
await page.waitForTimeout(400);
/* On the mockup page the control deck sits ABOVE the app frame in the DOM, so
   an unscoped getByText would hit a deck chip instead of the button a step
   means. clickText therefore searches inside #frame (the window + its overlays)
   and only falls back to the whole page when nothing there matches — which is
   also what happens on the real app (port 1431), where there is no #frame.
   `"deck": true` aims at the deck on purpose. */
const hasFrame = (await page.$('#frame')) != null;
async function byText(step) {
  const want = { exact: step.exact ?? true };
  if (hasFrame && !step.deck) {
    const inFrame = page.locator('#frame').getByText(step.clickText, want);
    if (await inFrame.count()) return inFrame.first();
  }
  if (hasFrame && step.deck)
    return page
      .locator('.mock-controls')
      .getByText(step.clickText, want)
      .first();
  return page.getByText(step.clickText, want).first();
}
for (const step of steps) {
  if (step.click) await page.locator(step.click).first().click();
  else if (step.clickText) await (await byText(step)).click();
  else if (step.fill)
    await page
      .locator(step.fill)
      .first()
      .fill(step.value ?? '');
  else if (step.press) await page.keyboard.press(step.press);
  else if (step.hover) await page.locator(step.hover).first().hover();
  else if (step.wait) await page.waitForTimeout(step.wait);
  else if (step.waitFor) await page.waitForSelector(step.waitFor);
  else if (step.eval) await page.evaluate(step.eval);
  else if (step.hash)
    await page.evaluate((h) => {
      location.hash = h;
    }, step.hash);
  else if (step.goto) await page.goto(step.goto, { waitUntil: 'load' });
  await page.waitForTimeout(step.settle ?? 150);
}
/* On the mockup page, shoot the app frame alone (the control deck above it
   grows with the page) unless FULLPAGE=1 asks for the whole page. */
const frame = process.env.FULLPAGE ? null : await page.$('#frame');
if (frame) await frame.screenshot({ path: out + '.png' });
else await page.screenshot({ path: out + '.png' });
/* Each selector is tried in turn — `document.querySelector('#root, .frame')`
   would answer in DOCUMENT order, which is always the outermost match. */
const rootSel = process.env.ROOT ?? '#root, .frame, body';
const html = await page.evaluate((sel) => {
  const el =
    sel
      .split(',')
      .map((s) => document.querySelector(s.trim()))
      .find(Boolean) ?? document.body;
  const overlays = document.querySelector('#overlays');
  /* …appended only when the dumped element does not already contain it (it
     does when ROOT is `.frame`, the mockup page's default). */
  const extra =
    overlays && overlays.childElementCount && !el.contains(overlays)
      ? '\n<!-- #overlays -->\n' + overlays.outerHTML
      : '';
  return el.outerHTML + extra;
}, rootSel);
await writeFile(out + '.html', html);
if (errors.length) console.error(errors.join('\n'));
console.log(
  `wrote ${out}.png and ${out}.html (${html.length} chars)${errors.length ? ` with ${errors.length} error(s)` : ''}`,
);
await browser.close();
