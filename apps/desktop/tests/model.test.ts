/**
 * Unit tests for domain models, role arithmetic, store permissions, and formatting.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { FIXTURE } from '../src/fixture';
import {
  admissionActive,
  storeDescription,
  storeDescriptionState,
  admits,
  applyLease,
  catalog,
  canCreateInStore,
  canChangeItem,
  fmtSize,
  formatRole,
  HUES,
  hue,
  initials,
  isLogin,
  itemKey,
  kindOf,
  leaseLapsed,
  nameOf,
  parseRole,
  partiesOf,
  partyName,
  peopleGroups,
  prefixOf,
  profileInventoryComplete,
  readersOf,
  roleRank,
  serverDisplayName,
  serverName,
  rtype,
  safestRemovalTarget,
  storeReadable,
  storeAvailability,
  storeAttentionState,
  storeDisplayOrder,
  storeHues,
  storeNavigationOrder,
  teamCaption,
} from '../src/model';
import type { Item, Store, AgentSnapshot } from '../src/model';

function item(snapshot: AgentSnapshot, key: string): Item {
  const found = snapshot.items.find((candidate) => itemKey(candidate) === key);
  assert.ok(found, `item not found in fixture: ${key}`);
  return found;
}

function readerCount(snapshot: AgentSnapshot, key: string): number {
  const readers = readersOf(snapshot, item(snapshot, key));
  assert.ok(readers, `item must belong to a team store: ${key}`);
  return readers.length;
}

test('server display names prefer labels and fall back to profile names', () => {
  assert.equal(
    serverDisplayName({ name: 'setup-foks-app-4430', label: 'FOKS' }),
    'FOKS',
  );
  assert.equal(
    serverDisplayName({ name: 'setup-foks-app-4430', label: null }),
    'setup-foks-app-4430',
  );
});

test('duplicate server labels include the profile name in store contexts', () => {
  const snapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server, index) =>
      index < 2 ? { ...server, label: 'Shared' } : server,
    ),
  };
  assert.equal(
    serverName(snapshot, { server: 'personal' }),
    'Shared · foks.example.net',
  );
  assert.equal(
    serverName(snapshot, { server: 'acme' }),
    'Shared · foks.acme-corp.com',
  );
});
/* ------------------------------------------------------------------ roles -- */

test('roleRank orders Member with visibility below Admin and Owner', () => {
  assert.equal(roleRank('Member · visibility -16384'), 1);
  assert.equal(roleRank('Member'), 1);
  assert.equal(roleRank('Admin'), 2);
  assert.equal(roleRank('Owner'), 3);

  // Visibility breaks ties within the Member role, but does not affect comparisons across different roles.
  assert.equal(
    admits('Member · visibility 0', 'Member · visibility -16384'),
    true,
  );
  assert.equal(
    admits('Member · visibility -16384', 'Member · visibility 0'),
    false,
  );
  assert.equal(admits('Admin', 'Member · visibility 0'), true);
  assert.equal(admits('Member · visibility 0', 'Admin'), false);
  assert.equal(admits('Owner', 'Admin'), true);
  assert.equal(admits('Admin', 'Owner'), false);
  assert.equal(admits('Admin', 'Admin'), true);
  assert.equal(admits('Owner', 'Owner'), true);

  // Unparseable roles must fail closed on both candidate and required roles.
  assert.equal(admits('Member · visibility 0', 'Reader'), false);
  assert.equal(admits('Member · visibility -32768', 'Superuser'), false);
  assert.equal(admits('Owner', 'Reader'), false);
  assert.equal(admits('Reader', 'Owner'), false);
  assert.equal(admits('Reader', 'Reader'), false);
});

test('admits supports structured role objects with visibility', () => {
  assert.equal(
    admits(
      { role: 'Member', visibility: 0 },
      { role: 'Member', visibility: -16384 },
    ),
    true,
  );
  assert.equal(
    admits(
      { role: 'Member', visibility: -16384 },
      { role: 'Member', visibility: 0 },
    ),
    false,
  );
});

