/**
 * Tests for location transitions, URL encoding, decoding, and store subscriptions.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  INITIAL_SCENE,
  INITIAL_STATE,
  LocationStore,
  decodeLocation,
  decodeScene,
  sceneHref,
  encodeLocation,
  getState,
  locationHref,
  parentLocation,
  railTabOf,
  sameLocation,
  setUrl,
  transition,
} from '../src/location';
import type {
  Location,
  LocationState,
  NavigationGuard,
  NavigationIntent,
  NavigationPrompter,
} from '../src/location';

const at = (location: Location): LocationState => ({
  ...INITIAL_STATE,
  location,
});

/* ------------------------------------------------------------ transitions -- */

test('navigating to a different location clears selection', () => {
  const selected: LocationState = {
    ...INITIAL_STATE,
    location: { kind: 'store', ref: 'team:eng' },
    selection: { store: 'team:eng', path: '/deploy/staging-token' },
    query: 'token',
  };
  const next = transition(selected, {
    type: 'navigate',
    location: { kind: 'all' },
  });
  assert.deepEqual(next.location, { kind: 'all' });
  assert.equal(next.selection, null);
  // Search query is preserved across location transitions.
  assert.equal(next.query, 'token');
});

test('navigating to the current location preserves selection', () => {
  const selected: LocationState = {
    ...INITIAL_STATE,
    location: { kind: 'store', ref: 'team:eng' },
    selection: { store: 'team:eng', path: '/deploy/staging-token' },
    query: '',
  };
  const next = transition(selected, {
    type: 'navigate',
    location: { kind: 'store', ref: 'team:eng' },
  });
  assert.deepEqual(next.selection, selected.selection);
});

