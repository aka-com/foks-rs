/**
 * Layer 3 of the plan's testing (§5): the built UI, in Chromium, with the
 * mocked bridge.
 *
 * This is new work, not something inherited — nothing in this repository
 * drove a browser before. It loads every deep link the shell answers at
 * 1280×860 and holds the Phase 1 gate: **zero console errors, zero page
 * errors, and `document.documentElement.scrollWidth === 1280`** — no
 * horizontal scrollbar at the design's width. A screenshot of each state
 * lands in `shots/` for visual inspection.
 *
 * What it cannot do is named here so nobody mistakes it for coverage it is
 * not: `playwright-core` drives Chromium, never a WKWebView or a WebKitGTK
 * Tauri window, so this validates layout and logic and never the shipping
 * webview. Drag-drop, the picker, clipboard hygiene and the app lock are
 * layer 4 or by hand.
 *
 * The build must be made with `VITE_FOKS_MOCK=1` (the `acceptance:foks-ui`
 * script does that), and it is served over HTTP: a Vite build uses absolute
 * `/assets/...` URLs and module scripts, so `file://` cannot load it.
 */

import { createServer } from 'node:http';
import { mkdir, readFile, rm } from 'node:fs/promises';
import { extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { chromium } from 'playwright-core';

const here = fileURLToPath(new URL('.', import.meta.url));
const DIST = resolve(here, '../../dist');
const SHOTS = resolve(here, 'shots');

/** The installed browser. `playwright install` is not run here. */
const CHROME =
  process.env.FOKS_CHROMIUM ??
  '/opt/pw-browsers/chromium-1194/chrome-linux/chrome';

const WIDTH = 1280;
const HEIGHT = 860;

/**
 * The states the shell answers, as `README.md`'s table lists them. Each is a
 * deep link a person could paste, and each is a screenshot.
 */
const STATES = [
  'all',
  'personal',
  'password',
  'show',
  'resource',
  'file',
  'link',
  'group',
  'new',
  'new-group',
  'new-resource',
  'new-file',
  'new-link',
  'exists',
  'conflict',
  'grid',
  'lease',
  'inactive',
  'issues',
  'agent-lost',
  'groups',
  'join',
  'people',
  'party',
  'federation',
  'items',
  'danger',
  'add',
  'demote',
  'remove',
  'admit',
  'create',
  'join-invite',
  'manage',
  'party-remove',
  'groups-lease',
  'groups-inactive',
  'rekey-menu',
  'servers-list',
  'servers-server',
  'servers-lapsed',
  'servers-rollback',
  'servers-reset',
  'servers-add',
  'servers-unprobed',
  'servers-check',
  'settings-macs',
  'settings-macs-work',
  'settings-phrase',
  'settings-keys',
  'settings-enrol',
  'settings-account',
  'settings-agent',
  'settings-about',
];

// These extensions get app screenshots and an interaction walk.
const GROUP_ITEM_STATES = [
  'group-new-text',
  'group-new-link',
  'group-new-file',
];

const FIRST_RUN_STATES = [
  'boot',
  'who',
  'address',
  'no-address',
  'checked',
  'compare',
  'error',
  'account',
  'existing',
  'protect',
  'phrase',
  'waiting',
  'added',
  'create-group',
  'done',
  'checklist-invited',
  'checklist-own',
];

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.json': 'application/json',
  '.woff2': 'font/woff2',
};

/** A static server over `root`, on a port the OS chooses. */
async function serve(root) {
  const server = createServer((request, response) => {
    const path = new URL(request.url, 'http://127.0.0.1').pathname;
    // The favicon is not part of the design; answering it keeps a 404 out of
    // the console, where it would read as a failure of the page.
    if (path === '/favicon.ico') {
      response.writeHead(200, { 'content-type': 'image/x-icon' });
      response.end();
      return;
    }
    const file = join(root, normalize(path === '/' ? '/index.html' : path));
    if (!file.startsWith(root)) {
      response.writeHead(403).end('forbidden');
      return;
    }
    readFile(file).then(
      (body) => {
        response.writeHead(200, {
          'content-type': TYPES[extname(file)] ?? 'application/octet-stream',
        });
        response.end(body);
      },
      () => {
        response.writeHead(404).end('not found');
      },
    );
  });
  await new Promise((done) => server.listen(0, '127.0.0.1', done));
  const { port } = server.address();
  return {
    origin: `http://127.0.0.1:${port}`,
    close: () => new Promise((done) => server.close(done)),
  };
}

