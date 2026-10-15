/**
 * The attention centre's model: one item per entity behind each rail signal,
 * each naming the page that resolves it, and the "Get set up" steps, each
 * left out until its fact has been read.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { attentionItems, setupChecklist } from '../src/attention';
import { FIXTURE } from '../src/mock/fixture';
import type { AgentSnapshot, Notification } from '../src/model';
import type { TeamRequestCount } from '../src/data/operation-queries';
import { DeviceAlertRegistry } from '../src/data/device-alert';

const NOW = Date.UTC(2026, 9, 11);
const BACKUP = {
  backupAlias: 'paper-backup',
  accountAlias: 'work',
  backupId: `10${'4'.repeat(64)}`,
};
const COMPUTER = (current: boolean, digit: string) => ({
  id: `04${digit.repeat(64)}`,
  role: 'owner' as const,
  current,
});
const CARD = (current: boolean) => ({
  id: `08${'5'.repeat(64)}`,
  role: 'member' as const,
  current,
});

/** The fixture without its unverified server, so no server is flagged. */
const QUIET: AgentSnapshot = {
  ...FIXTURE,
  servers: FIXTURE.servers.filter((server) => server.profileName !== 'partner'),
};

test('each rail signal becomes one item per entity, warnings first, each linked to its page', () => {
  const registry = new DeviceAlertRegistry();
  registry.reportRecoveryPhrase('acct:personal', []);
  registry.reportRecoveryPhrase('acct:work', [BACKUP]);
  registry.reportPairingOffer('acct:work', true);
  const teamRequests = new Map<string, TeamRequestCount>([
    ['team:eng', 2],
    ['team:household', '20+'],
    // An ad hoc team has no requests page; the Teams badge skips it too.
    ['team:homelab', 4],
  ]);
  const pendingMembership = new Map([
    ['team:eng', 1],
    ['team:household', 2],
  ]);
  const notice: Notification = {
    id: 'catalog-personal',
    severity: 'warn',
    title: 'Vault list could not be read',
    detail: 'Retry to read it again.',
    action: 'Retry',
  };
  const items = attentionItems({
    snapshot: FIXTURE,
    teamRequests,
    pendingMembership,
    devices: registry.getSnapshot(),
    phraseTests: {},
    notices: [notice],
    now: NOW,
  });
  assert.deepEqual(
    items.map((item) => [item.id, item.severity, item.title, item.detail]),
    [
      ['server:partner', 'warning', 'Server not verified', 'foks.partner.dev'],
      [
        'no-recovery-phrase:acct:personal',
        'warning',
        'No recovery phrase',
        'satoshi on Personal server',
      ],
      [
        'team-membership:team:eng',
        'warning',
        '1 incomplete membership change in Engineering',
        'It blocks other membership changes on this team.',
      ],
      [
        'team-membership:team:household',
        'warning',
        '2 incomplete membership changes in Household',
        'They block other membership changes on this team.',
      ],
      [
        'notice:catalog-personal',
        'warning',
        'Vault list could not be read',
        'Retry to read it again.',
      ],
      [
        'pairing-offer:acct:work',
        'info',
        'A pairing offer is open',
        'vitalik on Acme',
      ],
      [
        `recovery-phrase-test:${BACKUP.backupId}`,
        'info',
        'Recovery phrase paper-backup has never been tested',
        'vitalik on Acme',
      ],
      [
        'team-requests:team:eng',
        'info',
        '2 requests to join Engineering',
        undefined,
      ],
      [
        'team-requests:team:household',
        'info',
        'At least 20 requests to join Household',
        undefined,
      ],
    ],
  );
  assert.deepEqual(
    items.map((item) => item.location),
    [
      { kind: 'settings', section: 'account', profile: 'partner' },
      { kind: 'devices', store: 'acct:personal' },
      { kind: 'group-settings', ref: 'team:eng' },
      { kind: 'group-settings', ref: 'team:household' },
      { kind: 'settings', section: 'account' },
      { kind: 'devices', store: 'acct:work' },
      { kind: 'devices', store: 'acct:work' },
      { kind: 'group-settings', ref: 'team:eng', tab: 'requests' },
      { kind: 'group-settings', ref: 'team:household', tab: 'requests' },
    ],
  );
});

test('with nothing flagged and nothing read there is nothing to list', () => {
  assert.deepEqual(
    attentionItems({
      snapshot: QUIET,
      teamRequests: new Map([['team:eng', 0]]),
      pendingMembership: new Map(),
      devices: new DeviceAlertRegistry().getSnapshot(),
      phraseTests: {},
      notices: [],
      now: NOW,
    }),
    [],
  );
});

test('a setup step is left out until its fact has been read', () => {
  const steps = setupChecklist({
    snapshot: FIXTURE,
    devices: new DeviceAlertRegistry().getSnapshot(),
    catalogReady: false,
  });
  // The fixture already holds passwords and teams; the device facts and the
  // lock state have not been read.
  assert.deepEqual(
    steps.map((step) => [step.id, step.done]),
    [
      ['first-password', true],
      ['team', true],
    ],
  );
});