test('a replacing navigation keeps what the place was showing', () => {
  const showing: LocationState = {
    ...INITIAL_STATE,
    location: { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
    selection: { store: 'team:eng', path: '/deploy/staging-token' },
    folder: '/deploy',
    query: 'token',
  };
  // Walking the group page's tab strip is a move inside the page the reader
  // is on, not an arrival somewhere new, so it stands in place of the last
  // rather than clearing what that page had open.
  const next = transition(showing, {
    type: 'navigate',
    location: { kind: 'group-settings', ref: 'team:eng', tab: 'channels' },
    replace: true,
  });
  assert.deepEqual(next.location, {
    kind: 'group-settings',
    ref: 'team:eng',
    tab: 'channels',
  });
  assert.deepEqual(next.selection, showing.selection);
  assert.equal(next.folder, '/deploy');
  assert.equal(next.query, 'token');
});

test('a no-op action returns the same state object', () => {
  const state = at({ kind: 'all' });
  assert.equal(transition(state, { type: 'search', query: '' }), state);
  assert.notEqual(transition(state, { type: 'search', query: 'wifi' }), state);
});

test('folder selection clears an item while fold state remains independent', () => {
  const state: LocationState = {
    ...INITIAL_STATE,
    selection: { store: 'acct:personal', path: '/agents/key' },
    folder: 'acct:personal|/agents',
  };
  const selected = transition(state, {
    type: 'folder',
    folder: 'acct:personal|/documents',
  });
  assert.equal(selected.selection, null);
  assert.equal(selected.folder, 'acct:personal|/documents');

  const folded = transition(selected, {
    type: 'toggle-folder',
    folder: 'acct:personal|/agents',
  });
  assert.equal(folded.folder, selected.folder);
  assert.deepEqual(folded.closedFolders, ['acct:personal|/agents']);
});

test('select and search actions preserve the current location', () => {
  const state = at({ kind: 'store', ref: 'acct:personal' });
  const selected = transition(state, {
    type: 'select',
    selection: { store: 'acct:personal', path: '/latest-key' },
  });
  assert.deepEqual(selected.location, state.location);
  assert.deepEqual(selected.selection, {
    store: 'acct:personal',
    path: '/latest-key',
  });
  assert.equal(
    transition(selected, { type: 'select', selection: null }).selection,
    null,
  );
});

test('sameLocation compares full location properties', () => {
  assert.equal(
    sameLocation({ kind: 'store', ref: 'a' }, { kind: 'store', ref: 'a' }),
    true,
  );
  assert.equal(
    sameLocation({ kind: 'store', ref: 'a' }, { kind: 'store', ref: 'b' }),
    false,
  );
  assert.equal(
    sameLocation({ kind: 'settings' }, { kind: 'settings', section: 'mac' }),
    false,
  );
  assert.equal(
    sameLocation(
      { kind: 'first-run', step: 'who' },
      { kind: 'first-run', step: 'who' },
    ),
    true,
  );
  assert.equal(sameLocation({ kind: 'all' }, { kind: 'people' }), false);
  assert.equal(
    sameLocation({ kind: 'people' }, { kind: 'people', store: 'acct:work' }),
    false,
  );
  assert.equal(
    sameLocation(
      { kind: 'devices', section: 'macs' },
      { kind: 'devices', section: 'keys' },
    ),
    false,
  );
  assert.equal(sameLocation({ kind: 'chat' }, { kind: 'chat' }), true);
  // The chat location carries the open team and channel.
  assert.equal(
    sameLocation({ kind: 'chat' }, { kind: 'chat', ref: 'team:eng' }),
    false,
  );
  assert.equal(
    sameLocation(
      { kind: 'chat', ref: 'team:eng', channel: 'ab'.repeat(16) },
      { kind: 'chat', ref: 'team:eng', channel: 'ab'.repeat(16) },
    ),
    true,
  );
  assert.equal(
    sameLocation(
      { kind: 'chat', ref: 'team:eng' },
      { kind: 'chat', ref: 'team:household' },
    ),
    false,
  );
  assert.equal(sameLocation({ kind: 'files' }, { kind: 'teams' }), false);
  // Teams names the account it creates and discovers groups as.
  assert.equal(
    sameLocation({ kind: 'teams' }, { kind: 'teams', store: 'acct:work' }),
    false,
  );
  assert.equal(
    sameLocation(
      { kind: 'teams', store: 'acct:work' },
      { kind: 'teams', store: 'acct:work' },
    ),
    true,
  );
});

test('railTabOf names the tab a location belongs to', () => {
  assert.equal(railTabOf({ kind: 'people' }), 'people');
  assert.equal(railTabOf({ kind: 'chat' }), 'chat');
  assert.equal(railTabOf({ kind: 'chat', ref: 'team:eng' }), 'chat');
  assert.equal(railTabOf({ kind: 'files' }), 'files');
  assert.equal(railTabOf({ kind: 'all' }), 'files');
  assert.equal(railTabOf({ kind: 'store', ref: 'acct:personal' }), 'files');
  assert.equal(railTabOf({ kind: 'teams' }), 'teams');
  assert.equal(railTabOf({ kind: 'group-settings', ref: 'team:eng' }), 'teams');
  assert.equal(railTabOf({ kind: 'devices' }), 'devices');
  assert.equal(railTabOf({ kind: 'settings' }), 'settings');
  // First run replaces the rail, so it belongs to no tab.
  assert.equal(railTabOf({ kind: 'first-run', step: 'who' }), null);
});

/* -------------------------------------------------------------- URL codec -- */

const ROUND_TRIP: Location[] = [
  { kind: 'all' },
  { kind: 'store', ref: 'acct:personal' },
  { kind: 'store', ref: 'team:eng' },
  { kind: 'people' },
  { kind: 'people', store: 'acct:work' },
  { kind: 'chat' },
  { kind: 'chat', ref: 'team:eng' },
  { kind: 'chat', ref: 'team:eng', channel: 'ab'.repeat(16) },
  { kind: 'files' },
  { kind: 'teams' },
  { kind: 'teams', store: 'acct:work' },
  { kind: 'devices' },
  { kind: 'devices', section: 'macs' },
  { kind: 'devices', section: 'keys', store: 'acct:work' },
  { kind: 'devices', store: 'acct:personal', device: '04a779c40674' },
  { kind: 'devices', store: 'acct:personal', device: 'yubi:primary key' },
  { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
  { kind: 'group-settings', ref: 'team:eng', tab: 'channels' },
  { kind: 'group-settings', ref: 'team:eng', tab: 'files' },
  { kind: 'group-settings', ref: 'team:eng', tab: 'settings' },
  { kind: 'settings', section: 'servers' },
  { kind: 'settings', section: 'servers', profile: 'acme' },
  { kind: 'settings' },
  { kind: 'settings', section: 'preferences' },
  { kind: 'settings', section: 'preferences', store: 'acct:work' },
  { kind: 'settings', section: 'mac' },
  { kind: 'first-run', step: 'who' },
];

test('all supported locations round-trip through URL query serialization', () => {
  for (const location of ROUND_TRIP) {
    const href = locationHref('http://localhost/', location);
    const decoded = decodeLocation(new URL(href).search);
    assert.deepEqual(decoded, location, href);
  }
});

test('encoding clears query parameters from previous location', () => {
  const href = locationHref(
    'http://localhost/?state=store&store=team:eng&section=agent&step=who',
    { kind: 'all' },
  );
  const url = new URL(href);
  assert.equal(url.searchParams.get('state'), 'all');
  assert.equal(url.searchParams.get('store'), null);
  assert.equal(url.searchParams.get('section'), null);
  assert.equal(url.searchParams.get('step'), null);
});

test('legacy state aliases do not collide with group or first-run routes', () => {
  assert.deepEqual(decodeLocation('?state=servers-add'), {
    kind: 'settings',
    section: 'servers',
  });
  assert.deepEqual(decodeLocation('?state=settings-account'), {
    kind: 'people',
  });
  assert.deepEqual(decodeLocation('?state=add'), {
    kind: 'group-settings',
    ref: 'team:eng',
    tab: 'people',
  });
  assert.deepEqual(decodeLocation('?state=account&path=own'), {
    kind: 'first-run',
    step: 'account',
    path: 'own',
  });
  assert.deepEqual(
    decodeLocation('?state=settings&section=macs&store=acct:work'),
    { kind: 'devices', section: 'macs', store: 'acct:work' },
  );
});

test('the sections that became tabs keep their deep links', () => {
  // `section=` values written before the rail canonicalize onto the tab that
  // owns the pane now, carrying the account they named.
  assert.deepEqual(decodeLocation('?state=settings&section=macs'), {
    kind: 'devices',
    section: 'macs',
  });
  assert.deepEqual(decodeLocation('?state=settings&section=phrase'), {
    kind: 'devices',
    section: 'macs',
  });
  assert.deepEqual(
    decodeLocation('?state=settings&section=keys&store=acct:work'),
    { kind: 'devices', section: 'keys', store: 'acct:work' },
  );
  assert.deepEqual(decodeLocation('?state=settings&section=groups'), {
    kind: 'teams',
  });
  // Teams acts as one account too, so the account the address named comes with
  // it rather than being dropped for the first account on this Mac.
  assert.deepEqual(
    decodeLocation('?state=settings&section=groups&store=acct:work'),
    { kind: 'teams', store: 'acct:work' },
  );
  assert.deepEqual(decodeLocation('?state=teams&store=acct:work'), {
    kind: 'teams',
    store: 'acct:work',
  });
  assert.deepEqual(decodeLocation('?state=teams'), { kind: 'teams' });
  assert.deepEqual(
    decodeLocation('?state=settings&section=account&store=acct:work'),
    { kind: 'people', store: 'acct:work' },
  );
  // `section=phrase` is the backup phrase, which the Devices tab's recovery
  // pane opens; `?state=devices` reads it under its own name as well.
  assert.deepEqual(decodeLocation('?state=devices&section=phrase'), {
    kind: 'devices',
    section: 'macs',
  });
  assert.deepEqual(
    decodeLocation('?state=devices&section=phrase&store=acct:work'),
    { kind: 'devices', section: 'macs', store: 'acct:work' },
  );
  // One key's own page is an address of its own: the key id for a Mac or a
  // paper key, and `yubi:<alias>` for an enrollment the agent names by alias.
  assert.deepEqual(
    decodeLocation('?state=devices&store=acct:personal&device=04a779c40674'),
    { kind: 'devices', store: 'acct:personal', device: '04a779c40674' },
  );
  // The Devices names written before the page was one carry it too.
  assert.deepEqual(
    decodeLocation('?state=settings-macs-work&device=yubi:work key'),
    {
      kind: 'devices',
      section: 'macs',
      store: 'acct:work',
      device: 'yubi:work key',
    },
  );
  // A device is not a Settings address: `device=` is cleared with the rest.
  assert.equal(
    new URL(
      locationHref('http://localhost/?state=devices&device=04a7', {
        kind: 'settings',
        section: 'mac',
      }),
    ).searchParams.get('device'),
    null,
  );
  // The panes Settings kept are unchanged.
  assert.deepEqual(decodeLocation('?state=settings&section=servers'), {
    kind: 'settings',
    section: 'servers',
  });
  // Preferences carries the account the address named.
  assert.deepEqual(decodeLocation('?state=settings&section=preferences'), {
    kind: 'settings',
    section: 'preferences',
  });
  assert.deepEqual(
    decodeLocation('?state=settings&section=preferences&store=acct:work'),
    { kind: 'settings', section: 'preferences', store: 'acct:work' },
  );
  // `profile` belongs to the Servers section alone, so it is dropped here.
  assert.deepEqual(
    decodeLocation('?state=settings&section=preferences&profile=acme'),
    { kind: 'settings', section: 'preferences' },
  );
  // Written back out, that address is the same address.
  for (const location of [
    { kind: 'settings', section: 'preferences' },
    { kind: 'settings', section: 'preferences', store: 'acct:work' },
  ] as const) {
    const href = locationHref('http://localhost/?state=all', location);
    assert.deepEqual(decodeLocation(new URL(href).search), location, href);
  }
  // Accounts became People, and the address says so.
  assert.deepEqual(decodeLocation('?state=settings&section=account'), {
    kind: 'people',
  });
});

test('the sub-navigation’s three pages deep-link, and an address for a section that no longer exists lands on the tab', () => {
  for (const section of ['servers', 'preferences', 'mac'] as const) {
    assert.deepEqual(decodeLocation(`?state=settings&section=${section}`), {
      kind: 'settings',
      section,
    });
    // Written back out, each is the same address.
    const location = { kind: 'settings', section } as const;
    const href = locationHref('http://localhost/?state=all', location);
    assert.deepEqual(decodeLocation(new URL(href).search), location, href);
  }
  // A `section=` naming nothing this build has — never valid, or since
  // retired for a reason not in `RETIRED_SETTINGS_SECTIONS` — drops the
  // section rather than refusing the address; the tab opens on its default
  // page instead of erroring.
  assert.deepEqual(decodeLocation('?state=settings&section=display'), {
    kind: 'settings',
  });
});

test('the aliases for retired pages point at the tabs that replaced them', () => {
  assert.deepEqual(decodeLocation('?state=alerts'), { kind: 'people' });
  for (const state of ['join', 'groups', 'create'])
    assert.deepEqual(
      decodeLocation(`?state=${state}`),
      { kind: 'teams' },
      state,
    );
  assert.deepEqual(decodeLocation('?state=settings-macs'), {
    kind: 'devices',
    section: 'macs',
  });
  assert.deepEqual(decodeLocation('?state=settings-phrase'), {
    kind: 'devices',
    section: 'macs',
  });
  assert.deepEqual(decodeLocation('?state=settings-enrol'), {
    kind: 'devices',
    section: 'keys',
  });
  // An explicit store still overrides the alias's own account.
  assert.deepEqual(decodeLocation('?state=settings-account&store=acct:work'), {
    kind: 'people',
    store: 'acct:work',
  });
  // `team-chat` was the chat location before the team column named the team
  // on `chat` itself. Its deep links keep working.
  assert.deepEqual(decodeLocation('?state=team-chat&store=team:eng'), {
    kind: 'chat',
    ref: 'team:eng',
  });
  assert.deepEqual(
    decodeLocation(
      `?state=team-chat&store=team:eng&channel=${'ab'.repeat(16)}`,
    ),
    { kind: 'chat', ref: 'team:eng', channel: 'ab'.repeat(16) },
  );
  // A team-chat link with no team named no place at all.
  assert.equal(decodeLocation('?state=team-chat'), null);
  // A channel that is not a channel identity is rejected on either name.
  assert.equal(decodeLocation('?state=chat&store=team:eng&channel=zz'), null);
  // A channel belongs to the team that names it: with no team the tab resolves
  // one, whose channels this identifier is not.
  assert.deepEqual(decodeLocation(`?state=chat&channel=${'ab'.repeat(16)}`), {
    kind: 'chat',
  });
});

/* ------------------------------------------------- Settings by StoreRef -- */

/** Sample JSON StoreRef strings representing native account stores across profiles. */
const PERSONAL_ON_A =
  '{"kind":"account","profile":"personal","accountAlias":"personal"}';
const PERSONAL_ON_B =
  '{"kind":"account","profile":"acme","accountAlias":"personal"}';

test('a tab location round-trips JSON StoreRef query parameters', () => {
  for (const store of [
    PERSONAL_ON_A,
    PERSONAL_ON_B,
    // Verify query encoding with special characters, quotes, and Unicode strings.
    '{"kind":"account","profile":"100%","accountAlias":"a b"}',
    '{"kind":"account","profile":"say \\"hi\\"","accountAlias":"q"}',
    '{"kind":"account","profile":"Ünïcøde","accountAlias":"個人"}',
    '{"kind":"account","profile":"a&b=c#d","accountAlias":"e+f"}',
  ]) {
    const location: Location = { kind: 'devices', section: 'macs', store };
    const href = locationHref('http://localhost/', location);
    assert.deepEqual(decodeLocation(new URL(href).search), location, store);
    assert.equal(new URL(href).searchParams.get('store'), store);
  }
});

test('settings location without store parameter round-trips correctly', () => {
  for (const location of [
    { kind: 'settings' } as const,
    { kind: 'devices', section: 'keys' } as const,
  ]) {
    const href = locationHref('http://localhost/', location);
    assert.deepEqual(decodeLocation(new URL(href).search), location);
    assert.equal(new URL(href).searchParams.get('store'), null);
  }
});

test('sameLocation distinguishes accounts with identical aliases across profiles', () => {
  assert.equal(
    sameLocation(
      { kind: 'devices', section: 'macs', store: PERSONAL_ON_A },
      { kind: 'devices', section: 'macs', store: PERSONAL_ON_B },
    ),
    false,
  );
  assert.equal(
    sameLocation(
      { kind: 'devices', section: 'macs', store: PERSONAL_ON_B },
      { kind: 'devices', section: 'macs', store: PERSONAL_ON_B },
    ),
    true,
  );
  // A settings location without a store must not match one with a store.
  assert.equal(
    sameLocation(
      { kind: 'devices', section: 'macs' },
      { kind: 'devices', section: 'macs', store: PERSONAL_ON_A },
    ),
    false,
  );
});

test('deprecated account query parameter is ignored', () => {
  assert.deepEqual(
    decodeLocation('?state=settings&section=macs&account=work'),
    {
      kind: 'devices',
      section: 'macs',
    },
  );
  // When both store and legacy account parameters are present, store takes precedence.
  assert.deepEqual(
    decodeLocation(
      '?state=settings&section=macs&account=work&store=acct:personal',
    ),
    { kind: 'devices', section: 'macs', store: 'acct:personal' },
  );
  // Encoding a location strips legacy account parameters.
  const href = locationHref('http://localhost/?state=settings&account=work', {
    kind: 'devices',
    section: 'macs',
    store: 'acct:personal',
  });
  assert.equal(new URL(href).searchParams.get('account'), null);
});

test('navigating away from settings removes store and account parameters', () => {
  const from =
    'http://localhost/?state=settings&section=macs&store=acct%3Awork&account=work';
  for (const location of [
    { kind: 'all' } as const,
    { kind: 'people' } as const,
  ]) {
    const url = new URL(locationHref(from, location));
    assert.equal(url.searchParams.get('account'), null, location.kind);
    assert.equal(url.searchParams.get('section'), null, location.kind);
    assert.equal(url.searchParams.get('store'), null, location.kind);
  }
  // Store navigation replaces existing store and account parameters.
  const store = new URL(locationHref(from, { kind: 'store', ref: 'team:eng' }));
  assert.equal(store.searchParams.get('store'), 'team:eng');
  assert.equal(store.searchParams.get('account'), null);
});

test('encoding a new location strips unrelated query parameters', () => {
  const busy =
    'http://localhost/?state=first-run&store=team:eng&section=macs&profile=acme&step=who&path=own&account=work';
  const url = new URL(locationHref(busy, { kind: 'all' }));
  for (const parameter of [
    'store',
    'section',
    'profile',
    'step',
    'path',
    'account',
  ]) {
    assert.equal(url.searchParams.get(parameter), null, parameter);
  }
});

test('settings scene aliases map to specific account stores', () => {
  assert.deepEqual(decodeLocation('?state=settings-macs-work'), {
    kind: 'devices',
    section: 'macs',
    store: 'acct:work',
  });
  // An explicit store parameter overrides the scene default.
  assert.deepEqual(
    decodeLocation('?state=settings-macs-work&store=acct:personal'),
    { kind: 'devices', section: 'macs', store: 'acct:personal' },
  );
  // Scenes without default account mappings omit the store parameter.
  assert.deepEqual(decodeLocation('?state=settings-keys'), {
    kind: 'devices',
    section: 'keys',
  });
  assert.deepEqual(decodeLocation('?state=settings-agent'), {
    kind: 'settings',
    section: 'mac',
  });
  assert.deepEqual(decodeLocation('?state=settings-about'), {
    kind: 'settings',
    section: 'mac',
  });
  assert.deepEqual(decodeLocation('?state=settings&section=agent'), {
    kind: 'settings',
    section: 'mac',
  });
});

test('the six former Settings sections resolve to the three pages that hold them', () => {
  // Account (the passphrase rows) and Notifications are both Preferences;
  // This device and About both described this Mac; Security keys was a list
  // of links to the server pages, so it is Servers.
  for (const [former, section] of [
    ['credentials', 'preferences'],
    ['notifications', 'preferences'],
    ['device', 'mac'],
    ['about', 'mac'],
    ['agent', 'mac'],
    ['security-keys', 'servers'],
  ] as const) {
    assert.deepEqual(
      decodeLocation(`?state=settings&section=${former}`),
      { kind: 'settings', section },
      former,
    );
    // The account the address named comes with it.
    assert.deepEqual(
      decodeLocation(`?state=settings&section=${former}&store=acct:work`),
      { kind: 'settings', section, store: 'acct:work' },
      former,
    );
  }
  // A `security-keys` address pointed at a server row, not at a server: its
  // `profile` opens nothing, so the address lands on the Servers list.
  assert.deepEqual(
    decodeLocation('?state=settings&section=security-keys&profile=acme'),
    { kind: 'settings', section: 'servers' },
  );
});

test('group creation scenes default to engineering vault context', () => {
  for (const state of ['group-new-document']) {
    assert.deepEqual(
      decodeScene(`?state=${state}`).location,
      { kind: 'store', ref: 'team:eng' },
      state,
    );
  }
});

test("the mock's own state names still deep-link", () => {
  // Stable deep links used by the Playwright walk keep working.
  assert.deepEqual(decodeLocation('?state=all'), { kind: 'all' });
  assert.deepEqual(decodeLocation('?state=personal'), {
    kind: 'store',
    ref: 'acct:personal',
  });
  assert.deepEqual(decodeLocation('?state=group'), {
    kind: 'store',
    ref: 'team:household',
  });
  assert.deepEqual(decodeLocation('?state=household'), {
    kind: 'store',
    ref: 'team:household',
  });
  assert.deepEqual(decodeLocation('?state=alerts'), { kind: 'people' });
  assert.deepEqual(decodeLocation('?state=servers'), {
    kind: 'settings',
    section: 'servers',
  });
});

test('removed group tabs route to People or the group vault', () => {
  assert.deepEqual(decodeLocation('?state=federation'), {
    kind: 'group-settings',
    ref: 'team:eng',
    tab: 'people',
  });
  // `people` is the People tab. The mock's name for a *group's* People tab is
  // `group-people`, alongside `party` and `federation`.
  assert.deepEqual(decodeLocation('?state=people'), { kind: 'people' });
  assert.deepEqual(decodeLocation('?state=people&store=acct:work'), {
    kind: 'people',
    store: 'acct:work',
  });
  assert.deepEqual(decodeLocation('?state=group-people'), {
    kind: 'group-settings',
    ref: 'team:eng',
    tab: 'people',
  });
  assert.deepEqual(decodeLocation('?state=party'), {
    kind: 'group-settings',
    ref: 'team:eng',
    tab: 'people',
  });
  assert.deepEqual(decodeLocation('?state=items'), {
    kind: 'store',
    ref: 'team:eng',
  });
  assert.deepEqual(
    decodeLocation('?state=group-settings&store=team:eng&tab=federation'),
    { kind: 'group-settings', ref: 'team:eng' },
  );
});

test('a group page addresses each of its four tabs', () => {
  for (const tab of ['people', 'channels', 'files', 'settings'] as const)
    assert.deepEqual(
      decodeLocation(`?state=group-settings&store=team:eng&tab=${tab}`),
      { kind: 'group-settings', ref: 'team:eng', tab },
    );
  // The two tabs the group page grew have their own mock state names, and a
  // `tab=` on an alias still wins over the tab the alias names. Channels
  // opens on the group that has channels: Engineering's server offers no
  // chat, so that scene would be the reason rather than the tab.
  assert.deepEqual(decodeLocation('?state=group-channels'), {
    kind: 'group-settings',
    ref: 'team:household',
    tab: 'channels',
  });
  assert.deepEqual(decodeLocation('?state=group-files'), {
    kind: 'group-settings',
    ref: 'team:eng',
    tab: 'files',
  });
  assert.deepEqual(decodeLocation('?state=group-people&tab=files'), {
    kind: 'group-settings',
    ref: 'team:eng',
    tab: 'files',
  });
});

test('path-specific first-run review states keep the path the mock defines', () => {
  assert.deepEqual(decodeLocation('?state=waiting&path=own'), {
    kind: 'first-run',
    step: 'waiting',
    path: 'invited',
  });
  assert.equal(decodeLocation('?state=create-group&path=invited'), null);
  assert.equal(decodeLocation('?state=done&path=own'), null);
});

test('unrecognized state parameter values decode to null', () => {
  // Overlay states (e.g. modals, dialogs) do not represent standalone navigation locations.
  for (const state of ['new', 'manage', 'conflict', 'exists', 'show', 'grid']) {
    assert.equal(decodeLocation(`?state=${state}`), null, state);
  }
  assert.equal(decodeLocation(''), null);
  assert.equal(decodeLocation('?other=1'), null);
  // A store state missing a store parameter is invalid and returns null.
  assert.equal(decodeLocation('?state=store'), null);
  // An unknown settings section falls back to the pane itself, not to null.
  assert.deepEqual(decodeLocation('?state=settings&section=nope'), {
    kind: 'settings',
  });
});

test('getState and setUrl behave as the mock s helpers do', () => {
  assert.equal(getState('?state=all', 'boot'), 'all');
  assert.equal(getState('', 'boot'), 'boot');
  assert.equal(getState('?path=own', 'invited', 'path'), 'own');

  assert.equal(
    setUrl('http://localhost/?state=who&path=own', 'added'),
    'http://localhost/?state=added&path=own',
  );
  assert.equal(
    setUrl('http://localhost/?state=who&path=own', 'added', { path: null }),
    'http://localhost/?state=added',
  );
});

test('encodeLocation names the state the address carries', () => {
  assert.equal(encodeLocation({ kind: 'all' }).state, 'all');
  assert.equal(encodeLocation({ kind: 'store', ref: 'x' }).params.store, 'x');
  assert.equal(
    encodeLocation({ kind: 'first-run', step: 'protect' }).params.step,
    'protect',
  );
});

/* ----------------------------------------------------------------- scenes -- */

test('the mock s state names carry what is not a location', () => {
  // View mode, lease status, and selection are decoded at the scene layer rather than as standalone locations.
  // `grid` was a view mode before the tree replaced the toggle; it now
  // resolves to the one view left, so this scene is the initial one.
  assert.deepEqual(decodeScene('?state=grid'), INITIAL_SCENE);
  assert.deepEqual(decodeScene('?state=lease'), {
    ...INITIAL_SCENE,
    location: { kind: 'store', ref: 'acct:work' },
    lease: 'lapsed',
  });
  assert.deepEqual(decodeScene('?state=inactive'), {
    ...INITIAL_SCENE,
    location: { kind: 'store', ref: 'team:homelab' },
  });
  assert.deepEqual(decodeScene('?state=alerts'), {
    ...INITIAL_SCENE,
    location: { kind: 'people' },
    lease: 'lapsed',
  });
  assert.deepEqual(decodeScene('?state=show'), {
    ...INITIAL_SCENE,
    demo: 'password',
    reveal: true,
  });
});

test('full scene state round-trips through URL serialization', () => {
  const scenes = [
    INITIAL_SCENE,
    {
      ...INITIAL_SCENE,
      location: { kind: 'store' as const, ref: 'team:eng' },
      selection: { store: 'team:eng', path: '/deploy/production-token' },
      kind: 'Password' as const,
      sort: 'group' as const,
      folder: '/deploy',
      closedFolders: ['team:eng|/deploy/archive'],
      lease: 'lapsed' as const,
    },
    {
      ...INITIAL_SCENE,
      location: { kind: 'settings' as const, section: 'mac' as const },
    },
  ];
  for (const scene of scenes) {
    const href = sceneHref('http://localhost/', scene);
    assert.deepEqual(decodeScene(new URL(href).search), scene, href);
  }
});

test('default scene serializes to state query parameter without extra parameters', () => {
  assert.equal(
    sceneHref('http://localhost/', INITIAL_SCENE),
    'http://localhost/?state=all',
  );
  // Explicit query parameter overrides conflicting scene alias.
  assert.equal(decodeScene('?state=grid&view=list').view, 'list');
  // Unrecognized parameter values fall back to scene defaults.
  assert.equal(
    decodeScene('?state=all&sort=sideways').sort,
    INITIAL_SCENE.sort,
  );
  assert.equal(decodeScene('?state=all&sel=nostore').selection, null);
});

/* ------------------------------------------------------------------ store -- */

test('LocationStore notifies subscribers only when state changes', () => {
  const store = new LocationStore();
  let notifications = 0;
  const unsubscribe = store.subscribe(() => {
    notifications += 1;
  });

  assert.deepEqual(store.getSnapshot(), INITIAL_STATE);
  store.navigate({ kind: 'people' });
  assert.equal(notifications, 1);
  assert.deepEqual(store.getSnapshot().location, { kind: 'people' });

  // Redundant navigation returns identical state reference, skipping subscriber notification.
  store.navigate({ kind: 'people' });
  assert.equal(notifications, 1);

  store.dispatch({ type: 'search', query: 'wifi' });
  assert.equal(notifications, 2);
  assert.equal(store.getSnapshot().query, 'wifi');

  unsubscribe();
  store.navigate({ kind: 'all' });
  assert.equal(notifications, 2);
});

test('getSnapshot is stable across reads, as useSyncExternalStore requires', () => {
  const store = new LocationStore();
  assert.equal(store.getSnapshot(), store.getSnapshot());
  store.navigate({ kind: 'people' });
  assert.equal(store.getSnapshot(), store.getSnapshot());
});

test('account context follows objects and persists across cross-account tabs', async () => {
  const { FIXTURE } = await import('../src/fixture');
  const navigation = new LocationStore();
  navigation.setAccountStores(FIXTURE.stores);
  navigation.navigate({ kind: 'people', store: 'acct:work' });
  navigation.navigate({ kind: 'files' });
  navigation.navigate({ kind: 'devices' });
  assert.deepEqual(navigation.getSnapshot().location, {
    kind: 'devices',
    store: 'acct:work',
  });
  navigation.navigate({ kind: 'settings', section: 'preferences' });
  assert.equal(navigation.getAccount(), 'acct:work');
  navigation.navigate({ kind: 'chat', ref: 'team:household' });
  assert.equal(navigation.getAccount(), 'acct:personal');
  navigation.navigate({ kind: 'teams' });
  assert.deepEqual(navigation.getSnapshot().location, {
    kind: 'teams',
    store: 'acct:personal',
  });
  navigation.navigate({ kind: 'group-settings', ref: 'team:eng' });
  assert.equal(navigation.getAccount(), 'acct:work');
  navigation.navigate({ kind: 'files' });
  navigation.setAccountStores(
    FIXTURE.stores.filter((store) => store.account !== 'work'),
  );
  navigation.navigate({ kind: 'devices' });
  assert.equal(navigation.getAccount(), 'acct:personal');
});

test('rail tabs resume folders and sections without reopening item details', async () => {
  const { FIXTURE } = await import('../src/fixture');
  const navigation = new LocationStore();
  navigation.setAccountStores(FIXTURE.stores);
  navigation.navigate({ kind: 'store', ref: 'acct:work' });
  navigation.setFolder('/deploy');
  navigation.search('token');
  navigation.select({ store: 'acct:work', path: '/deploy/token' });
  navigation.navigateTab('settings');
  navigation.navigate({ kind: 'settings', section: 'preferences' });
  navigation.navigateTab('files');
  assert.deepEqual(navigation.getSnapshot().location, {
    kind: 'store',
    ref: 'acct:work',
  });
  assert.equal(navigation.getSnapshot().folder, '/deploy');
  assert.equal(navigation.getSnapshot().query, 'token');
  assert.equal(navigation.getSnapshot().selection, null);
  assert.equal(navigation.getSnapshot().details, false);
  navigation.navigateTab('settings');
  assert.deepEqual(navigation.getSnapshot().location, {
    kind: 'settings',
    section: 'preferences',
    store: 'acct:work',
  });
  navigation.clearTabMemory();
  navigation.navigateTab('files');
  assert.deepEqual(navigation.getSnapshot().location, { kind: 'files' });
});

test('remembered device pages follow a new acting account without reusing a key id', async () => {
  const { FIXTURE } = await import('../src/fixture');
  const navigation = new LocationStore();
  navigation.setAccountStores(FIXTURE.stores);
  navigation.navigate({
    kind: 'devices',
    store: 'acct:work',
    device: 'old-key',
  });
  navigation.navigate({ kind: 'people', store: 'acct:personal' });
  navigation.navigateTab('devices');
  assert.deepEqual(navigation.getSnapshot().location, {
    kind: 'devices',
    store: 'acct:personal',
  });
});

/* ---------------------------------------------------------------- parents -- */

test('a page inside a tab knows the page it returns to', () => {
  // All items is the Files tab's own root, whichever address names it.
  assert.equal(parentLocation({ kind: 'all' }), null);
  assert.equal(parentLocation({ kind: 'files' }), null);
  assert.deepEqual(parentLocation({ kind: 'store', ref: 'team:eng' }), {
    kind: 'files',
  });
  assert.deepEqual(
    parentLocation({ kind: 'group-settings', ref: 'team:eng', tab: 'people' }),
    { kind: 'teams' },
  );
  // A device's own page returns to the list, keeping the account it was read
  // through and the section it was listed under.
  assert.deepEqual(
    parentLocation({
      kind: 'devices',
      store: 'acct:work',
      section: 'keys',
      device: 'yubi:work',
    }),
    { kind: 'devices', section: 'keys', store: 'acct:work' },
  );
  assert.deepEqual(
    parentLocation({ kind: 'devices', store: 'acct:work', section: 'keys' }),
    { kind: 'devices', store: 'acct:work' },
  );
  assert.deepEqual(
    parentLocation({
      kind: 'settings',
      section: 'servers',
      profile: 'personal',
      store: 'acct:personal',
    }),
    { kind: 'settings', store: 'acct:personal' },
  );
});

test('a tab root has no parent, and neither does an account parameter', () => {
  for (const location of [
    { kind: 'files' },
    { kind: 'teams', store: 'acct:work' },
    { kind: 'people', store: 'acct:personal' },
    { kind: 'devices', store: 'acct:work' },
    { kind: 'settings', store: 'acct:work' },
    { kind: 'chat', ref: 'team:household' },
    { kind: 'first-run', step: 'who' },
  ] as Location[])
    assert.equal(parentLocation(location), null, `${location.kind} is a root`);
});

/* ---------------------------------------------------------------- guards -- */

/** Lets the prompter's promise and the dispatch behind it settle. */
const settled = (): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, 0));