/** Load one address, collect what the browser complained about, shoot it. */
async function visit(context, url, shot) {
  const page = await context.newPage();
  const problems = [];
  page.on('console', (message) => {
    if (message.type() === 'error') problems.push(`console: ${message.text()}`);
  });
  page.on('pageerror', (error) => {
    problems.push(`pageerror: ${error.message}`);
  });
  page.on('requestfailed', (request) => {
    problems.push(`request failed: ${request.url()}`);
  });

  const response = await page.goto(url, { waitUntil: 'load' });
  if (response && !response.ok()) {
    problems.push(`http ${response.status()}`);
  }
  // The shell is a synchronous render off the fixture, but wait for the
  // frame it draws rather than assuming it.
  await page
    .waitForSelector('.app', { timeout: 5000 })
    .catch(() => problems.push('the shell never drew `.app`'));
  if (new URL(url).searchParams.get('state') === 'show') {
    await page
      .waitForSelector('.details .v.mono', { timeout: 5000 })
      .catch(() =>
        problems.push('Show did not reveal the selected exact version'),
      );
  }
  if (
    new URL(url).searchParams.get('state') === 'phrase' &&
    new URL(url).protocol !== 'file:'
  ) {
    await page
      .waitForSelector('.sheet .word', { timeout: 5000 })
      .catch(() =>
        problems.push('the one-time backup phrase was not prepared'),
      );
  }
  if (
    new URL(url).protocol !== 'file:' &&
    new URL(url).searchParams.get('state') === 'link'
  ) {
    await page
      .getByRole('button', { name: 'Read target', exact: true })
      .waitFor({ timeout: 5000 })
      .catch(() =>
        problems.push(
          'the Link target was not kept masked behind an explicit read',
        ),
      );
  }

  const width = await page.evaluate(
    // Runs in the page, where `document` is the browser's, not node's.
    'document.documentElement.scrollWidth',
  );
  if (width !== WIDTH) {
    problems.push(
      `scrollWidth is ${width}, not ${WIDTH} — the page scrolls sideways`,
    );
  }
  await page.screenshot({ path: join(SHOTS, shot) });
  await page.close();
  return problems;
}

async function personaWalks(context, origin) {
  const page = await context.newPage();
  const failures = [];
  const check = (condition, message) => {
    if (!condition) failures.push(message);
  };
  page.on('pageerror', (error) => failures.push(`pageerror: ${error.message}`));
  page.on('console', (message) => {
    if (message.type() === 'error') failures.push(`console: ${message.text()}`);
  });
  try {
    // Marcus: find and open the PDF, then reveal the Household Wi-Fi value.
    await page.goto(`${origin}/?state=all`, { waitUntil: 'load' });
    await page.locator('.search input').fill('passport');
    await page
      .locator('.body .row')
      .filter({ hasText: 'passport-scan.pdf' })
      .click();
    check(
      (await page.locator('.details .dh h2').textContent()) ===
        'passport-scan.pdf',
      'Marcus could not open the passport PDF details',
    );

    await page.goto(`${origin}/?state=group`, { waitUntil: 'load' });
    await page
      .locator('.details')
      .getByRole('button', { name: 'Show', exact: true })
      .click();
    await page.waitForSelector('.details .v.mono');
    check(
      (await page.locator('.details .v.mono').textContent()) ===
        'sunny-kettle-42',
      'Marcus could not reveal the Wi-Fi password',
    );

    // Jun: the same service account is excluded by Admin, included by Member 0.
    const selection = encodeURIComponent('team:eng|/deploy/production-token');
    await page.goto(
      `${origin}/?state=store&store=team%3Aeng&sel=${selection}`,
      { waitUntil: 'load' },
    );
    const deployBot = page
      .locator('.details .party')
      .filter({ hasText: 'deploy-bot' });
    check(
      (await deployBot.textContent())?.includes('cannot read this'),
      'Sharing did not show that deploy-bot cannot read production-token',
    );
    await page
      .locator('.body .row')
      .filter({ hasText: 'staging-token' })
      .click();
    const stagingBot = page
      .locator('.details .party')
      .filter({ hasText: 'deploy-bot' });
    check(
      !(await stagingBot.textContent())?.includes('cannot read this'),
      'Sharing did not show that deploy-bot can read staging-token',
    );
  } catch (error) {
    failures.push(
      `persona walk: ${error instanceof Error ? error.message : String(error)}`,
    );
  } finally {
    await page.close();
  }
  return failures;
}

