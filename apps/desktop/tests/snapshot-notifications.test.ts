import assert from 'node:assert/strict';
import test from 'node:test';

import { notificationsOf } from '../src/bridge/snapshot-notifications';
import type { CatalogDto } from '../src/bridge/vault-catalog';

const error = {
  code: 'io',
  message: 'Could not reach the server.',
  fatal: false,
  retryable: false,
  ambiguous: false,
};

test('exact duplicate catalog failures produce one notice', () => {
  const catalog: CatalogDto = {
    profiles: ['foks-app'],
    stores: [],
    knownStores: [],
    inventory: [],
    items: [],
    blockedProfiles: [],
    failures: [
      { scope: 'store', profile: 'foks-app', store: 'one', error },
      { scope: 'store', profile: 'foks-app', store: 'two', error },
    ],
  };
  const notes = notificationsOf(catalog, [], 0);
  assert.equal(notes.length, 1);
  assert.equal(notes[0].title, 'Could not load vault on foks-app');
});

test('catalog failures with different details remain separate', () => {
  const catalog: CatalogDto = {
    profiles: ['foks-app'],
    stores: [],
    knownStores: [],
    inventory: [],
    items: [],
    blockedProfiles: [],
    failures: [
      { scope: 'store', profile: 'foks-app', store: 'one', error },
      {
        scope: 'store',
        profile: 'foks-app',
        store: 'two',
        error: { ...error, message: 'A different vault failure.' },
      },
    ],
  };
  assert.equal(notificationsOf(catalog, [], 0).length, 2);
});
