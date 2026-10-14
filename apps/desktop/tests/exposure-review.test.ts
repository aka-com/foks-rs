/**
 * The exposure review model: which items a removed member, a lowered member
 * or a removed device could read, how an entry's state follows the live
 * item's version, and how reviews are stored, merged, marked and removed
 * without item names or paths.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { installDom } from './lib/dom-harness';

installDom({ url: 'http://localhost/' });

const exposure = await import('../src/exposure-review');
const { FIXTURE } = await import('../src/fixture');
type AgentSnapshot = import('../src/model').AgentSnapshot;
type Party = import('../src/model').Party;

test.afterEach(() => window.localStorage.clear());

function party(store: string, username: string): Party {
  const found = FIXTURE.parties.find(
    (candidate) => candidate.store === store && candidate.username === username,
  );
  assert.ok(found, `no party ${username} in ${store}`);
  return found;
}

const paths = (items: readonly { store: string; path: string }[]) =>
  items.map((item) => `${item.store}|${item.path}`);

test('a member reads the team items its role admits, Passwords first', () => {
  const sam = party('team:household', 'sam');
  assert.deepEqual(
    paths(exposure.exposureOrder(exposure.memberReads(FIXTURE, sam))),
    [
      'team:household|/streaming/netflix',
      'team:household|/wifi/guest-password',
      'team:household|/documents/emergency.pdf',
    ],
  );
  // A Member at visibility 0 does not read the Admin-only token.
  const dana = party('team:eng', 'dana.okafor');
  assert.equal(
    paths(exposure.memberReads(FIXTURE, dana)).includes(
      'team:eng|/deploy/production-token',
    ),
    false,
  );
});

test('a lower role lists only the items it no longer admits', () => {
  const priya = party('team:eng', 'priya.n');
  // Admin to Member 0: only the Admin-read token is lost.
  assert.deepEqual(
    paths(
      exposure.loweredReads(FIXTURE, priya, { role: 'Member', visibility: 0 }),
    ),
    ['team:eng|/deploy/production-token'],
  );
  const dana = party('team:eng', 'dana.okafor');
  assert.deepEqual(
    paths(
      exposure.exposureOrder(
        exposure.loweredReads(FIXTURE, dana, {
          role: 'Member',
          visibility: -1,
        }),
      ),
    ),
    [
      'team:eng|/deploy/staging-token',
      'team:eng|/onboarding/README.md',
      'team:eng|/release/bundle.tar',
    ],
  );
});

test('a device reads every item in its account and the account’s teams', () => {
  const items = exposure.accountReads(FIXTURE, {
    server: 'personal',
    account: 'personal',
  });
  const stores = new Set(items.map((item) => item.store));
  assert.deepEqual([...stores].sort(), ['acct:personal', 'team:household']);
  assert.equal(items.length, 9);
  assert.equal(
    exposure.accountExposureReadiness(FIXTURE, {
      server: 'personal',
      account: 'personal',
    }),
    'ready',
  );
});

test('the list waits while a store’s items are still being read', () => {
  const loading: AgentSnapshot = {
    ...FIXTURE,
    storeInventory: FIXTURE.storeInventory.map((entry) =>
      entry.store === 'team:household'
        ? { ...entry, status: 'loading' as const }
        : entry,
    ),
  };
  assert.equal(
    exposure.memberExposureReadiness(loading, 'team:household'),
    'loading',
  );
  assert.equal(
    exposure.accountExposureReadiness(loading, {
      server: 'personal',
      account: 'personal',
    }),
    'loading',
  );
  assert.equal(exposure.memberExposureReadiness(loading, 'team:eng'), 'ready');
});

test('an entry is changed once its item is written, and missing once it moves', () => {
  const sam = party('team:household', 'sam');
  const [netflix] = exposure.entriesOf(
    exposure.exposureOrder(exposure.memberReads(FIXTURE, sam)),
  );
  assert.ok(netflix);
  assert.equal(exposure.entryState(FIXTURE, netflix), 'unchanged');
  const written: AgentSnapshot = {
    ...FIXTURE,
    items: FIXTURE.items.map((item) =>
      item.direntId === netflix.dirent
        ? { ...item, version: item.version + 1 }
        : item,
    ),
  };
  assert.equal(exposure.entryState(written, netflix), 'changed');
  const moved: AgentSnapshot = {
    ...FIXTURE,
    items: FIXTURE.items.map((item) =>
      item.direntId === netflix.dirent
        ? { ...item, direntId: 'ff'.repeat(16), path: '/streaming/old' }
        : item,
    ),
  };
  assert.equal(exposure.entryState(moved, netflix), 'missing');
  // A mark is the user's answer, whatever the item's version says.
  assert.equal(
    exposure.entryState(written, { ...netflix, mark: 'not-needed' }),
    'not-needed',
  );
});

test('a review stores ids and versions, never item names or paths', () => {
  const sam = party('team:household', 'sam');
  const items = exposure.memberReads(FIXTURE, sam);
  assert.equal(
    exposure.recordExposure(
      {
        scope: 'team:household',
        party: sam.party_id_hex,
        name: 'sam',
        cause: 'removed',
        entries: exposure.entriesOf(items),
      },
      1_000,
    ),
    true,
  );
  const raw = window.localStorage.getItem('foks.exposureReviews') ?? '';
  for (const item of items) {
    assert.equal(raw.includes(item.path), false);
    assert.equal(raw.includes(item.path.split('/').pop() ?? ''), false);
  }
  const [review] = exposure.exposureReviews();
  assert.ok(review);
  assert.equal(review.id, `team:household|${sam.party_id_hex}`);
  assert.equal(review.entries.length, 3);
  assert.equal(exposure.openEntries(review), 3);
});

test('an empty list keeps no review', () => {
  assert.equal(
    exposure.recordExposure({
      scope: 'team:eng',
      party: 'p',
      name: 'p',
      cause: 'lowered',
      entries: [],
    }),
    true,
  );
  assert.deepEqual(exposure.exposureReviews(), []);
});

test('a later change for the same party keeps earlier versions and marks', () => {
  const base = {
    scope: 'team:eng',
    party: 'p1',
    name: 'dana',
  };
  exposure.recordExposure({
    ...base,
    cause: 'lowered',
    entries: [{ store: 'team:eng', dirent: 'a', version: 3 }],
  });
  const id = exposure.reviewId('team:eng', 'p1');
  exposure.markExposure(id, { store: 'team:eng', dirent: 'a' }, 'rotated');
  exposure.recordExposure({
    ...base,
    cause: 'removed',
    entries: [
      { store: 'team:eng', dirent: 'a', version: 5 },
      { store: 'team:eng', dirent: 'b', version: 1 },
    ],
  });
  const reviews = exposure.exposureReviews();
  assert.equal(reviews.length, 1);
  assert.equal(reviews[0]?.cause, 'removed');
  assert.deepEqual(reviews[0]?.entries, [
    { store: 'team:eng', dirent: 'a', version: 3, mark: 'rotated' },
    { store: 'team:eng', dirent: 'b', version: 1 },
  ]);
  exposure.markExposure(id, { store: 'team:eng', dirent: 'a' }, null);
  assert.equal(exposure.exposureReviews()[0]?.entries[0]?.mark, undefined);
  exposure.dismissExposure(id);
  assert.deepEqual(exposure.exposureReviews(), []);
});

test('malformed stored reviews and entries are dropped', () => {
  window.localStorage.setItem(
    'foks.exposureReviews',
    JSON.stringify([
      { scope: 'team:eng', party: 'p', name: 'x', cause: 'unknown', at: 1 },
      {
        scope: 'team:eng',
        party: 'q',
        name: 'y',
        cause: 'removed',
        at: 2,
        entries: [
          { store: 'team:eng', dirent: 'a', version: 1, mark: 'bogus' },
          { store: 'team:eng', dirent: 'b' },
          'junk',
        ],
      },
    ]),
  );
  const reviews = exposure.exposureReviews();
  assert.equal(reviews.length, 1);
  assert.deepEqual(reviews[0]?.entries, [
    { store: 'team:eng', dirent: 'a', version: 1 },
  ]);
  window.localStorage.setItem('foks.exposureReviews', '{not json');
  assert.deepEqual(exposure.exposureReviews(), []);
});