async function writeWalk(context, origin) {
  const page = await context.newPage();
  const failures = [];
  page.on('pageerror', (error) => failures.push(`pageerror: ${error.message}`));
  page.on('console', (message) => {
    if (message.type() === 'error') failures.push(`console: ${message.text()}`);
  });
  try {
    await page.goto(`${origin}/?state=new-resource`, { waitUntil: 'load' });
    await page
      .locator('input[aria-label="Name"]')
      .fill('PHASE3_ACCEPTANCE_KEY');
    await page.locator('input[aria-label="Value"]').fill('acceptance value');
    await page
      .getByRole('button', { name: 'Create in Personal', exact: true })
      .click();
    await page.waitForSelector('.flash');
    await page.locator('.search input').fill('phase3_acceptance_key');
    await page
      .locator('.body .row')
      .filter({ hasText: 'phase3_acceptance_key' })
      .waitFor();

    await page.goto(`${origin}/?state=exists`, { waitUntil: 'load' });
    await page.getByRole('button', { name: /Open version/ }).click();
    await page.locator('.details .dh h2', { hasText: 'github.com' }).waitFor();

    await page.goto(`${origin}/?state=agent-lost`, { waitUntil: 'load' });
    await page.getByRole('button', { name: 'Retry', exact: true }).click();
    await page.waitForSelector('.stopwrap', { state: 'detached' });
  } catch (error) {
    failures.push(
      `write walk: ${error instanceof Error ? error.message : String(error)}`,
    );
  } finally {
    await page.close();
  }
  return failures;
}

async function groupWalk(context, origin) {
  const page = await context.newPage();
  const failures = [];
  page.on('pageerror', (error) => failures.push(`pageerror: ${error.message}`));
  page.on('console', (message) => {
    if (message.type() === 'error') failures.push(`console: ${message.text()}`);
  });
  try {
    await page.goto(`${origin}/?state=add`, { waitUntil: 'load' });
    await page.locator('.sheet input').first().fill('phase4.person');
    await page.getByRole('button', { name: 'Add phase4.person' }).click();
    await page.locator('.rt .prow', { hasText: 'phase4.person' }).waitFor();

    await page.goto(`${origin}/?state=party`, { waitUntil: 'load' });
    await page.locator('.details', { hasText: 'deploy-bot' }).waitFor();

    await page.goto(`${origin}/?state=federation`, { waitUntil: 'load' });
    const inactive = page.locator('.rt.fed .prow', { hasText: 'Inactive' });
    await inactive.getByRole('button', { name: 'Restore access' }).click();
    await page.waitForSelector('.flash');

    await page.goto(`${origin}/?state=group`, { waitUntil: 'load' });
    await page
      .getByRole('button', { name: 'Group settings', exact: true })
      .click();
    await page.locator('.ghero', { hasText: 'Household' }).waitFor();
    await page.locator('.ghero .sub', { hasText: 'Group settings' }).waitFor();

    await page.goto(`${origin}/?state=party-remove`, { waitUntil: 'load' });
    const remove = page.getByRole('button', {
      name: 'Remove and rekey',
      exact: true,
    });
    if (!(await remove.isDisabled()))
      failures.push(
        'short party-remove allowed a non-local service account removal',
      );

    await page.goto(`${origin}/?state=join`, { waitUntil: 'load' });
    const inviteCount = await page
      .getByRole('button', { name: /^Invite as .+…$/ })
      .count();
    if (inviteCount !== 2)
      failures.push(
        `Settings Groups offered ${inviteCount} invite choices, not one per fixture account`,
      );

    await page.goto(`${origin}/?state=demote`, { waitUntil: 'load' });
    await page
      .getByRole('button', { name: 'Change role', exact: true })
      .click();
    await page.waitForSelector('.flash');
  } catch (error) {
    failures.push(
      `group walk: ${error instanceof Error ? error.message : String(error)}`,
    );
  } finally {
    await page.close();
  }
  return failures;
}

