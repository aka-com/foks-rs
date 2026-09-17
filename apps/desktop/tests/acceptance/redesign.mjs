/* global document, innerWidth */
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';
import { serve } from './run.mjs';

const dist = fileURLToPath(new URL('../../dist/', import.meta.url));
const shots = fileURLToPath(new URL('./shots/', import.meta.url));
await mkdir(shots, { recursive: true });
const site = await serve(dist);
const browser = await chromium.launch({
  executablePath: process.env.FOKS_CHROMIUM ?? chromium.executablePath(),
  args: ['--no-sandbox'],
});

try {
  for (const width of [1280, 960]) {
    const context = await browser.newContext({
      viewport: { width, height: 860 },
    });
    const page = await context.newPage();
    page.setDefaultTimeout(10000);
    const errors = [];
    page.on('pageerror', (error) => errors.push(error.message));
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(message.text());
    });
    // At minimum width, use actual Tab traversal and keyboard activation.
    const focus = async (target) => {
      await target.waitFor({ state: 'visible' });
      for (let i = 0; i < 160; i++) {
        if (await target.evaluate((node) => node === document.activeElement))
          return;
        const key = await target.evaluate((node) => {
          if (node.closest('[role="menu"]')) return 'ArrowDown';
          const tabs = node.closest('[role="tablist"]');
          if (tabs && tabs.contains(document.activeElement))
            return 'ArrowRight';
          return 'Tab';
        });
        await page.keyboard.press(key);
      }
      assert.fail(`Control is not keyboard reachable: ${target}`);
    };
    const activate = async (target, key = 'Enter') => {
      if (width === 960) {
        await focus(target);
        await page.keyboard.press(key);
      } else await target.click();
    };
    const tab = (name) =>
      page
        .locator('.rail-tabs')
        .getByRole('button', { name: new RegExp(`^${name}(?:\\s|$)`) });
    const shot = async (name) => {
      assert.ok(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
        `${name} overflows at ${width}px`,
      );
      await page.screenshot({ path: `${shots}/redesign-${width}-${name}.png` });
    };
    try {
      await page.goto(`${site.origin}/?state=people`);
      await page
        .getByRole('heading', { name: 'Account', exact: true })
        .waitFor();
      await activate(page.locator('.rail .who'));
      await activate(
        page.getByRole('menu').getByRole('menuitem', { name: /vitalik/ }),
      );
      await activate(tab('Devices'));
      await page
        .getByRole('heading', { name: 'Devices', exact: true })
        .waitFor();
      assert.equal(new URL(page.url()).searchParams.get('store'), 'acct:work');
      assert.match(await page.locator('.rail .who').innerText(), /vitalik/);
      await shot('devices');

      await activate(tab('Settings'));
      const enable = page.getByRole('checkbox', {
        name: 'Enable desktop alerts on this device',
      });
      await activate(enable, 'Space');
      await activate(
        page.getByRole('checkbox', { name: 'Include message previews' }),
        'Space',
      );
      assert.ok(await enable.isChecked());
      await shot('settings');

      await activate(tab('Teams'));
      await activate(
        page.locator('.rowline > .row').filter({ hasText: 'Household' }),
      );
      await page.locator('.ghero', { hasText: 'Household' }).waitFor();
      assert.match(await page.locator('.rail .who').innerText(), /satoshi/);
      await activate(
        page.getByRole('button', {
          name: 'Invitations and requests',
          exact: true,
        }),
      );
      const invitations = page.getByRole('region', {
        name: 'Group invitations and requests',
      });
      await activate(
        invitations.getByRole('button', {
          name: 'Create invitation',
          exact: true,
        }),
      );
      await activate(
        invitations.getByRole('button', { name: 'Submit', exact: true }),
      );
      await invitations.getByLabel('Shareable invitation').waitFor();
      assert.match(
        await invitations.getByLabel('Shareable invitation').innerText(),
        /personal\/personal\/team:household/,
      );
      await activate(
        invitations.getByRole('button', {
          name: 'Refresh requests',
          exact: true,
        }),
      );
      const first = invitations
        .locator('article')
        .filter({ hasText: 'fixture-joiner' });
      await activate(
        first.getByRole('button', { name: 'Approve', exact: true }),
      );
      const second = invitations
        .locator('article')
        .filter({ hasText: 'fixture-second-joiner' });
      await activate(
        second.getByRole('button', { name: 'Reject', exact: true }),
      );
      await activate(
        invitations.getByRole('button', {
          name: 'Refresh requests',
          exact: true,
        }),
      );
      assert.equal(await invitations.locator('article').count(), 0);
      await shot('invitations');
      await activate(
        page.getByRole('button', {
          name: 'Invitations and requests',
          exact: true,
        }),
      );

      await activate(page.getByRole('tab', { name: /^Channels/ }));
      await activate(
        page.getByRole('button', {
          name: 'Open #general in Chat',
          exact: true,
        }),
      );
      await page
        .getByRole('textbox', { name: 'Message', exact: true })
        .waitFor();
      const conversation = new URL(page.url()).searchParams.get('channel');
      await shot('chat');
      await activate(
        page.getByRole('button', { name: 'Channel info', exact: true }),
      );
      await page
        .getByRole('complementary', { name: 'Channel info', exact: true })
        .waitFor();
      if (width === 960) {
        const pane = await page.locator('.chat-conversation').boundingBox();
        const info = await page.locator('.chat-info').boundingBox();
        assert.ok(
          pane &&
            info &&
            Math.abs(pane.x - info.x) <= 1 &&
            Math.abs(pane.width - info.width) <= 1,
          'narrow info panel must cover the conversation without squeezing it',
        );
      }
      await shot('chat-info');
      await activate(
        page.getByRole('button', {
          name: 'Device notification settings',
          exact: true,
        }),
      );
      assert.ok(
        await page
          .getByRole('checkbox', {
            name: 'Enable desktop alerts on this device',
          })
          .isChecked(),
      );
      await activate(tab('Chat'));
      assert.equal(
        new URL(page.url()).searchParams.get('channel'),
        conversation,
      );
      const closeInfo = page.getByRole('button', {
        name: 'Close channel info',
        exact: true,
      });
      if (await closeInfo.count()) await activate(closeInfo);
      await activate(
        page.getByRole('button', { name: 'Team files', exact: true }),
      );
      await page
        .getByRole('heading', { name: 'Household', exact: true })
        .waitFor();
      assert.equal(
        new URL(page.url()).searchParams.get('store'),
        'team:household',
      );
      await shot('files');
      await activate(tab('Teams'));
      await page.locator('.ghero', { hasText: 'Household' }).waitFor();
      await activate(
        page.getByRole('button', { name: 'Teams home', exact: true }),
      );
      await page.getByRole('heading', { name: 'Teams', exact: true }).waitFor();
      assert.equal(
        new URL(page.url()).searchParams.get('store'),
        'acct:personal',
      );
      await activate(tab('Files'));
      await page
        .getByRole('heading', { name: 'Household', exact: true })
        .waitFor();
      await activate(
        page.getByRole('button', { name: 'Files home', exact: true }),
      );
      await page.getByRole('heading', { name: 'Files', exact: true }).waitFor();
      await shot('roots');
      assert.deepEqual(errors, []);
      console.log(
        `ok redesign journeys at ${width}px${width === 960 ? ' (keyboard only)' : ''}`,
      );
    } catch (error) {
      await page.screenshot({ path: `${shots}/redesign-${width}-failure.png` });
      throw error;
    } finally {
      await context.close();
    }
  }
} finally {
  await browser.close();
  await site.close();
}