/** A guard that prompts for every intent, with a countable `onConfirm`. */
function prompting(onConfirm?: () => void): NavigationGuard {
  return () => ({
    verdict: 'prompt',
    title: 'Discard unsaved changes?',
    body: 'Your draft has not been saved.',
    confirm: 'Discard changes',
    ...(onConfirm ? { onConfirm } : {}),
  });
}

/** Collects the prompts raised, handing back the answer for each. */
function prompts(): {
  asked: string[];
  answer: (index: number, confirmed: boolean) => void;
  prompter: NavigationPrompter;
} {
  const asked: string[] = [];
  const answers: ((confirmed: boolean) => void)[] = [];
  return {
    asked,
    answer: (index, confirmed) => answers[index](confirmed),
    prompter: (verdict) => {
      asked.push(verdict.title);
      return new Promise<boolean>((resolve) => answers.push(resolve));
    },
  };
}

test('with no guards registered a navigation is unchanged', () => {
  const store = new LocationStore();
  store.navigate({ kind: 'files' });
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
  assert.equal(
    store.navigationVerdict({
      kind: 'navigate',
      location: at({ kind: 'teams' }).location,
    }),
    null,
  );
});

test('guards are asked in registration order and the first verdict decides', () => {
  const store = new LocationStore();
  const asked: string[] = [];
  const refused: string[] = [];
  store.setRefusalHandler((reason) => refused.push(reason));
  store.registerGuard(() => {
    asked.push('first');
    return null;
  });
  store.registerGuard(() => {
    asked.push('second');
    return { verdict: 'refuse', reason: 'A key is being enrolled.' };
  });
  store.registerGuard(() => {
    asked.push('third');
    return { verdict: 'refuse', reason: 'never reached' };
  });
  store.navigate({ kind: 'files' });
  // The third guard is never asked, and nothing moved.
  assert.deepEqual(asked, ['first', 'second']);
  assert.deepEqual(refused, ['A key is being enrolled.']);
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
});