async function groupItemWalk(context, origin) {
  const page = await context.newPage();
  const failures = [];
  page.on('pageerror', (error) => failures.push(`pageerror: ${error.message}`));
  page.on('console', (message) => {
    if (message.type() === 'error') failures.push(`console: ${message.text()}`);
  });
  try {
    await page.goto(`${origin}/?state=group-new-text`, { waitUntil: 'load' });
    await page
      .getByRole('button', { name: 'Read role Admin', exact: true })
      .click();
    const preview = await page.locator('.sheet').textContent();
    const count = /would be readable by (\d+) of (\d+)/.exec(preview ?? '');
    if (!count)
      failures.push('group create did not render a computed reader preview');
    await page
      .getByRole('textbox', { name: 'Name', exact: true })
      .fill('PHASE7_BROWSER_KEY');
    await page
      .getByRole('textbox', { name: 'Value', exact: true })
      .fill('browser value');
    await page
      .getByRole('button', { name: 'Create in Engineering', exact: true })
      .click();
    const created = page
      .locator('.body .row')
      .filter({ hasText: 'phase7_browser_key' });
    await created.waitFor();
    const people = Number(count?.[1]);
    if (
      !(await created.locator('.chip').textContent())?.includes(String(people))
    ) {
      failures.push(
        'created group item did not retain its computed read-role count',
      );
    }

    await created.click();
    await page
      .locator('.details .dh h2', { hasText: 'phase7_browser_key' })
      .waitFor();
    await page.getByRole('button', { name: 'Edit', exact: true }).click();
    await page.locator('.details textarea').fill('edited browser value');
    await page
      .getByRole('button', { name: 'Save version 2', exact: true })
      .click();
    await page.locator('.flash', { hasText: 'Saved version 2' }).waitFor();
    await page.getByRole('button', { name: 'Remove', exact: true }).click();
    await page.getByRole('button', { name: 'Remove', exact: true }).click();
    await page.locator('.flash', { hasText: 'Removed version 2' }).waitFor();
    await created.waitFor({ state: 'detached' });

    await page.goto(`${origin}/?state=group-new-link`, { waitUntil: 'load' });
    await page
      .getByRole('textbox', { name: 'Points to', exact: true })
      .fill('/deploy/staging-token');
    await page
      .getByRole('button', { name: 'Create in Engineering', exact: true })
      .click();
    await page
      .locator('.body .row')
      .filter({ hasText: 'latest-key' })
      .waitFor();

    await page.goto(`${origin}/?state=group-new-file`, { waitUntil: 'load' });
    await page
      .getByRole('button', { name: 'Choose file and create', exact: true })
      .click();
    await page
      .locator('.body .row')
      .filter({ hasText: 'emergency.pdf' })
      .waitFor();
  } catch (error) {
    failures.push(
      `group item walk: ${error instanceof Error ? error.message : String(error)}`,
    );
  } finally {
    await page.close();
  }
  return failures;
}