test('parseRole accepts string and object role formats and rejects invalid roles', () => {
  assert.deepEqual(parseRole('Owner'), { kind: 'owner' });
  assert.deepEqual(parseRole('Admin'), { kind: 'admin' });
  assert.deepEqual(parseRole('Member'), { kind: 'member', visibility: 0 });
  assert.deepEqual(parseRole('Member · visibility -16384'), {
    kind: 'member',
    visibility: -16384,
  });
  assert.deepEqual(parseRole({ role: 'Member' }), {
    kind: 'member',
    visibility: 0,
  });
  assert.deepEqual(parseRole({ role: 'Owner' }), { kind: 'owner' });

  // Ensure standard non-FOKS role names are rejected.
  for (const invented of ['Reader', 'Manager', 'Viewer', 'Editor']) {
    assert.equal(parseRole(invented), null, invented);
    assert.equal(roleRank(invented), 0, invented);
  }
});

test('formatRole formats role objects into canonical display strings', () => {
  assert.equal(formatRole({ kind: 'owner' }), 'Owner');
  assert.equal(formatRole({ kind: 'admin' }), 'Admin');
  assert.equal(
    formatRole({ kind: 'member', visibility: 0 }),
    'Member · visibility 0',
  );
  assert.equal(
    formatRole({ kind: 'member', visibility: -16384 }),
    'Member · visibility -16384',
  );
});

/* ---------------------------------------------------------------- readers -- */

test('readerCount calculates eligible readers from item role and roster', () => {
  // production-token requires Admin: sam.ortiz (Owner), vitalik and priya.n
  // (Admin). dana.okafor and deploy-bot are Members; homelab is excluded.
  assert.equal(readerCount(FIXTURE, 'team:eng|/deploy/production-token'), 3);

  // staging-token and bundle.tar require Member · visibility 0: the same three
  // plus dana.okafor and deploy-bot — five. Homelab holds Member · visibility 0
  // and would qualify, but its admission reports inactive.
  assert.equal(readerCount(FIXTURE, 'team:eng|/deploy/staging-token'), 5);
  assert.equal(readerCount(FIXTURE, 'team:eng|/release/bundle.tar'), 5);
  assert.equal(readerCount(FIXTURE, 'team:eng|/onboarding/README.md'), 5);

  // Household has two parties and both have Member · visibility 0.
  assert.equal(readerCount(FIXTURE, 'team:household|/wifi/guest-password'), 2);
  assert.equal(
    readerCount(FIXTURE, 'team:household|/documents/emergency.pdf'),
    2,
  );
});

test('activating a federated team admission grants item read access', () => {
  const homelab = partiesOf(FIXTURE, 'team:eng').find(
    (party) => party.party_kind === 'named-team',
  );
  assert.ok(homelab);
  assert.equal(admissionActive(FIXTURE, homelab, 'team:eng'), false);
  assert.equal(admits(homelab.destination_role, 'Member · visibility 0'), true);

  // Activating federation admission increments reader count.
  const admitted: AgentSnapshot = {
    ...FIXTURE,
    federation: FIXTURE.federation.map((entry) => ({ ...entry, active: true })),
  };
  assert.equal(readerCount(admitted, 'team:eng|/deploy/staging-token'), 6);
  assert.equal(readerCount(admitted, 'team:eng|/deploy/production-token'), 3);
});

test('admitted groups fail closed when the matching admission is missing or ambiguous', () => {
  const homelab = partiesOf(FIXTURE, 'team:eng').find(
    (party) => party.party_kind === 'named-team',
  );
  assert.ok(homelab);
  assert.equal(
    admissionActive({ ...FIXTURE, federation: [] }, homelab, 'team:eng'),
    false,
  );
  const active = { ...FIXTURE.federation[0], active: true };
  assert.equal(
    admissionActive(
      { ...FIXTURE, federation: [active, { ...active }] },
      homelab,
      'team:eng',
    ),
    false,
  );
  const adhoc = { ...homelab, party_kind: 'ad-hoc-team' as const };
  assert.equal(
    admissionActive({ ...FIXTURE, federation: [] }, adhoc, 'team:eng'),
    false,
  );
});