test('the checklist names the account a step is missing on and links to where it is done', () => {
  const registry = new DeviceAlertRegistry();
  registry.reportRecoveryPhrase('acct:personal', [
    { ...BACKUP, accountAlias: 'personal' },
  ]);
  registry.reportRecoveryPhrase('acct:work', []);
  registry.reportAccountKeys('acct:personal', [
    COMPUTER(true, '1'),
    CARD(false),
  ]);
  registry.reportAccountKeys('acct:work', [COMPUTER(true, '2')]);
  const empty: AgentSnapshot = {
    ...FIXTURE,
    items: [],
    stores: FIXTURE.stores.filter((store) => store.kind === 'account'),
  };
  const steps = setupChecklist({
    snapshot: empty,
    devices: registry.getSnapshot(),
    appLock: { available: false, unavailableReason: 'polkit is not installed' },
    catalogReady: true,
  });
  assert.deepEqual(
    steps.map((step) => [step.id, step.done, step.location]),
    [
      [
        'recovery-phrase',
        false,
        { kind: 'devices', store: 'acct:work', open: 'phrase' },
      ],
      [
        'second-key',
        false,
        { kind: 'devices', store: 'acct:work', open: 'add' },
      ],
      // Nothing in the app makes the lock available, so the step has no link.
      ['app-lock', false, undefined],
      ['first-password', false, { kind: 'files' }],
      ['team', false, { kind: 'teams' }],
    ],
  );
  assert.match(steps[0].detail, /vitalik on Acme/);
  assert.match(steps[1].detail, /vitalik on Acme has one key/);
  assert.equal(steps[2].detail, 'polkit is not installed');
  // This computer cannot have the lock, so the step is listed and not open.
  assert.equal(steps[2].unavailable, true);
  assert.equal(steps[3].detail, 'Add a password to a vault.');
});

test('the password step counts only logins, as its text says', () => {
  const login = FIXTURE.items.find((item) => item.shape === 'login');
  const other = FIXTURE.items.find(
    (item) => item.kind === 'Secret' && item.shape === 'other',
  );
  assert.ok(login && other);
  const step = (items: AgentSnapshot['items']) =>
    setupChecklist({
      snapshot: { ...FIXTURE, items },
      devices: new DeviceAlertRegistry().getSnapshot(),
      catalogReady: true,
    }).find((entry) => entry.id === 'first-password');
  assert.equal(step([other])?.done, false);
  assert.equal(step([other, login])?.done, true);
});

test('an account whose one key is a hardware card still needs a second key', () => {
  const registry = new DeviceAlertRegistry();
  registry.reportAccountKeys('acct:personal', [
    COMPUTER(true, '1'),
    COMPUTER(false, '3'),
  ]);
  registry.reportAccountKeys('acct:work', [CARD(true)]);
  const step = setupChecklist({
    snapshot: FIXTURE,
    devices: registry.getSnapshot(),
    catalogReady: true,
  }).find((entry) => entry.id === 'second-key');
  assert.equal(step?.done, false);
  assert.match(step?.detail ?? '', /vitalik on Acme has one key/);
});

test('a per-account step is done only once every account has been read', () => {
  const registry = new DeviceAlertRegistry();
  registry.reportRecoveryPhrase('acct:personal', [BACKUP]);
  registry.reportAccountKeys('acct:personal', [
    COMPUTER(true, '1'),
    COMPUTER(false, '3'),
  ]);
  const ids = (devices: DeviceAlertRegistry) =>
    setupChecklist({
      snapshot: FIXTURE,
      devices: devices.getSnapshot(),
      catalogReady: true,
    }).map((step) => step.id);
  // The work account's keys are unknown, so neither step can be called done.
  assert.deepEqual(ids(registry), ['first-password', 'team']);
  registry.reportRecoveryPhrase('acct:work', [BACKUP]);
  registry.reportAccountKeys('acct:work', [COMPUTER(true, '2'), CARD(false)]);
  assert.deepEqual(ids(registry), [
    'recovery-phrase',
    'second-key',
    'first-password',
    'team',
  ]);
});

test('a second computer or a hardware key completes the second-key step', () => {
  const registry = new DeviceAlertRegistry();
  registry.reportRecoveryPhrase('acct:personal', [BACKUP]);
  registry.reportRecoveryPhrase('acct:work', [BACKUP]);
  registry.reportAccountKeys('acct:personal', [
    COMPUTER(true, '1'),
    COMPUTER(false, '3'),
  ]);
  registry.reportAccountKeys('acct:work', [COMPUTER(true, '2'), CARD(false)]);
  const steps = setupChecklist({
    snapshot: FIXTURE,
    devices: registry.getSnapshot(),
    appLock: { available: true },
    catalogReady: true,
  });
  assert.deepEqual(
    steps.map((step) => [step.id, step.done]),
    [
      ['recovery-phrase', true],
      ['second-key', true],
      ['app-lock', true],
      ['first-password', true],
      ['team', true],
    ],
  );
  assert.ok(steps.every((step) => step.location === undefined));

  // Forgetting an account's keys drops it from the step, as from the dot.
  registry.forgetAccountKeys('acct:personal');
  registry.forgetAccountKeys('acct:work');
  assert.equal(
    setupChecklist({
      snapshot: FIXTURE,
      devices: registry.getSnapshot(),
      catalogReady: true,
    }).some((step) => step.id === 'second-key'),
    false,
  );
});
