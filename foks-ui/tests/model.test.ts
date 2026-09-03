/**
 * Goldens for the model port.
 *
 * Every number here is the wave 6 fixture's own, taken by running
 * `dev/foks-desktop/iteration/wave6/shell.js` under node and reading the
 * answers off it. **They are the specification.** If the port disagrees with
 * one, the port is wrong: go back to `shell.js`, not to this file.
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
  readersOf,
  roleRank,
  rtype,
  safestRemovalTarget,
  storeReadable,
  storeDisplayOrder,
  storeNavigationOrder,
} from '../src/model';
import type { Item, World } from '../src/model';

function item(world: World, key: string): Item {
  const found = world.items.find((candidate) => itemKey(candidate) === key);
  assert.ok(found, `${key} is in the fixture`);
  return found;
}

function readerCount(world: World, key: string): number {
  const readers = readersOf(world, item(world, key));
  assert.ok(readers, `${key} is in a team store`);
  return readers.length;
}

/* ------------------------------------------------------------------ roles -- */

test('the role order is Member(-0x4000) < Member(0) < Admin < Owner', () => {
  assert.equal(roleRank('Member · visibility -16384'), 1);
  assert.equal(roleRank('Member'), 1);
  assert.equal(roleRank('Admin'), 2);
  assert.equal(roleRank('Owner'), 3);

  // Within Member the band breaks the tie; across ranks it never applies.
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

  // A role this client cannot parse is refused whichever side it is on. The
  // needed side is the one that used to fail open: an unparseable `need`
  // ranks 0, so a Member out-ranked it and was told it could read an item
  // whose read role the client does not understand.
  assert.equal(admits('Member · visibility 0', 'Reader'), false);
  assert.equal(admits('Member · visibility -32768', 'Superuser'), false);
  assert.equal(admits('Owner', 'Reader'), false);
  assert.equal(admits('Reader', 'Owner'), false);
  assert.equal(admits('Reader', 'Reader'), false);
});