test('readersOf returns null for account store items', () => {
  assert.equal(
    readersOf(FIXTURE, item(FIXTURE, 'acct:personal|/logins/github.com')),
    null,
  );
});

test('peopleGroups formats counts of individuals, machines and teams separately', () => {
  // A machine is a party like any other, but it is not a person: the summary
  // splits it out so a list row and the group page agree on "4 people".
  assert.equal(
    peopleGroups(partiesOf(FIXTURE, 'team:eng')),
    '4 people · 1 machine · 1 group',
  );
  assert.equal(peopleGroups(partiesOf(FIXTURE, 'team:household')), '2 people');
  assert.equal(peopleGroups([]), '0 people');
  assert.equal(
    peopleGroups(partiesOf(FIXTURE, 'team:household').slice(0, 1)),
    '1 person',
  );
});

test('partyName falls back from username to team name and identifier', () => {
  const [sam, , , , homelab] = partiesOf(FIXTURE, 'team:eng');
  assert.equal(partyName(sam), 'sam.ortiz');
  assert.equal(partyName(homelab), 'homelab @ foks.example.net');
});

/* ------------------------------------------------------------------ kinds -- */

test('kindOf classifies Secret items as Password or Resource based on path and content', () => {
  // Under /logins/ — a Password regardless of how its value is parsed.
  assert.equal(
    kindOf(item(FIXTURE, 'acct:personal|/logins/github.com')),
    'Password',
  );
  // A `password:` line makes a Password anywhere.
  assert.equal(
    kindOf(item(FIXTURE, 'team:household|/streaming/netflix')),
    'Password',
  );
  assert.equal(
    kindOf(item(FIXTURE, 'team:household|/wifi/guest-password')),
    'Password',
  );
  // Any other Secret is a Resource.
  assert.equal(
    kindOf(item(FIXTURE, 'acct:personal|/env/prod/DATABASE_URL')),
    'Resource',
  );
  assert.equal(
    kindOf(item(FIXTURE, 'acct:personal|/agents/anthropic-api-key')),
    'Resource',
  );
  // A small file is stored as the same `small-file` node as a note, so the
  // File product's /documents/ path is what keeps its kind as File even
  // when its bytes happen to be UTF-8 text.
  assert.equal(
    kindOf({
      kind: 'Secret',
      path: '/documents/readme.txt',
      value: 'plain UTF-8 text',
    }),
    'File',
  );
  assert.equal(
    kindOf({ kind: 'Secret', path: '/documents/empty.csv', value: '' }),
    'File',
  );
  // File and Link items preserve their declared kinds.
  assert.equal(
    kindOf(item(FIXTURE, 'acct:personal|/documents/passport-scan.pdf')),
    'File',
  );
  assert.equal(kindOf(item(FIXTURE, 'acct:personal|/latest-key')), 'Link');
  assert.equal(kindOf(item(FIXTURE, 'acct:personal|/ssh')), 'Folder');
});

test('isLogin returns true only for items under /logins/', () => {
  assert.equal(
    isLogin(item(FIXTURE, 'acct:personal|/logins/github.com')),
    true,
  );
  assert.equal(
    isLogin(item(FIXTURE, 'team:household|/streaming/netflix')),
    false,
  );
  assert.equal(isLogin(item(FIXTURE, 'acct:personal|/latest-key')), false);
});

test('rtype maps UI item kinds to underlying node storage types', () => {
  assert.equal(rtype({ kind: 'Secret' }), 'small_file');
  assert.equal(rtype({ kind: 'File' }), 'file');
  assert.equal(rtype({ kind: 'Link' }), 'symlink');
  assert.equal(rtype({ kind: 'Folder' }), 'directory');
});

test('nameOf and prefixOf extract filename and parent directory from path', () => {
  assert.equal(nameOf('/logins/github.com'), 'github.com');
  assert.equal(prefixOf('/logins/github.com'), 'logins');
  assert.equal(nameOf('/latest-key'), 'latest-key');
  assert.equal(prefixOf('/latest-key'), '');
  assert.equal(nameOf('/'), '/');
});

