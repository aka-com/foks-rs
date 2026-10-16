/**
 * The webview's `localStorage` records: one module touches storage, every
 * key is registered, and purging a profile removes the entries that name it
 * from each profile-scoped record while leaving other profiles' entries and
 * the device's preferences.
 */

import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { installDom } from './lib/dom-harness';
import { collectSourceFiles } from './lib/source';

installDom({ url: 'http://localhost/' });

const storage = await import('../src/platform/device-storage');
const labels =
  await import('../src/features/teams/invitations/invitation-labels');
const recovery = await import('../src/first-run-recovery');
const operations = await import('../src/first-run-operations');
const firstRun = await import('../src/first-run-state');
const lists = await import('../src/recent-favorites');
const reviews = await import('../src/features/teams/exposure-review-model');
const { rememberFilesView, storedFilesView } =
  await import('../src/features/files/files-view-pref');

const { STORAGE_KEYS, purgeProfiles } = storage;

test.afterEach(() => window.localStorage.clear());

test('only the device-storage module touches webview storage', async () => {
  const owner = fileURLToPath(
    new URL('../src/platform/device-storage.ts', import.meta.url),
  );
  const access =
    /\b(?:localStorage|sessionStorage)\s*\.|\.\s*(?:localStorage|sessionStorage)\b/;
  const offenders: string[] = [];
  for (const url of await collectSourceFiles(
    new URL('../src/', import.meta.url),
  )) {
    const path = fileURLToPath(url);
    if (path === owner) continue;
    if (access.test(await readFile(url, 'utf8'))) offenders.push(path);
  }
  assert.deepEqual(offenders, []);
});

test('theme-init.js reads the registered appearance key', async () => {
  const source = await readFile(
    new URL('../public/theme-init.js', import.meta.url),
    'utf8',
  );
  const keys = [...source.matchAll(/getItem\('([^']+)'\)/g)].map(
    (match) => match[1],
  );
  assert.deepEqual(keys, [STORAGE_KEYS.appearance.name]);
});

test('every registered key has its own name', () => {
  const names = Object.values(STORAGE_KEYS).map((key) => key.name);
  assert.equal(new Set(names).size, names.length);
});

function storeRef(profile: string, account: string): string {
  // The shape `store_id` in src-tauri/src/commands/vault.rs writes.
  return JSON.stringify({
    kind: 'account',
    profile,
    accountAlias: account,
  });
}

function checkpoint(
  profile: string,
  id: string,
): import('../src/first-run-state').FirstRunCheckpoint {
  return {
    ...firstRun.initialFirstRun('own', 'operation-pending'),
    profile: {
      profile,
      hostId: `02${'2'.repeat(64)}`,
      lookupName: 'localhost',
      canonicalName: 'localhost',
      acceptance: 'unchanged',
      chain: 1,
      epoch: 1,
    },
    serverAddress: 'localhost:4430',
    provisioning: {
      id,
      kind: 'signup',
      alias: 'main',
      deviceName: 'Mac',
      back: 'account',
    },
  };
}

function seed(): void {
  for (const profile of ['personal', 'work']) {
    labels.rememberInvitationLabel(profile, 'main', 'op', {
      label: `${profile} invitee`,
      role: 'member',
      created: 1,
    });
    recovery.retainSetup(checkpoint(profile, `attempt-${profile}`));
    lists.rememberRecent({ store: storeRef(profile, 'main'), dirent: 'd1' });
    lists.toggleFavorite({ store: storeRef(profile, 'main'), dirent: 'd2' });
    reviews.recordExposure({
      scope: storeRef(profile, 'main'),
      party: 'device-1',
      name: 'Old laptop',
      cause: 'device-removed',
      entries: [{ store: storeRef(profile, 'main'), dirent: 'd1', version: 1 }],
    });
  }
  operations.persistFirstRun(checkpoint('personal', 'attempt-current'));
  rememberFilesView('grid');
}