async function firstRunWalk(context, origin) {
  const page = await context.newPage();
  const failures = [];
  page.on('pageerror', (error) => failures.push(`pageerror: ${error.message}`));
  page.on('console', (message) => {
    if (message.type() === 'error') failures.push(`console: ${message.text()}`);
  });
  const reloadAt = async (copy) => {
    await page.reload({ waitUntil: 'load' });
    await page.locator('.main', { hasText: copy }).waitFor();
  };
  try {
    await page.goto(`${origin}/?state=who&path=invited`, { waitUntil: 'load' });
    await page.evaluate("window.localStorage.removeItem('foks.first-run.v1')");
    await page.reload({ waitUntil: 'load' });
    await page.getByRole('button', { name: /Someone invited me/ }).click();
    await reloadAt('Select a server address');
    await page
      .getByRole('button', { name: 'Check the server', exact: true })
      .click();
    await page.locator('.pane', { hasText: 'Pinned on this Mac' }).waitFor();
    await reloadAt('Pinned on this Mac');
    await page.getByRole('button', { name: 'Continue', exact: true }).click();
    await page.locator('.pane', { hasText: 'Create an account' }).waitFor();
    await reloadAt('Create an account');
    await page
      .getByRole('button', { name: 'Create my account', exact: true })
      .click();
    await page.locator('.pane', { hasText: 'Save recovery phrase' }).waitFor();
    await reloadAt('Save recovery phrase');

    await page
      .getByRole('button', { name: 'Show my phrase', exact: true })
      .click();
    await page.locator('.sheet .word').first().waitFor();
    await page.reload({ waitUntil: 'load' });
    if (await page.locator('.sheet').count())
      failures.push('a prepared backup phrase survived reload');
    await page
      .getByRole('button', { name: 'Show my phrase', exact: true })
      .click();
    await page.locator('.sheet .word').first().waitFor();
    await page.locator('.sheet .check').click();
    await page
      .locator('.sheet')
      .getByRole('button', { name: 'Done', exact: true })
      .click();
    await page
      .getByRole('button', { name: 'Show my phrase', exact: true })
      .click();
    await page.locator('.sheet .word').first().waitFor();
    await page
      .locator('.sheet')
      .getByRole('button', { name: 'Not now', exact: true })
      .click();
    await page.getByRole('button', { name: 'Continue', exact: true }).click();
    await page.locator('.pane', { hasText: 'Waiting for sam.ortiz' }).waitFor();
    await reloadAt('Waiting for sam.ortiz');
    await page
      .locator('.pcard')
      .getByRole('button', { name: 'Check now', exact: true })
      .click();
    await page
      .locator('.notice', { hasText: 'You’re in Engineering' })
      .waitFor();
    await reloadAt('You’re in Engineering');

    const checkpoint = await page.evaluate(
      "window.localStorage.getItem('foks.first-run.v1') ?? ''",
    );
    if (
      /orbit|"invite"|"passphrase"|"recoveryPhrase"|"backupPhrase"/.test(
        checkpoint,
      )
    ) {
      failures.push('the first-run checkpoint retained secret form material');
    }
  } catch (error) {
    failures.push(
      `first-run walk: ${error instanceof Error ? error.message : String(error)}`,
    );
  } finally {
    await page.close();
  }
  return failures;
}

async function adeWalk(context, origin) {
  const page = await context.newPage();
  const failures = [];
  page.on('pageerror', (error) => failures.push(`pageerror: ${error.message}`));
  page.on('console', (message) => {
    if (message.type() === 'error') failures.push(`console: ${message.text()}`);
  });
  try {
    await page.goto(`${origin}/?state=servers-unprobed`, { waitUntil: 'load' });
    await page.getByRole('button', { name: 'Check now', exact: true }).click();
    await page.locator('.main', { hasText: 'New pin' }).waitFor();
    await page
      .locator('summary', { hasText: 'Details' })
      .click()
      .catch(() => {});
    await page.getByRole('button', { name: 'Show full', exact: true }).click();
    const host = await page.locator('.inset .mono').first().textContent();
    if (!host) failures.push('Ade could not inspect the checked host id');
    else {
      if (!/^02[0-9a-f]{64}$/.test(host))
        failures.push('Ade did not receive the full canonical host id');
      await page.getByLabel('They published').fill(host);
      if (
        (await page.locator('.server-compare').textContent())?.includes(
          'Match',
        ) !== true
      )
        failures.push('Ade out-of-band comparison did not report Match');
    }

    await page.goto(`${origin}/?state=servers-reset`, { waitUntil: 'load' });
    await page.locator('.sheet', { hasText: 'team-creation' }).waitFor();
    await page.locator('.sheet input').fill('personal');
    await page
      .getByRole('button', { name: 'Reset local state', exact: true })
      .click();
    await page.waitForSelector('.sheet', { state: 'detached' });

    await page.goto(`${origin}/?state=settings-phrase`, { waitUntil: 'load' });
    const tokens = await page.locator('.sheet .word').count();
    if (tokens !== 17) failures.push(`Ade saw ${tokens} backup tokens, not 17`);
    if (await page.getByRole('button', { name: /copy/i }).count())
      failures.push('backup phrase offered a copy button');

    await page.goto(`${origin}/?state=settings-enrol`, { waitUntil: 'load' });
    await page
      .locator('.sheet', { hasText: 'Create a YubiKey account' })
      .waitFor();
    if ((await page.locator('.sheet input[type="password"]').count()) < 2)
      failures.push('YubiKey PIN/PUK fields were not concealed');
  } catch (error) {
    failures.push(
      `Ade walk: ${error instanceof Error ? error.message : String(error)}`,
    );
  } finally {
    await page.close();
  }
  return failures;
}