/* ----------------------------------------------------------------- format -- */

test('fmtSize formats byte counts with standard human-readable units', () => {
  assert.equal(fmtSize(null), 'Size unavailable');
  assert.equal(fmtSize(0), '0 B');
  assert.equal(fmtSize(142), '142 B');
  assert.equal(fmtSize(999), '999 B');
  assert.equal(fmtSize(1000), '1.0 KB');
  assert.equal(fmtSize(5120), '5.1 KB');
  assert.equal(fmtSize(9999), '10.0 KB');
  assert.equal(fmtSize(10000), '10 KB');
  assert.equal(fmtSize(2841992), '2.8 MB');
  assert.equal(fmtSize(84399718), '84.4 MB');
});

test('initials drop the mail domain and take at most two words', () => {
  assert.equal(initials('sam.ortiz'), 'SO');
  assert.equal(initials('vitalik'), 'V');
  assert.equal(initials('deploy-bot'), 'DB');
  assert.equal(initials('priya.n'), 'PN');
  assert.equal(initials('family@example.net'), 'F');
  assert.equal(initials('satoshi'), 'S');
});

test('a name always gets the same avatar colour', () => {
  assert.equal(hue('sam.ortiz'), '#a2845e');
  assert.equal(hue('vitalik'), '#34c759');
  assert.equal(hue('deploy-bot'), '#a2845e');
  assert.equal(hue('sam.ortiz'), hue('sam.ortiz'));
});

/** A store with only the fields `storeHues` reads. */
function refs(ids: string[]): Store[] {
  return ids.map((id) => ({ id }) as Store);
}

test('store marks keep their colour and never repeat one while the palette holds', () => {
  const stores = storeNavigationOrder(FIXTURE);
  const colors = storeHues(stores);
  assert.equal(colors.size, stores.length);
  assert.deepEqual([...colors.entries()], [...storeHues(stores).entries()]);
  // The palette has eight entries; the fixture has fewer stores than that.
  assert.ok(stores.length <= 8);
  assert.equal(new Set(colors.values()).size, stores.length);
  // A store keyed on its reference takes the hashed colour when it is free.
  const first = stores[0];
  assert.equal(colors.get(first.id), hue(first.id));
});

test('a colliding store takes the first free palette entry, and only that store moves', () => {
  // `hue` sums code points, so these two references hash to the same entry.
  const left = 'acct:ab';
  const right = 'acct:ba';
  assert.equal(hue(left), hue(right));
  const colors = storeHues(refs([left, right]));
  assert.equal(colors.get(left), hue(left));
  assert.notEqual(colors.get(right), hue(right));
  assert.equal(
    colors.get(right),
    HUES.find((entry) => entry !== hue(left)),
  );
});

test('store marks fall back to the hashed colour once the palette is spent', () => {
  const ids = Array.from(
    { length: HUES.length + 2 },
    (_, index) => `s${index}`,
  );
  const colors = storeHues(refs(ids));
  assert.equal(colors.size, ids.length);
  assert.equal(new Set([...colors.values()]).size, HUES.length);
  const palette: readonly string[] = HUES;
  for (const color of colors.values()) assert.ok(palette.includes(color));
});

/* ------------------------------------------------------------------ lease -- */

test('fixture initializes with fresh lease state', () => {
  const acme = FIXTURE.servers.find((server) => server.id === 'acme');
  assert.equal(acme?.compatibility.status, 'required');
  assert.equal(acme?.trust.status, 'verified');
  assert.equal(leaseLapsed(FIXTURE, 'team:eng'), false);
});

test('an inactive team lists nothing even on a fresh lease', () => {
  assert.equal(storeReadable(FIXTURE, 'team:homelab'), false);
  assert.equal(storeReadable(FIXTURE, 'team:eng'), true);
  assert.equal(storeReadable(FIXTURE, 'acct:work'), true);
});

test('an inactive group is described as setup incomplete', () => {
  const store = FIXTURE.stores.find(
    (candidate) => candidate.id === 'team:homelab',
  );
  assert.ok(store);
  assert.equal(storeDescription(FIXTURE, store), 'Setup incomplete');
});