test('purging a profile removes its entries and keeps the others', () => {
  seed();
  const notified: string[] = [];
  const unsubscribe = [
    lists.subscribeRecentFavorites(() => notified.push('recent-favorites')),
    reviews.subscribeExposureReviews(() => notified.push('exposure-reviews')),
  ];

  purgeProfiles(['personal']);
  for (const stop of unsubscribe) stop();

  assert.equal(labels.invitationLabel('personal', 'main', 'op'), undefined);
  assert.equal(
    labels.invitationLabel('work', 'main', 'op')?.label,
    'work invitee',
  );
  assert.deepEqual(
    recovery.retainedSetups().map((entry) => entry.id),
    ['attempt-work'],
  );
  assert.equal(operations.storedFirstRunCheckpoint(), null);
  assert.deepEqual(lists.recentFavorites(), {
    recent: [{ store: storeRef('work', 'main'), dirent: 'd1' }],
    favorites: [{ store: storeRef('work', 'main'), dirent: 'd2' }],
  });
  assert.deepEqual(
    reviews.exposureReviews().map((review) => review.scope),
    [storeRef('work', 'main')],
  );
  // Device preferences are not tied to a profile.
  assert.equal(storedFilesView(), 'grid');
  // Subscribed views re-read the purged records.
  assert.deepEqual(notified.sort(), ['exposure-reviews', 'recent-favorites']);
});

test('purging every profile with entries removes the profile records', () => {
  seed();
  purgeProfiles(['personal', 'work']);
  for (const key of Object.values(STORAGE_KEYS))
    if (key.scope === 'profile')
      assert.equal(window.localStorage.getItem(key.name), null, key.name);
  assert.equal(storedFilesView(), 'grid');
});

test('a first-run checkpoint for another profile, or none, is kept', () => {
  operations.persistFirstRun(checkpoint('work', 'attempt-current'));
  purgeProfiles(['personal']);
  assert.equal(operations.storedFirstRunCheckpoint()?.profile?.profile, 'work');

  const unbound = firstRun.initialFirstRun('own');
  operations.persistFirstRun(unbound);
  purgeProfiles(['personal']);
  assert.ok(operations.storedFirstRunCheckpoint());
});

test('entries whose store ref names no profile are kept', () => {
  // The mock bridge's refs carry no profile, so nothing can attribute them.
  lists.rememberRecent({ store: 'acct:personal', dirent: 'd1' });
  purgeProfiles(['personal']);
  assert.deepEqual(lists.recentFavorites().recent, [
    { store: 'acct:personal', dirent: 'd1' },
  ]);
});

test('a profile record that cannot be parsed is removed', () => {
  window.localStorage.setItem(STORAGE_KEYS.setupRecovery.name, '{not json');
  window.localStorage.setItem(STORAGE_KEYS.invitationLabels.name, '[]');
  purgeProfiles(['personal']);
  assert.equal(
    window.localStorage.getItem(STORAGE_KEYS.setupRecovery.name),
    null,
  );
  assert.equal(
    window.localStorage.getItem(STORAGE_KEYS.invitationLabels.name),
    null,
  );
  assert.deepEqual(recovery.retainedSetups(), []);
});

test('reads and writes report unavailable storage instead of throwing', () => {
  const original = Object.getOwnPropertyDescriptor(window, 'localStorage');
  Object.defineProperty(window, 'localStorage', {
    configurable: true,
    get() {
      throw new Error('Storage is blocked');
    },
  });
  try {
    assert.equal(storage.readStorage(STORAGE_KEYS.filesView), null);
    assert.equal(storage.writeStorage(STORAGE_KEYS.filesView, 'grid'), false);
    assert.throws(
      () => storage.writeStorageOrThrow(STORAGE_KEYS.firstRun, '{}'),
      /blocked/,
    );
    assert.throws(() => recovery.retainedSetups(), /blocked/);
    purgeProfiles(['personal']);
  } finally {
    if (original) Object.defineProperty(window, 'localStorage', original);
  }
});
