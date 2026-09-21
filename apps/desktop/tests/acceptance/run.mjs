/* global document, innerWidth -- browser globals inside page.evaluate */
/**
 * Acceptance test runner for the built FOKS desktop UI in Chromium with a mocked bridge.
 *
 * Validates layout and deep-link routing at 1280x860, asserting zero console errors,
 * zero page errors, and no horizontal overflow. Screenshots are saved to `shots/`.
 *
 * Requires a build produced with `VITE_FOKS_MOCK=1`.
 */

import { createServer } from 'node:http';
import { mkdir, readFile, rm } from 'node:fs/promises';
import { extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { chromium } from 'playwright-core';

const here = fileURLToPath(new URL('.', import.meta.url));
const DIST = resolve(here, '../../dist');
const SHOTS = resolve(here, 'shots');

/** Path to the Chromium binary used for acceptance tests. */
const CHROME = process.env.FOKS_CHROMIUM ?? chromium.executablePath();

const WIDTH = 1280;
const HEIGHT = 860;

/**
 * UI states and routes verified by the acceptance test suite.
 */
const STATES = [
  'all',
  'personal',
  'password',
  'show',
  'resource',
  'file',
  'group',
  'new',
  'new-group',
  'new-document',
  'exists',
  'conflict',
  'grid',
  'folders',
  'lease',
  'inactive',
  'agent-lost',
  'groups',
  'join',
  'people',
  'group-people',
  'group-channels',
  'group-files',
  'invite',
  'party',
  'federation',
  'items',
  'danger',
  'add',
  'demote',
  'remove',
  'admit',
  'create',
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
const GROUP_ITEM_STATES = ['group-new-document'];

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
export async function serve(root) {
  const server = createServer((request, response) => {
    const path = new URL(request.url, 'http://127.0.0.1').pathname;
    // Stub favicon request to avoid non-fatal console 404 errors during tests.
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

/** Load one address, collect console/page errors, and save a screenshot. */
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
  // Wait for the app container to mount before inspecting state.
  await page
    .waitForSelector('.app', { timeout: 5000 })
    .catch(() => problems.push('the shell never rendered `.app`'));
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

/**
 * Waits for a toast notification element with the specified message to appear.
 */
function toast(page, text) {
  return page.locator('.toast', { hasText: text }).waitFor();
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
    // Chat: create a channel, send text, and navigate back without vault mutation refreshes.
    await page.goto(`${origin}/?state=team-chat&store=team%3Ahousehold`, {
      waitUntil: 'load',
    });
    // Opening a team no longer opens its most-recent channel on its own; the
    // reader picks one from the team's own row in the inbox column. Household
    // has only its general channel, so its row opens straight into it.
    await page
      .getByRole('complementary', { name: 'Chat inbox' })
      .getByRole('button', { name: /^Household/ })
      .click();
    await page.getByText('Team chat is ready.', { exact: true }).waitFor();
    // The sheet opens straight onto the creation form, with the team chosen
    // from its own selector rather than from a step in front of the form.
    await page
      .getByRole('button', { name: 'New channel', exact: true })
      .click();
    await page.getByRole('button', { name: 'Team', exact: true }).click();
    await page.getByRole('option', { name: /^Household/ }).click();
    await page
      .getByRole('textbox', { name: 'Channel name' })
      .fill('design-chat');
    await page
      .getByRole('button', { name: 'Create channel', exact: true })
      .click();
    await page.getByRole('dialog').waitFor({ state: 'detached' });
    await page.getByRole('button', { name: '#design-chat' }).click();
    await page
      .getByRole('textbox', { name: 'Message', exact: true })
      .fill('Browser walkthrough message');
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await page
      .getByText('Browser walkthrough message', { exact: true })
      .waitFor();
    check(
      (await page.locator('.chat-message').count()) === 1,
      'chat message was duplicated',
    );
    check(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
      'chat overflowed the viewport',
    );
    await page.screenshot({ path: join(SHOTS, 'team-chat.png') });
    // The header's folder icon is where "Files" used to be a labeled button.
    await page.getByRole('button', { name: 'Team files', exact: true }).click();
    check(
      !new URL(page.url()).searchParams.has('channel'),
      'chat channel leaked into file navigation',
    );

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
      `${origin}/?state=store&store=team%3Aeng&sel=${selection}&view=list`,
      { waitUntil: 'load' },
    );
    const deployBot = page
      .locator('.details .party')
      .filter({ hasText: 'deploy-bot' });
    check(
      (await deployBot.textContent())?.includes('No read access'),
      'Sharing did not show that deploy-bot cannot read production-token',
    );
    // The folder browser opens on production-token's selection, not its
    // folder; staging-token is a sibling reached by entering /deploy.
    await page.locator('.body .row').filter({ hasText: 'deploy' }).click();
    await page
      .locator('.body .row')
      .filter({ hasText: 'staging-token' })
      .click();
    const stagingBot = page
      .locator('.details .party')
      .filter({ hasText: 'deploy-bot' });
    check(
      !(await stagingBot.textContent())?.includes('No read access'),
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
    await page.goto(`${origin}/?state=new-document`, { waitUntil: 'load' });
    await page
      .locator('input[aria-label="Name"]')
      .fill('PHASE3_ACCEPTANCE_KEY');
    await page.locator('input[aria-label="Value"]').fill('acceptance value');
    // Submit creation in target vault.
    await page
      .getByRole('button', { name: 'Create item', exact: true })
      .click();
    await toast(page, 'created in Personal');
    // Verify the item appears in the list under the selected vault.
    await page.locator('.search input').fill('phase3_acceptance_key');
    const written = page
      .locator('.body .row')
      .filter({ hasText: 'phase3_acceptance_key' });
    await written.waitFor();
    if (!(await written.textContent())?.includes('Personal'))
      failures.push('the created item did not appear in Personal');

    await page.goto(`${origin}/?state=exists`, { waitUntil: 'load' });
    await page.getByRole('button', { name: /Open existing item/ }).click();
    await page.locator('.details .dh h2', { hasText: 'github.com' }).waitFor();

    await page.goto(`${origin}/?state=agent-lost`, { waitUntil: 'load' });
    await page.getByRole('button', { name: 'Retry', exact: true }).click();
    await page.waitForSelector('.stopwrap', { state: 'detached' });
    await page.locator('.body .row').first().waitFor();
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

    // A member's actions live in that row's menu, which the scene opens. The
    // menu is named for the member it acts on, since every row has one.
    await page.goto(`${origin}/?state=party`, { waitUntil: 'load' });
    await page.locator('.rt .prow', { hasText: 'deploy-bot' }).waitFor();
    await page
      .locator('.menu[aria-label="Actions for deploy-bot"]', {
        hasText: 'Lower role',
      })
      .waitFor();

    await page.goto(`${origin}/?state=federation`, { waitUntil: 'load' });
    // Admitted teams are drawn in the same list as people now, not a
    // separately classed table.
    const inactive = page.locator('.rt.bare .prow', { hasText: 'Inactive' });
    await inactive.getByRole('button', { name: 'Restore access' }).click();
    await toast(page, 'Team access restored');
    // Verify the table row displays active status.
    await page.locator('.rt.bare .prow', { hasText: 'Active' }).waitFor();
    if (await inactive.count())
      failures.push('the restored team is still Inactive');

    await page.goto(`${origin}/?state=group`, { waitUntil: 'load' });
    await page
      .getByRole('button', { name: 'Team settings', exact: true })
      .click();
    await page.locator('.ghero', { hasText: 'Household' }).waitFor();
    // The header chip is the server and the member count; the bare role
    // this account holds reads on its own row under Members instead.
    const subtitle = (await page.locator('.ghero .sub').innerText()).trim();
    if (subtitle !== 'Personal server · 2 members')
      failures.push(`the team header subtitle read "${subtitle}"`);
    await page
      .locator('.rt.bare .prow', { hasText: 'you' })
      .filter({ hasText: 'Owner' })
      .waitFor();

    await page.goto(`${origin}/?state=party-remove`, { waitUntil: 'load' });
    const remove = page.getByRole('button', {
      name: 'Remove and rekey',
      exact: true,
    });
    if (!(await remove.isDisabled()))
      failures.push(
        'short party-remove allowed a non-local service account removal',
      );

    await page.goto(`${origin}/?state=demote`, { waitUntil: 'load' });
    await page
      .getByRole('button', { name: 'Change role', exact: true })
      .click();
    await toast(page, 'Lower priya.n\u2019s role completed');
    // Confirm updated role in roster.
    const demoted = page.locator('.rt .prow', { hasText: 'priya.n' });
    await demoted.filter({ hasText: 'Member' }).waitFor();
    if ((await demoted.textContent())?.includes('Admin'))
      failures.push('priya.n is still an Admin after the role change');
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
    await page.goto(`${origin}/?state=group-new-document&view=list`, {
      waitUntil: 'load',
    });
    // Select Admin role from the radiogroup.
    await page
      .getByRole('radiogroup', { name: 'Who can read' })
      .getByRole('radio', { name: /^Admin/ })
      .click();
    const preview = await page.locator('.sheet').textContent();
    const count = /readable by (\d+) of (\d+)/.exec(preview ?? '');
    if (!count)
      failures.push('group create did not render a computed reader preview');
    await page
      .getByRole('textbox', { name: 'Name', exact: true })
      .fill('PHASE7_BROWSER_KEY');
    await page
      .getByRole('textbox', { name: 'Value', exact: true })
      .fill('browser value');
    await page
      .getByRole('button', { name: 'Create item', exact: true })
      .click();
    await toast(page, 'created in Engineering');
    const created = page
      .locator('.body .row')
      .filter({ hasText: 'phase7_browser_key' });
    await created.waitFor();
    await created.click();
    await page
      .locator('.details .dh h2', { hasText: 'phase7_browser_key' })
      .waitFor();
    const readableParties = page.locator('.details .party').filter({
      hasNotText: 'No read access',
    });
    if ((await readableParties.count()) !== Number(count?.[1])) {
      failures.push(
        'created group item did not retain its computed read-role count',
      );
    }
    await page.getByRole('button', { name: 'Edit', exact: true }).click();
    await page.locator('.details textarea').fill('edited browser value');
    await page
      .getByRole('button', { name: 'Save changes', exact: true })
      .click();
    await toast(page, 'Changes saved');
    // Verify the updated version number is rendered.
    await page.locator('.details', { hasText: 'Version2' }).waitFor();
    // The confirmation sheet adds a second "Delete", so each click is scoped.
    await page
      .locator('.details')
      .getByRole('button', { name: 'Delete', exact: true })
      .click();
    await page
      .locator('.sheet')
      .getByRole('button', { name: 'Delete', exact: true })
      .click();
    await toast(page, 'Deleted phase7_browser_key');
    await created.waitFor({ state: 'detached' });

    // A Document is either typed or brought in from disk; the File side of
    // that switch replaces the old dedicated Link and File item kinds.
    await page.goto(`${origin}/?state=group-new-document&view=list`, {
      waitUntil: 'load',
    });
    await page
      .getByRole('textbox', { name: 'Name', exact: true })
      .fill('emergency.pdf');
    await page
      .getByRole('group', { name: 'Document content' })
      .getByRole('button', { name: 'File', exact: true })
      .click();
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

async function folderWalk(context, origin) {
  const page = await context.newPage();
  const failures = [];
  page.on('pageerror', (error) => failures.push(`pageerror: ${error.message}`));
  page.on('console', (message) => {
    if (message.type() === 'error') failures.push(`console: ${message.text()}`);
  });
  try {
    await page.goto(`${origin}/?state=folders`, { waitUntil: 'load' });
    const personal = page
      .locator('.tpane .fn')
      .filter({ hasText: /^Personal$/ });
    await personal.locator('.fselect').click();
    await page
      .locator('.folder-body .row')
      .filter({ hasText: 'agents' })
      .click();
    await page.locator('.crumbs .cur', { hasText: 'agents' }).waitFor();
    await page
      .locator('.folder-body .row')
      .filter({ hasText: 'anthropic-api-key' })
      .waitFor();

    await page
      .locator('.toolbar')
      .getByRole('button', { name: 'New', exact: true })
      .click();
    await page.getByRole('menuitem', { name: 'Document' }).click();
    await page.getByRole('textbox', { name: 'Name' }).fill('FOLDER_TEST');
    await page.getByText('Advanced', { exact: true }).click();
    const path = await page.getByRole('textbox', { name: 'Path' }).inputValue();
    if (path !== '/agents/folder_test')
      failures.push(`new item path ignored the selected folder: ${path}`);
    await page.getByRole('button', { name: 'Cancel', exact: true }).click();

    await page.locator('.search input').fill('staging-token');
    // The tree is permanent navigation now; search flattens the list beside
    // it rather than hiding it.
    if (!(await page.locator('.tpane').count()))
      failures.push('the folder tree was hidden during search');
    const result = page
      .locator('.body .row')
      .filter({ hasText: 'staging-token' });
    await result.waitFor();
    const subtitle = (await result.locator('small').textContent()) ?? '';
    if (!subtitle.includes('Engineering · /deploy/staging-token'))
      failures.push(
        `search result did not show its full location: ${subtitle}`,
      );

    await page
      .locator('.toolbar')
      .getByRole('button', { name: 'New', exact: true })
      .click();
    await page.getByRole('menuitem', { name: 'Document' }).click();
    await page
      .getByRole('textbox', { name: 'Name' })
      .fill('SEARCH_FOLDER_TEST');
    await page.getByText('Advanced', { exact: true }).click();
    const retainedPath = await page
      .getByRole('textbox', { name: 'Path' })
      .inputValue();
    if (retainedPath !== '/agents/search_folder_test')
      failures.push(`search changed the selected save folder: ${retainedPath}`);
  } catch (error) {
    failures.push(
      `folder walk: ${error instanceof Error ? error.message : String(error)}`,
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
    await page.evaluate("window.localStorage.removeItem('foks.first-run.v2')");
    await page.reload({ waitUntil: 'load' });
    // Select joining path and proceed.
    await page.getByRole('radio', { name: /Join an existing team/ }).click();
    await page.getByRole('button', { name: 'Continue', exact: true }).click();
    await reloadAt('Select a server address');
    await page.getByRole('button', { name: 'Continue', exact: true }).click();
    await page.locator('.pane', { hasText: 'verified' }).waitFor();
    await reloadAt('verified');
    await page.getByRole('button', { name: 'Continue', exact: true }).click();
    await page.locator('.pane', { hasText: 'Set up your account' }).waitFor();
    await reloadAt('Set up your account');
    await page
      .getByRole('button', { name: 'Create my account', exact: true })
      .click();
    await page.locator('.pane', { hasText: 'Save recovery phrase' }).waitFor();
    await reloadAt('Save recovery phrase');

    await page
      .getByRole('button', {
        name: 'Generate a recovery phrase',
        exact: true,
      })
      .click();
    await page.locator('.sheet .word').first().waitFor();
    await page.reload({ waitUntil: 'load' });
    if (await page.locator('.sheet').count())
      failures.push('a prepared backup phrase survived reload');
    await page
      .getByRole('button', {
        name: 'Generate a recovery phrase',
        exact: true,
      })
      .click();
    await page.locator('.sheet .word').first().waitFor();
    await page.locator('.sheet .check').click();
    await page
      .locator('.sheet')
      .getByRole('button', { name: 'Done', exact: true })
      .click();
    await page
      .getByRole('button', {
        name: 'Generate a recovery phrase',
        exact: true,
      })
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
    await page.locator('.notice', { hasText: 'Joined Engineering' }).waitFor();
    // The mock bridge starts with fixture inventory on reload; the checkpoint
    // survives, but the group created during this run does not. The app must
    // retain the receipt and report that missing vault rather than invent it:
    // reconciliation drops back to the waiting pane, named for the team it
    // still remembers rather than a generic message.
    await reloadAt('Team vault unavailable');
    await page.locator('.main h1', { hasText: 'Engineering' }).waitFor();

    const checkpoint = await page.evaluate(
      "window.localStorage.getItem('foks.first-run.v2') ?? ''",
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
    // Verify pinned server status after initial check.
    await toast(page, 'Server identity pinned');
    await page.locator('.hostid').first().waitFor();

    // Verify host ID consistency across the tooltip, check response, and display.
    await page
      .getByRole('button', { name: 'Inspect last check response', exact: true })
      .click();
    const response = await page.locator('.main pre').first().textContent();
    const host = /"hostId":\s*"([0-9a-f]+)"/.exec(response ?? '')?.[1];
    if (!host) failures.push('Ade could not inspect the checked host id');
    else {
      if (!/^02[0-9a-f]{64}$/.test(host))
        failures.push('Ade did not receive the full canonical host id');
      const shown = page.locator('.hostid').first();
      if ((await shown.getAttribute('title')) !== host)
        failures.push('the host id shown to Ade is not the checked host id');
      const short = (await shown.textContent())?.trim() ?? '';
      const [head, tail] = short.split('\u2026');
      if (!head || !tail || !host.startsWith(head) || !host.endsWith(tail))
        failures.push(
          'the abbreviated host id does not abbreviate the real one',
        );
    }

    // The legacy reset scene now opens the unified local-removal dialog.
    await page.goto(`${origin}/?state=servers-reset`, { waitUntil: 'load' });
    await page
      .locator('.sheet', {
        hasText: 'Remove Personal server and its credentials?',
      })
      .waitFor();
    await page.locator('.sheet input').fill('personal');
    await page
      .getByRole('button', {
        name: 'Remove server and credentials',
        exact: true,
      })
      .click();
    await page.waitForSelector('.sheet', { state: 'detached' });
    await toast(page, 'Removed Personal server and its credentials');

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

  context.setDefaultTimeout(10000);

  let failed = 0;
  try {
    for (const state of process.argv.includes('--walks-only') ? [] : STATES) {
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

    for (const state of process.argv.includes('--walks-only')
      ? []
      : GROUP_ITEM_STATES) {
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

    const folderProblems = await folderWalk(context, site.origin);
    if (folderProblems.length) {
      failed += 1;
      console.error('FAIL folder browsing / create target walk');
      for (const problem of folderProblems) console.error(`  ${problem}`);
    } else {
      console.log('ok   folder browsing / create target walk');
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

    for (const path of process.argv.includes('--walks-only')
      ? []
      : ['invited', 'own']) {
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

if (process.argv[1] === fileURLToPath(import.meta.url)) await main();