test('a refusal from a later guard outranks an earlier prompt', () => {
  const store = new LocationStore();
  const refused: string[] = [];
  let prompted = 0;
  store.setRefusalHandler((reason) => refused.push(reason));
  store.setPrompter(() => {
    prompted++;
    return Promise.resolve(true);
  });
  store.registerGuard(() => ({
    verdict: 'prompt',
    title: 'Discard message?',
    body: 'Your unsent message will be lost.',
    confirm: 'Discard',
  }));
  store.registerGuard(() => ({
    verdict: 'refuse',
    reason: 'Wait for the channel to finish being created.',
  }));
  assert.equal(
    store.navigationVerdict({ kind: 'navigate', location: { kind: 'files' } })
      ?.verdict,
    'refuse',
  );
  store.navigate({ kind: 'files' });
  assert.equal(prompted, 0);
  assert.deepEqual(refused, ['Wait for the channel to finish being created.']);
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
});

test('unregistering a guard takes it out of the order', () => {
  const store = new LocationStore();
  const off = store.registerGuard(() => ({
    verdict: 'refuse',
    reason: 'no',
  }));
  store.navigate({ kind: 'files' });
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
  off();
  store.navigate({ kind: 'files' });
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
});

test('a confirmed prompt runs onConfirm, then the navigation', async () => {
  const store = new LocationStore();
  const order: string[] = [];
  store.registerGuard(prompting(() => order.push('onConfirm')));
  const { asked, answer, prompter } = prompts();
  store.setPrompter(prompter);
  store.navigate({ kind: 'files' });
  // The prompt is up and nothing has moved yet.
  assert.deepEqual(asked, ['Discard unsaved changes?']);
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
  answer(0, true);
  await settled();
  assert.deepEqual(order, ['onConfirm']);
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
});