async function main() {
  await rm(SHOTS, { recursive: true, force: true });
  await mkdir(SHOTS, { recursive: true });

  const site = await serve(DIST);
  const browser = await chromium.launch({
    executablePath: CHROME,
    args: ['--no-sandbox'],
  });
  const context = await browser.newContext({
    viewport: { width: WIDTH, height: HEIGHT },
    deviceScaleFactor: 1,
  });

  let failed = 0;
  try {
    for (const state of STATES) {
      const problems = await visit(
        context,
        `${site.origin}/?state=${state}`,
        `${state}.png`,
      );
      if (problems.length) {
        failed += 1;
        console.error(`FAIL ?state=${state}`);
        for (const problem of problems) console.error(`  ${problem}`);
      } else {
        console.log(`ok   ?state=${state}`);
      }
    }

    for (const state of GROUP_ITEM_STATES) {
      const problems = await visit(
        context,
        `${site.origin}/?state=${state}`,
        `${state}.png`,
      );
      if (problems.length) {
        failed += 1;
        console.error(`FAIL ?state=${state}`);
        for (const problem of problems) console.error(`  ${problem}`);
      } else {
        console.log(`ok   ?state=${state} (Phase 7 extension)`);
      }
    }

    const walkProblems = await personaWalks(context, site.origin);
    if (walkProblems.length) {
      failed += 1;
      console.error('FAIL Marcus / Jun acceptance walks');
      for (const problem of walkProblems) console.error(`  ${problem}`);
    } else {
      console.log('ok   Marcus PDF + Wi-Fi / Jun deploy-bot Sharing walks');
    }

    const writeProblems = await writeWalk(context, site.origin);
    if (writeProblems.length) {
      failed += 1;
      console.error('FAIL Phase 3 create / refusal / reconnect walk');
      for (const problem of writeProblems) console.error(`  ${problem}`);
    } else {
      console.log('ok   Phase 3 create / refusal / reconnect walk');
    }

    const groupProblems = await groupWalk(context, site.origin);
    if (groupProblems.length) {
      failed += 1;
      console.error('FAIL Phase 4 roster / party / federation walk');
      for (const problem of groupProblems) console.error(`  ${problem}`);
    } else {
      console.log('ok   Phase 4 roster / party / federation walk');
    }

    const groupItemProblems = await groupItemWalk(context, site.origin);
    if (groupItemProblems.length) {
      failed += 1;
      console.error('FAIL Phase 7 group create / exact edit / remove walk');
      for (const problem of groupItemProblems) console.error(`  ${problem}`);
    } else {
      console.log('ok   Phase 7 group create / exact edit / remove walk');
    }

    const firstRunProblems = await firstRunWalk(context, site.origin);
    if (firstRunProblems.length) {
      failed += 1;
      console.error('FAIL Phase 5 Sol first-run / quit-resume walk');
      for (const problem of firstRunProblems) console.error(`  ${problem}`);
    } else {
      console.log('ok   Phase 5 Sol first-run / quit-resume at every step');
    }

    const adeProblems = await adeWalk(context, site.origin);
    if (adeProblems.length) {
      failed += 1;
      console.error('FAIL Phase 6 Ade trust / reset / recovery-key walk');
      for (const problem of adeProblems) console.error(`  ${problem}`);
    } else {
      console.log('ok   Phase 6 Ade trust / reset / recovery-key walk');
    }

    for (const path of ['invited', 'own']) {
      for (const state of FIRST_RUN_STATES) {
        const shot = `first-run-${path}-${state}.png`;
        const problems = await visit(
          context,
          `${site.origin}/?state=${state}&path=${path}`,
          shot,
        );
        if (problems.length) {
          failed += 1;
          console.error(`FAIL ?state=${state}&path=${path}`);
          for (const problem of problems) console.error(`  ${problem}`);
        } else {
          console.log(`ok   ?state=${state}&path=${path}`);
        }
      }
    }
  } finally {
    await context.close();
    await browser.close();
    await site.close();
  }

  console.log(`\nshots in ${SHOTS}`);
  if (failed) {
    console.error(`${failed} state(s) failed the Phase 1 gate`);
    process.exitCode = 1;
  }
}

await main();
