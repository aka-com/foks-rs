/**
 * The navigation model: pure transitions and the `?state=` round trip.
 *
 * The URL codec is the acceptance layer's entry point — the Playwright walk
 * loads each state by address — so a change here that breaks a deep link
 * breaks the gate every later phase leans on.
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
  sameLocation,
  setUrl,
  transition,
} from '../src/location';
import type { Location, LocationState } from '../src/location';

const at = (location: Location): LocationState => ({
  ...INITIAL_STATE,
  location,
});

/* ------------------------------------------------------------ transitions -- */

test('navigating somewhere else drops the selection', () => {
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
  // The search box is not cleared: the person is still looking for the same
  // thing.
  assert.equal(next.query, 'token');
});

test('navigating to where you already are keeps the selection', () => {
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

test('a no-op action returns the same state object', () => {
  const state = at({ kind: 'all' });
  assert.equal(transition(state, { type: 'search', query: '' }), state);
  assert.notEqual(transition(state, { type: 'search', query: 'wifi' }), state);
});

test('selecting and searching leave the location alone', () => {
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

test('sameLocation compares the payload, not just the kind', () => {
  assert.equal(
    sameLocation({ kind: 'store', ref: 'a' }, { kind: 'store', ref: 'a' }),
    true,
  );
  assert.equal(
    sameLocation({ kind: 'store', ref: 'a' }, { kind: 'store', ref: 'b' }),
    false,
  );
  assert.equal(
    sameLocation({ kind: 'settings' }, { kind: 'settings', section: 'agent' }),
    false,
  );
  assert.equal(
    sameLocation({ kind: 'first-run', step: 'who' }, { kind: 'first-run', step: 'who' }),
    true,
  );
  assert.equal(sameLocation({ kind: 'all' }, { kind: 'issues' }), false);
});

/* -------------------------------------------------------------- URL codec -- */

const ROUND_TRIP: Location[] = [
  { kind: 'all' },
  { kind: 'store', ref: 'acct:personal' },
  { kind: 'store', ref: 'team:eng' },
  { kind: 'issues' },
  { kind: 'join' },
  { kind: 'groups' },
  { kind: 'group-admin', ref: 'team:eng' },
  { kind: 'servers' },
  { kind: 'settings' },
  { kind: 'settings', section: 'agent' },
  { kind: 'first-run', step: 'who' },
];

test('every location round-trips through the address bar', () => {
  for (const location of ROUND_TRIP) {
    const href = locationHref('http://localhost/', location);
    const decoded = decodeLocation(new URL(href).search);
    assert.deepEqual(decoded, location, href);
  }
});

test('encoding clears the parameters the previous location left behind', () => {
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

test('Phase 6 aliases do not collide with Groups or first-run states', () => {
  assert.deepEqual(decodeLocation('?state=servers-add'), { kind: 'servers' });
  assert.deepEqual(decodeLocation('?state=settings-account'), {
    kind: 'settings',
    section: 'account',
  });
  assert.deepEqual(decodeLocation('?state=add'), {
    kind: 'group-admin',
    ref: 'team:eng',
  });
  assert.deepEqual(decodeLocation('?state=account&path=own'), {
    kind: 'first-run',
    step: 'account',
    path: 'own',
  });
  assert.deepEqual(
    decodeLocation('?state=settings&section=macs&store=acct:work'),
    { kind: 'settings', section: 'macs', store: 'acct:work' },
  );
});

/* ------------------------------------------------- Settings by StoreRef -- */

/**
 * The identity a native account store is written with. `store_id` in
 * `foks-tauri/src/commands.rs` serialises a JSON object, so a real StoreRef
 * carries braces, quotes, colons and commas through the address bar; the two
 * below differ only in their profile, which is exactly the collision an alias
 * cannot express.
 */
const PERSONAL_ON_A =
  '{"kind":"account","profile":"personal","accountAlias":"personal"}';
const PERSONAL_ON_B =
  '{"kind":"account","profile":"acme","accountAlias":"personal"}';

test('Settings round-trips a native StoreRef through the address bar', () => {
  for (const store of [
    PERSONAL_ON_A,
    PERSONAL_ON_B,
    // Percent-sign-like text, a quote inside a value, and a Unicode profile
    // and alias. `URLSearchParams` does the encoding; nothing pre-encodes.
    '{"kind":"account","profile":"100%","accountAlias":"a b"}',
    '{"kind":"account","profile":"say \\"hi\\"","accountAlias":"q"}',
    '{"kind":"account","profile":"Ünïcøde","accountAlias":"個人"}',
    '{"kind":"account","profile":"a&b=c#d","accountAlias":"e+f"}',
  ]) {
    const location: Location = { kind: 'settings', section: 'macs', store };
    const href = locationHref('http://localhost/', location);
    assert.deepEqual(decodeLocation(new URL(href).search), location, store);
    assert.equal(new URL(href).searchParams.get('store'), store);
  }
});

test('Settings without a store round-trips as Settings without a store', () => {
  for (const location of [
    { kind: 'settings' } as const,
    { kind: 'settings', section: 'keys' } as const,
  ]) {
    const href = locationHref('http://localhost/', location);
    assert.deepEqual(decodeLocation(new URL(href).search), location);
    assert.equal(new URL(href).searchParams.get('store'), null);
  }
});

test('sameLocation tells two accounts with the same alias apart', () => {
  assert.equal(
    sameLocation(
      { kind: 'settings', section: 'macs', store: PERSONAL_ON_A },
      { kind: 'settings', section: 'macs', store: PERSONAL_ON_B },
    ),
    false,
  );
  assert.equal(
    sameLocation(
      { kind: 'settings', section: 'macs', store: PERSONAL_ON_B },
      { kind: 'settings', section: 'macs', store: PERSONAL_ON_B },
    ),
    true,
  );
  // A Settings page with no account chosen is not the page for one.
  assert.equal(
    sameLocation(
      { kind: 'settings', section: 'macs' },
      { kind: 'settings', section: 'macs', store: PERSONAL_ON_A },
    ),
    false,
  );
});

test('the retired account parameter is ignored rather than resolved', () => {
  assert.deepEqual(decodeLocation('?state=settings&section=macs&account=work'), {
    kind: 'settings',
    section: 'macs',
  });
  // An address carrying both reads only the exact one.
  assert.deepEqual(
    decodeLocation('?state=settings&section=macs&account=work&store=acct:personal'),
    { kind: 'settings', section: 'macs', store: 'acct:personal' },
  );
  // And encoding a Settings location clears the obsolete parameter.
  const href = locationHref('http://localhost/?state=settings&account=work', {
    kind: 'settings',
    section: 'macs',
    store: 'acct:personal',
  });
  assert.equal(new URL(href).searchParams.get('account'), null);
});

test('leaving Settings clears the account it was on', () => {
  const from = 'http://localhost/?state=settings&section=macs&store=acct%3Awork&account=work';
  for (const location of [
    { kind: 'all' } as const,
    { kind: 'groups' } as const,
    { kind: 'join' } as const,
    { kind: 'servers', profile: 'personal' } as const,
  ]) {
    const url = new URL(locationHref(from, location));
    assert.equal(url.searchParams.get('account'), null, location.kind);
    assert.equal(url.searchParams.get('section'), null, location.kind);
    if (location.kind !== 'servers') {
      assert.equal(url.searchParams.get('store'), null, location.kind);
    }
  }
  // And a store location's own ref replaces it rather than joining it.
  const store = new URL(locationHref(from, { kind: 'store', ref: 'team:eng' }));
  assert.equal(store.searchParams.get('store'), 'team:eng');
  assert.equal(store.searchParams.get('account'), null);
});

test('a location clears the parameters every other location owns', () => {
  const busy =
    'http://localhost/?state=first-run&store=team:eng&section=macs&profile=acme&step=who&path=own&account=work';
  const url = new URL(locationHref(busy, { kind: 'all' }));
  for (const parameter of ['store', 'section', 'profile', 'step', 'path', 'account']) {
    assert.equal(url.searchParams.get(parameter), null, parameter);
  }
});

test('the named Settings scenes name an exact account store', () => {
  assert.deepEqual(decodeLocation('?state=settings-macs-work'), {
    kind: 'settings',
    section: 'macs',
    store: 'acct:work',
  });
  // An explicit store in the address beats the scene's default.
  assert.deepEqual(
    decodeLocation('?state=settings-macs-work&store=acct:personal'),
    { kind: 'settings', section: 'macs', store: 'acct:personal' },
  );
  // The scenes that name no account still name none.
  assert.deepEqual(decodeLocation('?state=settings-keys'), {
    kind: 'settings',
    section: 'keys',
  });
});

test('Phase 7 group-create scenes retain Engineering as their vault context', () => {
  for (const state of ['group-new-text', 'group-new-link', 'group-new-file']) {
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
  assert.deepEqual(decodeLocation('?state=issues'), { kind: 'issues' });
  assert.deepEqual(decodeLocation('?state=servers'), { kind: 'servers' });
});

test('path-specific first-run review states keep the path the mock defines', () => {
  assert.deepEqual(decodeLocation('?state=waiting&path=own'), {
    kind: 'first-run',
    step: 'waiting',
    path: 'invited',
  });
  assert.deepEqual(decodeLocation('?state=create-group&path=invited'), {
    kind: 'first-run',
    step: 'create-group',
    path: 'own',
  });
});

test('a state that is not a location decodes to nothing, not to a guess', () => {
  // These are sheets over a location in the mock, not places of their own.
  for (const state of ['new', 'manage', 'conflict', 'exists', 'show', 'grid']) {
    assert.equal(decodeLocation(`?state=${state}`), null, state);
  }
  assert.equal(decodeLocation(''), null);
  assert.equal(decodeLocation('?other=1'), null);
  // A store state with no store names nothing.
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
  assert.equal(encodeLocation({ kind: 'first-run', step: 'protect' }).params.step, 'protect');
});

/* ----------------------------------------------------------------- scenes -- */

test('the mock s state names carry what is not a location', () => {
  // `grid` is a view preference, `lease` is a world and `show` is a
  // selection. `decodeLocation` refuses all three; the scene says what they
  // mean.
  assert.deepEqual(decodeScene('?state=grid'), {
    ...INITIAL_SCENE,
    view: 'grid',
  });
  assert.deepEqual(decodeScene('?state=lease'), {
    ...INITIAL_SCENE,
    location: { kind: 'store', ref: 'acct:work' },
    lease: 'lapsed',
  });
  assert.deepEqual(decodeScene('?state=inactive'), {
    ...INITIAL_SCENE,
    location: { kind: 'store', ref: 'team:homelab' },
  });
  assert.deepEqual(decodeScene('?state=issues'), {
    ...INITIAL_SCENE,
    location: { kind: 'issues' },
    lease: 'lapsed',
  });
  assert.deepEqual(decodeScene('?state=show'), {
    ...INITIAL_SCENE,
    demo: 'password',
    reveal: true,
  });
});

test('nothing a scene holds is dropped by a reload', () => {
  const scenes = [
    INITIAL_SCENE,
    { ...INITIAL_SCENE, view: 'grid' as const },
    {
      ...INITIAL_SCENE,
      location: { kind: 'store' as const, ref: 'team:eng' },
      selection: { store: 'team:eng', path: '/deploy/production-token' },
      kind: 'Password' as const,
      sort: 'version' as const,
      view: 'grid' as const,
      lease: 'lapsed' as const,
    },
    { ...INITIAL_SCENE, location: { kind: 'settings' as const, section: 'agent' as const } },
  ];
  for (const scene of scenes) {
    const href = sceneHref('http://localhost/', scene);
    assert.deepEqual(decodeScene(new URL(href).search), scene, href);
  }
});

test('a scene at its defaults writes nothing but its state name', () => {
  assert.equal(sceneHref('http://localhost/', INITIAL_SCENE), 'http://localhost/?state=all');
  // An explicit parameter beats the alias it disagrees with.
  assert.equal(decodeScene('?state=grid&view=list').view, 'list');
  // Nonsense is ignored rather than guessed at.
  assert.equal(decodeScene('?state=all&sort=sideways').sort, INITIAL_SCENE.sort);
  assert.equal(decodeScene('?state=all&sel=nostore').selection, null);
});

/* ------------------------------------------------------------------ store -- */

test('the store publishes only when the state actually moved', () => {
  const store = new LocationStore();
  let notifications = 0;
  const unsubscribe = store.subscribe(() => {
    notifications += 1;
  });

  assert.deepEqual(store.getSnapshot(), INITIAL_STATE);
  store.navigate({ kind: 'issues' });
  assert.equal(notifications, 1);
  assert.deepEqual(store.getSnapshot().location, { kind: 'issues' });

  // The same place again: `transition` returns the state unchanged, so no
  // listener runs and React does not re-render.
  store.navigate({ kind: 'issues' });
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
  store.navigate({ kind: 'servers' });
  assert.equal(store.getSnapshot(), store.getSnapshot());
});