test('a cancelled prompt leaves the location and onConfirm alone', async () => {
  const store = new LocationStore();
  let confirms = 0;
  store.registerGuard(prompting(() => (confirms += 1)));
  const { answer, prompter } = prompts();
  store.setPrompter(prompter);
  store.navigate({ kind: 'files' });
  answer(0, false);
  await settled();
  assert.equal(confirms, 0);
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
});

test('a prompt with no prompter installed allows the navigation', () => {
  const store = new LocationStore();
  store.registerGuard(prompting());
  store.navigate({ kind: 'files' });
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
});

test('force bypasses the guards on navigate, navigateTab and select', () => {
  const store = new LocationStore();
  let asked = 0;
  store.registerGuard(() => {
    asked += 1;
    return { verdict: 'refuse', reason: 'no' };
  });
  store.navigate({ kind: 'files' }, { force: true });
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
  store.select({ store: 'team:eng', path: '/a' }, { force: true });
  assert.deepEqual(store.getSnapshot().selection, {
    store: 'team:eng',
    path: '/a',
  });
  store.navigateTab('teams', { force: true });
  // No inventory is loaded here, so the tab carries no acting account.
  assert.deepEqual(store.getSnapshot().location, {
    kind: 'teams',
    store: undefined,
  });
  assert.equal(asked, 0);
});