test('a group-detail failure changes its caption without stopping item access', () => {
  const snapshot = {
    ...FIXTURE,
    groupDetailFailures: [
      {
        store: 'team:eng',
        source: 'roster' as const,
        code: 'rate-limited',
        message: 'busy',
        retryable: true,
      },
    ],
  };
  const store = snapshot.stores.find(
    (candidate) => candidate.id === 'team:eng',
  );
  assert.ok(store);
  assert.equal(storeDescription(snapshot, store), 'Roster unavailable');
  assert.equal(storeReadable(snapshot, store.id), true);
});

test('group item changes require one authenticated local party that admits the write role', () => {
  const staging = item(FIXTURE, 'team:eng|/deploy/staging-token');
  const production = item(FIXTURE, 'team:eng|/deploy/production-token');
  const personal = item(FIXTURE, 'acct:personal|/logins/github.com');
  assert.equal(canChangeItem(FIXTURE, staging), true, 'Admin admits Admin');
  assert.equal(
    canChangeItem(FIXTURE, production),
    false,
    'Admin does not admit Owner',
  );
  assert.equal(
    canChangeItem(FIXTURE, personal),
    true,
    'the available account store is local',
  );

  const noAuthenticatedParty: AgentSnapshot = {
    ...FIXTURE,
    parties: FIXTURE.parties.map((party) =>
      party.store === 'team:eng' && party.label === 'you'
        ? { ...party, label: undefined }
        : party,
    ),
  };
  assert.equal(canChangeItem(noAuthenticatedParty, staging), false);
  assert.equal(canCreateInStore(noAuthenticatedParty, 'team:eng'), false);
  assert.equal(canChangeItem(applyLease(FIXTURE, 'lapsed'), staging), false);
  assert.equal(
    canCreateInStore(applyLease(FIXTURE, 'lapsed'), 'team:eng'),
    false,
  );
});

test('lapsed lease disables reads and writes for affected server stores', () => {
  const lapsed = applyLease(FIXTURE, 'lapsed');

  assert.equal(leaseLapsed(lapsed, 'team:eng'), true);
  assert.equal(storeReadable(lapsed, 'team:eng'), false);
  assert.equal(storeReadable(lapsed, 'acct:work'), false);
  // foks.example.net is untouched.
  assert.equal(storeReadable(lapsed, 'acct:personal'), true);
  assert.equal(storeReadable(lapsed, 'team:household'), true);

  // Folders and items on lapsed servers are excluded from the catalog.
  assert.equal(catalog(FIXTURE).length, 14);
  assert.equal(catalog(lapsed).length, 10);

  // applyLease does not mutate the input state.
  assert.equal(storeReadable(FIXTURE, 'team:eng'), true);

  // Lease restoration round-trips correctly.
  assert.equal(storeReadable(applyLease(lapsed, 'fresh'), 'team:eng'), true);
});

test('the catalog excludes folders', () => {
  assert.equal(
    catalog(FIXTURE).some((entry) => entry.kind === 'Folder'),
    false,
  );
  assert.equal(
    FIXTURE.items.filter((entry) => entry.kind === 'Folder').length,
    1,
  );
});

test('every store the sidebar orders is in the fixture', () => {
  const ordered = storeDisplayOrder(FIXTURE);
  for (const { id } of ordered) {
    assert.ok(
      FIXTURE.stores.some((store) => store.id === id),
      id,
    );
  }
  assert.equal(ordered.length, FIXTURE.stores.length);
});

test('navigation orders vaults, named groups, then ad-hoc shares', () => {
  const byId = new Map(FIXTURE.stores.map((store) => [store.id, store]));
  const snapshot: AgentSnapshot = {
    ...FIXTURE,
    stores: [
      byId.get('team:homelab'),
      byId.get('team:household'),
      byId.get('acct:work'),
      byId.get('team:eng'),
      byId.get('acct:personal'),
    ].filter((store) => store !== undefined),
  };
  assert.deepEqual(
    storeNavigationOrder(snapshot).map((store) => store.id),
    [
      'acct:work',
      'acct:personal',
      'team:household',
      'team:eng',
      'team:homelab',
    ],
  );
});

