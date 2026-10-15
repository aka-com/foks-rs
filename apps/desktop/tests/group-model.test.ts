/**
 * The group page's roster order and the reasons a member's menu gives for an
 * action it cannot take, from src/model/group-model.ts. Loaded in node with
 * no DOM.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { FIXTURE } from '../src/fixture';
import { partiesOf, partyName } from '../src/model';
import type { Party, TeamStore } from '../src/model';
import { sortRoster, targetReason } from '../src/model/group-model';

function engineering(): TeamStore {
  const store = FIXTURE.stores.find((entry) => entry.id === 'team:eng');
  assert.ok(store?.kind === 'team');
  return store;
}

function party(name: string): Party {
  const found = partiesOf(FIXTURE, 'team:eng').find(
    (entry) => partyName(entry) === name,
  );
  assert.ok(found, name);
  return found;
}

test('a roster lists higher roles first and the reader first within a role', () => {
  const sorted = sortRoster([
    party('dana.okafor'),
    party('priya.n'),
    party('vitalik'),
    party('sam.ortiz'),
  ]).map(partyName);
  assert.deepEqual(sorted, ['sam.ortiz', 'vitalik', 'priya.n', 'dana.okafor']);
});

test('a member menu says why an action is unavailable', () => {
  const store = engineering();
  assert.equal(
    targetReason(FIXTURE, store, party('dana.okafor'), false),
    'Only an Admin or an Owner can change this team’s members.',
  );
  assert.equal(
    targetReason(FIXTURE, store, party('vitalik'), true),
    'You cannot change your own role or remove your own account.',
  );
  assert.match(
    targetReason(FIXTURE, store, party('homelab @ foks.example.net'), true),
    /managed on their own server/,
  );
  assert.equal(
    targetReason(FIXTURE, store, party('sam.ortiz'), true),
    'An Admin cannot change another Admin or the Owner.',
  );
  assert.equal(
    targetReason(FIXTURE, store, party('dana.okafor'), true),
    'This member cannot be changed from this device.',
  );
  assert.equal(
    targetReason(
      FIXTURE,
      { ...store, team_kind: 'adhoc' },
      party('dana.okafor'),
      true,
    ),
    'Memberships can’t be changed in an ad-hoc team.',
  );
});