test('a rail tab and a selection are guarded like any other move', () => {
  const store = new LocationStore();
  const seen: NavigationIntent[] = [];
  store.registerGuard((intent) => {
    seen.push(intent);
    return { verdict: 'refuse', reason: 'no' };
  });
  store.navigateTab('teams');
  store.select({ store: 'team:eng', path: '/a' });
  assert.deepEqual(seen, [
    {
      kind: 'navigate',
      location: { kind: 'teams', store: undefined },
      tab: true,
    },
    { kind: 'select', selection: { store: 'team:eng', path: '/a' } },
  ]);
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
  assert.equal(store.getSnapshot().selection, null);
});

test('navigationVerdict answers without moving or prompting', () => {
  const store = new LocationStore();
  store.registerGuard(prompting());
  const { asked, prompter } = prompts();
  store.setPrompter(prompter);
  const verdict = store.navigationVerdict({
    kind: 'navigate',
    location: { kind: 'files' },
  });
  assert.equal(verdict?.verdict, 'prompt');
  assert.deepEqual(asked, []);
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
});

test('a second navigation supersedes the prompt already up', async () => {
  const store = new LocationStore();
  store.registerGuard(prompting());
  const { asked, answer, prompter } = prompts();
  store.setPrompter(prompter);
  store.navigate({ kind: 'files' });
  store.navigate({ kind: 'teams' });
  // The newer intent is asked about in its own right.
  assert.equal(asked.length, 2);
  // Confirming the superseded prompt moves nothing: its intent is gone.
  answer(0, true);
  await settled();
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
  answer(1, true);
  await settled();
  assert.deepEqual(store.getSnapshot().location, { kind: 'teams' });
});

