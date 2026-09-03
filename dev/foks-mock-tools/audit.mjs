#!/usr/bin/env node
/**
 * Audit the assembled mock in a headless browser, without capturing anything:
 *
 *   node dev/foks-mock-tools/audit.mjs [base-url]
 *
 * Walks every flow step (via the flow API, then again as a cold `?flow=&step=`
 * deep link for a sample), checks that
 *   - the step lands on the page it names            -> badPage
 *   - every key it sets is declared by that page, so the deck can light a chip
 *     for it and the key survives navigation         -> undeclared
 *   - every value it sets lights a chip in its group -> unlit
 *   - the render raises no console/page errors       -> errors
 * then renders every registered page once (pages), reports deck chip labels
 * that collide with in-frame button text (chipClash — advisory since
 * capture.mjs scopes clickText to the frame) and round-trips five deep links
 * (roundTrip). Everything but chipClash should be empty.
 */
import { chromium } from 'playwright-core';

const base = process.argv[2] ?? 'http://localhost:8133/dev/app-foks.html';
const CHROME =
  process.env.FOKS_CHROMIUM ??
  '/Applications/Brave Browser.app/Contents/MacOS/Brave Browser';
const browser = await chromium.launch({ executablePath: CHROME, args: ['--no-sandbox'] });
const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
const errors = [];
/* The page links ui/kit/tokens.css, which is not in this repo, and the browser
   asks for a favicon: two 404s per load, and Chromium reports them without the
   URL in the message, so the location has to be checked too. */
const KNOWN_404 = /tokens\.css|favicon/;
page.on('console', (m) => {
  if (m.type() !== 'error') return;
  const where = m.location() && m.location().url ? m.location().url : '';
  if (KNOWN_404.test(m.text()) || KNOWN_404.test(where)) return;
  errors.push('console: ' + m.text() + (where ? ' <' + where + '>' : ''));
});
page.on('pageerror', (e) => errors.push('pageerror: ' + e.message));
await page.goto(base, { waitUntil: 'load' });
await page.waitForTimeout(300);

const report = await page.evaluate(() => {
  const M = window.FOKS_MOCK;
  const out = { flows: [], steps: 0, badPage: [], undeclared: [], unlit: [], pages: [], chipClash: [] };
  const appKeys = new Set(M.appGroups.map((g) => g.key));
  const appValues = new Map(M.appGroups.map((g) => [g.key, new Set(g.values.map((v) => String(v.v)))]));
  for (const id of M.flowOrder) {
    const flow = M.flows.find((f) => f.id === id);
    out.flows.push({ id, group: flow.group ?? null, steps: flow.steps.length });
    flow.steps.forEach((st, i) => {
      out.steps += 1;
      M.startFlow(id, i);
      const where = `${id}#${i + 1} ${st.label}`;
      if (M.s.page !== st.page) out.badPage.push(`${where}: wanted ${st.page}, got ${M.s.page}`);
      const declared = new Set((M.pages[M.s.page]?.controls ?? []).map((g) => g.key));
      (M.pages[M.s.page]?.keeps ?? []).forEach((k) => declared.add(k));
      const groups = new Map((M.pages[M.s.page]?.controls ?? []).map((g) => [g.key, g]));
      for (const key of Object.keys(st.set ?? {})) {
        const val = String(st.set[key]);
        if (appKeys.has(key)) {
          if (!appValues.get(key).has(val)) out.unlit.push(`${where}: ${key}=${val} is not a declared app value`);
          continue;
        }
        if (!key.startsWith('v.')) { out.undeclared.push(`${where}: ${key} is not an app key`); continue; }
        const bare = key.slice(2);
        if (!declared.has(bare)) { out.undeclared.push(`${where}: v.${bare} not in ${M.s.page}.controls`); continue; }
        /* A value no chip in the group carries: the deck shows it as a
           `custom` chip, which is right for free text (a draft, a name being
           typed) and wrong for anything the group means to enumerate. */
        const g = groups.get(bare);
        /* `applied` is comma-joinable (README, "the shared vocabulary"), so a
           step may set two tokens the group only declares one at a time. */
        const parts = bare === 'applied' ? val.split(',') : [val];
        if (g && val !== '' && !g.freeText && !parts.every((v) => g.values.some((o) => String(o.v) === v))) {
          out.unlit.push(`${where}: v.${bare}=${val} lights no chip in ${M.s.page}.${bare}`);
        }
      }
    });
  }
  M.stopFlow();
  for (const id of M.pageOrder) {
    try { M.go(id); out.pages.push(id); } catch (e) { out.pages.push(`${id}: THREW ${e.message}`); }
  }
  /* Deck chip labels vs in-frame BUTTON text — capture.mjs clickText hits the first
     DOM match, and the deck is above the frame. Step buttons are excluded: they
     are numbered circles now, so their only text is an integer. */
  M.go(M.pageOrder[0]);
  const deck = new Set(
    [...document.querySelectorAll('.mock-controls .mock-chip')]
      .map((el) => el.textContent.trim()),
  );
  for (const id of M.pageOrder) {
    M.go(id);
    for (const el of document.querySelectorAll('#win button, #overlays button, #win a')) {
      const t = el.textContent.trim();
      if (t && deck.has(t)) out.chipClash.push(`${id}: “${t}”`);
    }
  }
  out.chipClash = [...new Set(out.chipClash)];
  return out;
});

/* URL round-trip: a handful of deep links reproduced on a cold load. */
const trips = [
  '?page=all&acme=lapsed',
  '?flow=first-run-own&step=8',
  '?flow=server-add-and-check&step=6',
  '?page=eng-people&failure=roster',
  '?page=store-personal&sel=acct%3Apersonal%7C%2Flogins%2Fgithub.com&details=shown&reveal=1',
];
const roundTrip = [];
for (const q of trips) {
  await page.goto(base + q, { waitUntil: 'load' });
  await page.waitForTimeout(250);
  const after = await page.evaluate(() => location.search);
  const want = new URLSearchParams(q.slice(1));
  const got = new URLSearchParams(after.slice(1));
  const missing = [...want.entries()].filter(([k, v]) => got.get(k) !== v);
  roundTrip.push({ q, after, missing });
}

console.log(JSON.stringify({ ...report, roundTrip, errors }, null, 1));
await browser.close();