test('admits reads the structured role the roster carries', () => {
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

test('parseRole normalises both wire shapes and refuses anything else', () => {
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

  // The product has no Reader, Manager, Viewer or Editor item role.
  for (const invented of ['Reader', 'Manager', 'Viewer', 'Editor']) {
    assert.equal(parseRole(invented), null, invented);
    assert.equal(roleRank(invented), 0, invented);
  }
});

test('formatRole writes the roles the way the design does', () => {
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

test('Readable by is read role times roster, on the fixture s own numbers', () => {
  // production-token reads at Admin: sam.ortiz (Owner), rae.chen and priya.n
  // (Admin). dana.okafor and deploy-bot are Members; homelab is excluded.
  assert.equal(readerCount(FIXTURE, 'team:eng|/deploy/production-token'), 3);

  // staging-token and bundle.tar read at Member · visibility 0: the same three
  // plus dana.okafor and deploy-bot — five. Homelab holds Member · visibility 0
  // and would qualify, but its admission reports inactive.
  assert.equal(readerCount(FIXTURE, 'team:eng|/deploy/staging-token'), 5);
  assert.equal(readerCount(FIXTURE, 'team:eng|/release/bundle.tar'), 5);
  assert.equal(readerCount(FIXTURE, 'team:eng|/onboarding/README.md'), 5);

  // Household has two parties and both read at Member · visibility 0.
  assert.equal(readerCount(FIXTURE, 'team:household|/wifi/guest-password'), 2);
  assert.equal(
    readerCount(FIXTURE, 'team:household|/documents/emergency.pdf'),
    2,
  );
});

test('an inactive federation admission is the only thing keeping homelab out', () => {
  const homelab = partiesOf(FIXTURE, 'team:eng').find(
    (party) => party.party_kind === 'named-team',
  );
  assert.ok(homelab);
  assert.equal(admissionActive(FIXTURE, homelab, 'team:eng'), false);
  assert.equal(admits(homelab.destination_role, 'Member · visibility 0'), true);

  // Flip the admission active and the count goes to six — nothing else changed.
  const admitted: World = {
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

test('an account store answers null rather than a count', () => {
  assert.equal(
    readersOf(FIXTURE, item(FIXTURE, 'acct:personal|/logins/github.com')),
    null,
  );
});

test('a party that is a team is not a person', () => {
  assert.equal(
    peopleGroups(partiesOf(FIXTURE, 'team:eng')),
    '5 people · 1 group',
  );
  assert.equal(peopleGroups(partiesOf(FIXTURE, 'team:household')), '2 people');
  assert.equal(peopleGroups([]), '0 people');
  assert.equal(
    peopleGroups(partiesOf(FIXTURE, 'team:household').slice(0, 1)),
    '1 person',
  );
});

test('a party names itself by username, team name, then id', () => {
  const [sam, , , , homelab] = partiesOf(FIXTURE, 'team:eng');
  assert.equal(partyName(sam), 'sam.ortiz');
  assert.equal(partyName(homelab), 'homelab @ foks.example.net');
});

/* ------------------------------------------------------------------ kinds -- */

test('the kind rule reads Secrets as Passwords or Resources', () => {
  // Under /logins/ — a Password however its value reads.
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
  // File and Link are node types, not readings.
  assert.equal(
    kindOf(item(FIXTURE, 'acct:personal|/documents/passport-scan.pdf')),
    'File',
  );
  assert.equal(kindOf(item(FIXTURE, 'acct:personal|/latest-key')), 'Link');
  assert.equal(kindOf(item(FIXTURE, 'acct:personal|/ssh')), 'Folder');
});

test('isLogin is the /logins/ Password, not every Password', () => {
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

test('rtype names the node type the kind is a reading of', () => {
  assert.equal(rtype({ kind: 'Secret' }), 'small_file');
  assert.equal(rtype({ kind: 'File' }), 'file');
  assert.equal(rtype({ kind: 'Link' }), 'symlink');
  assert.equal(rtype({ kind: 'Folder' }), 'directory');
});

test('paths split into a name and a folder chip', () => {
  assert.equal(nameOf('/logins/github.com'), 'github.com');
  assert.equal(prefixOf('/logins/github.com'), 'logins');
  assert.equal(nameOf('/latest-key'), 'latest-key');
  assert.equal(prefixOf('/latest-key'), '');
  assert.equal(nameOf('/'), '/');
});

/* ----------------------------------------------------------------- format -- */

test('sizes read the way the design writes them', () => {
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
  assert.equal(initials('rae.chen'), 'RC');
  assert.equal(initials('deploy-bot'), 'DB');
  assert.equal(initials('priya.n'), 'PN');
  assert.equal(initials('family@example.net'), 'F');
  assert.equal(initials('rae'), 'R');
});

test('a name always gets the same avatar colour', () => {
  assert.equal(hue('sam.ortiz'), '#a2845e');
  assert.equal(hue('rae.chen'), '#34c759');
  assert.equal(hue('deploy-bot'), '#a2845e');
  assert.equal(hue('sam.ortiz'), hue('sam.ortiz'));
});

/* ------------------------------------------------------------------ lease -- */

test('the shell starts in the fresh lease world', () => {
  assert.equal(FIXTURE.leaseState, 'fresh');
  const acme = FIXTURE.servers.find((server) => server.id === 'acme');
  assert.deepEqual(acme?.lease, { state: 'fresh', expires_in: '12 d' });
  assert.equal(acme?.state, 'ok');
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
  const world = {
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
  const store = world.stores.find((candidate) => candidate.id === 'team:eng');
  assert.ok(store);
  assert.equal(storeDescription(world, store), 'Roster unavailable');
  assert.equal(storeReadable(world, store.id), true);
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

  const noAuthenticatedParty: World = {
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

test('a lapsed lease stops reads with writes, and only on that server', () => {
  const lapsed = applyLease(FIXTURE, 'lapsed');

  assert.equal(lapsed.leaseState, 'lapsed');
  assert.equal(leaseLapsed(lapsed, 'team:eng'), true);
  assert.equal(storeReadable(lapsed, 'team:eng'), false);
  assert.equal(storeReadable(lapsed, 'acct:work'), false);
  // foks.example.net is untouched.
  assert.equal(storeReadable(lapsed, 'acct:personal'), true);
  assert.equal(storeReadable(lapsed, 'team:household'), true);

  // Folders are not items; Acme's four items and Homelab's none drop out.
  assert.equal(catalog(FIXTURE).length, 14);
  assert.equal(catalog(lapsed).length, 10);

  // applyLease is pure: the world it was handed did not move.
  assert.equal(FIXTURE.leaseState, 'fresh');
  assert.equal(storeReadable(FIXTURE, 'team:eng'), true);

  // And it round-trips.
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
  const world: World = {
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
    storeNavigationOrder(world).map((store) => store.id),
    [
      'acct:work',
      'acct:personal',
      'team:household',
      'team:eng',
      'team:homelab',
    ],
  );
});

test('the safest removal target comes from live authority with roster order as the tie-break', () => {
  assert.equal(
    safestRemovalTarget(FIXTURE, 'team:eng')?.username,
    'dana.okafor',
  );

  const changed: World = {
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

  const lowerBand: World = {
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

test("an admission from another host is not this scoped party's admission", () => {
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
  // The same admission, active, but recorded from a different host: `loadWorld`
  // would not select it for this party, and neither may the reader check —
  // it used to count it and list the group as a live reader.
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
  // `lastIndexOf` is -1 here and `slice(1, -1)` chopped both ends off the name.
  assert.equal(prefixOf('noSlash'), '');
  assert.equal(prefixOf('/top'), '');
  assert.equal(prefixOf('/logins/github.com'), 'logins');
});

test('a never-probed server is a stopped store, not a normal one', () => {
  const store = FIXTURE.stores.find(
    (candidate) => candidate.kind === 'account',
  );
  assert.ok(store);
  const world = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === store.server
        ? { ...server, state: 'never-probed' as const, lease: null }
        : server,
    ),
  };
  // `loadWorld` hides this server's stores exactly as it hides a lapsed one's;
  // describing it as `normal` drew an undimmed row over an unexplained empty list.
  assert.equal(storeDescriptionState(world, store), 'never-probed');
  assert.equal(storeDescriptionState(FIXTURE, store), 'normal');
});

test('every fixture admission names a profile, not an address', () => {
  // `remote_profile` is the profile name as the agent records it. The fixture
  // used to store the server's address here, which is why the pin lookup
  // once had to try the address too.
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