test('safestRemovalTarget selects member with lowest role rank, breaking ties by roster order', () => {
  assert.equal(
    safestRemovalTarget(FIXTURE, 'team:eng')?.username,
    'dana.okafor',
  );

  const changed: AgentSnapshot = {
    ...FIXTURE,
    parties: FIXTURE.parties.map((party) =>
      party.username === 'dana.okafor'
        ? { ...party, destination_role: { role: 'Owner' as const } }
        : party,
    ),
  };
  assert.equal(
    safestRemovalTarget(changed, 'team:eng')?.username,
    'deploy-bot',
  );

  const lowerBand: AgentSnapshot = {
    ...FIXTURE,
    parties: FIXTURE.parties.map((party) =>
      party.username === 'deploy-bot'
        ? {
            ...party,
            destination_role: { role: 'Member' as const, visibility: -1 },
          }
        : party,
    ),
  };
  assert.equal(
    safestRemovalTarget(lowerBand, 'team:eng')?.username,
    'deploy-bot',
  );
});

test('admissionActive rejects federation admissions with mismatching host IDs', () => {
  const homelab = FIXTURE.parties.find(
    (party) => party.party_kind !== 'user' && party.scoped_host_id_hex,
  );
  assert.ok(
    homelab?.scoped_host_id_hex,
    'the fixture has a host-scoped member group',
  );
  const own = FIXTURE.federation.find(
    (entry) => entry.remote_team_id_hex === homelab.party_id_hex,
  );
  assert.ok(own);
  // Active admissions recorded from a different host must not grant access.
  const elsewhere = {
    ...own,
    remote_host_id_hex: 'ffff-not-this-host',
    active: true,
  };
  assert.equal(
    admissionActive(
      { ...FIXTURE, federation: [elsewhere] },
      homelab,
      own.store,
    ),
    false,
  );
  assert.equal(
    admissionActive(
      { ...FIXTURE, federation: [{ ...own, active: true }] },
      homelab,
      own.store,
    ),
    true,
  );
});

test('prefixOf yields no prefix for a path with no slash', () => {
  // Paths without directory slashes have no parent prefix.
  assert.equal(prefixOf('noSlash'), '');
  assert.equal(prefixOf('/top'), '');
  assert.equal(prefixOf('/logins/github.com'), 'logins');
});

test('a never-probed server is a stopped store, not a normal one', () => {
  const store = FIXTURE.stores.find(
    (candidate) => candidate.kind === 'account',
  );
  assert.ok(store);
  const snapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === store.server
        ? { ...server, trust: { status: 'unprobed' as const } }
        : server,
    ),
  };
  // Never-probed servers must be treated as inactive rather than normal.
  assert.equal(storeDescriptionState(snapshot, store), 'verification-required');
  assert.equal(storeDescriptionState(FIXTURE, store), 'normal');
});

test('storeAttentionState counts an unread detail the row would summarize', () => {
  const store = FIXTURE.stores.find(
    (candidate) => candidate.id === 'team:household',
  );
  assert.ok(store);
  // Nothing is wrong with the fixture's group: its roster is the caption.
  assert.equal(storeAttentionState(FIXTURE, store), 'normal');
  assert.equal(storeDescription(FIXTURE, store), '2 people');

  // A roster the agent could not read leaves the store reachable, so only
  // `storeAttentionState` knows the row has something to say. Without it a
  // list would print "Roster unavailable" where the roster summary goes.
  const unread = {
    ...FIXTURE,
    parties: FIXTURE.parties.filter((party) => party.store !== store.id),
    groupDetailFailures: [
      {
        store: store.id,
        source: 'roster' as const,
        code: 'unavailable',
        message: 'The group roster could not be read from the agent.',
        retryable: true,
      },
    ],
  };
  assert.equal(storeDescriptionState(unread, store), 'normal');
  assert.equal(storeAttentionState(unread, store), 'roster-unavailable');
  assert.equal(storeDescription(unread, store), 'Roster unavailable');

  // A federation read that failed is the same kind of fact.
  const federation = {
    ...unread,
    groupDetailFailures: unread.groupDetailFailures.map((failure) => ({
      ...failure,
      source: 'federation' as const,
    })),
  };
  assert.equal(
    storeAttentionState(federation, store),
    'federation-unavailable',
  );

  // An unavailable store keeps the reason it is unavailable: the access
  // decision outranks a detail that could not be read under it.
  const lapsed = applyLease(unread, 'lapsed', store.server);
  assert.equal(storeAttentionState(lapsed, store), 'check-in-expired');
});