test('an item opens its page and selects on it, or neither', async () => {
  const store = new LocationStore();
  store.registerGuard(prompting());
  const { answer, prompter } = prompts();
  store.setPrompter(prompter);
  store.navigateAndSelect(
    { kind: 'store', ref: 'team:eng' },
    { store: 'team:eng', path: '/deploy/token' },
  );
  // Neither half is applied while the question is open.
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
  assert.equal(store.getSnapshot().selection, null);
  answer(0, true);
  await settled();
  assert.deepEqual(store.getSnapshot().location, {
    kind: 'store',
    ref: 'team:eng',
  });
  assert.deepEqual(store.getSnapshot().selection, {
    store: 'team:eng',
    path: '/deploy/token',
  });
});

test('a chat channel returns to its team inbox, whose back target is disabled', () => {
  const parent = parentLocation({
    kind: 'chat',
    ref: 'team:eng',
    channel: 'ab'.repeat(16),
  });
  assert.deepEqual(parent, { kind: 'chat', ref: 'team:eng' });
  assert.equal(parentLocation(parent), null);
});

test('tab sheet drafts stay in memory and explicit navigation clears them', () => {
  const store = new LocationStore(at({ kind: 'files' }));
  store.setSheetField('new.site', 'draft.example');
  const address = encodeLocation(store.getSnapshot().location);
  assert.equal(JSON.stringify(address).includes('draft.example'), false);
  store.navigateTab('teams');
  store.navigateTab('files');
  assert.deepEqual(store.getSnapshot().sheet, { 'new.site': 'draft.example' });
  store.navigate({ kind: 'all' });
  assert.equal(store.getSnapshot().sheet, undefined);
});

test('an unsafe sheet cannot leave a restorable draft when navigation applies', () => {
  const store = new LocationStore(at({ kind: 'files' }));
  const id = Symbol();
  store.setSheetField('new.site', 'draft.example');
  store.setSheetRestorable(id, false);
  store.navigationVerdict({
    kind: 'navigate',
    location: { kind: 'teams' },
    tab: true,
  });
  assert.ok(store.getSnapshot().sheet, 'asking for a verdict is pure');
  store.navigateTab('teams');
  store.setSheetRestorable(id, undefined);
  store.navigateTab('files');
  assert.equal(store.getSnapshot().sheet, undefined);
});