test('teamCaption drops the server on a page that is already about one', () => {
  const store = FIXTURE.stores.find(
    (candidate) => candidate.id === 'team:household',
  );
  assert.ok(store && store.kind === 'team');
  assert.equal(teamCaption(FIXTURE, store), 'Named group · Personal server');
  assert.equal(teamCaption(FIXTURE, store, { server: false }), 'Named group');
  // The account is named only where it is asked for, and after the server.
  assert.equal(
    teamCaption(FIXTURE, store, { shared: true }),
    'Named group · Personal server · as satoshi',
  );
  assert.equal(
    teamCaption(FIXTURE, store, { server: false, shared: true }),
    'Named group · as satoshi',
  );
});

test('authoritative empty inventory is complete while an omitted configured profile is not', () => {
  const empty: AgentSnapshot = {
    ...FIXTURE,
    servers: [],
    stores: [],
    storeInventory: [],
    profileInventory: [],
    catalogProfiles: [],
    profileInventoryStatus: 'complete',
  };
  assert.equal(profileInventoryComplete(empty, 'profiles'), true);
  assert.equal(profileInventoryComplete(empty, 'accounts'), true);
  assert.equal(profileInventoryComplete(empty, 'teams'), true);
  const unknownEmpty = {
    ...empty,
    profileInventoryStatus: 'unavailable' as const,
  };
  assert.equal(profileInventoryComplete(unknownEmpty, 'accounts'), false);
  assert.equal(profileInventoryComplete(unknownEmpty, 'teams'), false);

  const missing = {
    ...FIXTURE,
    catalogProfiles: [...FIXTURE.catalogProfiles, 'omitted-profile'],
  };
  assert.equal(profileInventoryComplete(missing, 'accounts'), false);
  assert.equal(profileInventoryComplete(missing, 'teams'), false);
});

test('store security restrictions outrank server lease and inventory failures', () => {
  const store = FIXTURE.stores.find((entry) => entry.server === 'acme');
  assert.ok(store);
  const error = {
    code: 'schema',
    message: 'arbitrary',
    fatal: false,
    retryable: false,
    ambiguous: false,
  };
  const snapshot: AgentSnapshot = {
    ...applyLease(FIXTURE, 'lapsed', 'acme', 100),
    storeInventory: FIXTURE.storeInventory.map((entry) =>
      entry.store === store.id
        ? {
            ...entry,
            status: 'unavailable' as const,
            error,
            restrictions: [{ kind: 'schema-incompatible' as const, error }],
          }
        : entry,
    ),
  };
  assert.deepEqual(storeAvailability(snapshot, store, { nowSeconds: 100 }), {
    available: false,
    reason: 'schema-incompatible',
  });
  const neighbor = FIXTURE.stores.find((entry) => entry.server === 'personal');
  assert.ok(neighbor);
  assert.equal(storeAvailability(snapshot, neighbor).available, true);
});

test('every fixture admission names a profile, not an address', () => {
  // Verify that remote_profile references a known server profile ID rather than an address.
  for (const entry of FIXTURE.federation) {
    assert.ok(
      FIXTURE.servers.some((server) => server.id === entry.remote_profile),
      `${entry.remote_profile} is a profile in the fixture`,
    );
    assert.ok(
      !FIXTURE.servers.some((server) => server.name === entry.remote_profile),
      `${entry.remote_profile} is not a server address`,
    );
  }
});
