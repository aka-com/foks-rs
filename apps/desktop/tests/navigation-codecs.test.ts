import assert from 'node:assert/strict';
import test from 'node:test';

import {
  decodeAcceptanceScene,
  decodeForRuntimeScene,
  decodeLocation,
  decodeProductionLocation,
  decodeProductionScene,
  decodeScene,
  INITIAL_SCENE,
  locationHref,
  sceneHref,
} from '../src/location';
import type { Location } from '../src/location';
import { decodeProductionLocation as productionLocation } from '../src/navigation/production-codec';
import { decodeProductionScene as productionScene } from '../src/navigation/scene-codec';

const ACCOUNT = JSON.stringify({
  kind: 'account',
  profile: 'server a&b',
  accountAlias: 'owner',
});
const TEAM = JSON.stringify({
  kind: 'team',
  profile: 'server a&b',
  accountAlias: 'owner',
  teamAlias: 'project',
});
const CHANNEL = 'ab'.repeat(16);

const FIXTURE_LOCATION_NAMES = [
  'personal',
  'work',
  'household',
  'group',
  'homelab',
  'group-people',
  'group-channels',
  'group-files',
  'party',
  'federation',
  'items',
  'danger',
  'rekey-menu',
  'invite',
  'add',
  'demote',
  'remove',
  'add-team',
  'servers-server',
  'servers-lapsed',
  'servers-rollback',
  'servers-reset',
  'servers-unprobed',
  'servers-check',
  'settings-macs-work',
];
const FIRST_RUN_NAMES = [
  'boot',
  'who',
  'local',
  'address',
  'no-address',
  'checked',
  'compare',
  'error',
  'account',
  'existing',
  'protect',
  'phrase',
  'waiting',
  'added',
  'local-done',
  'identity-pending',
  'operation-pending',
  'checklist-invited',
  'checklist-own',
];
const FIXTURE_SCENE_NAMES = [
  'password',
  'show',
  'resource',
  'file',
  'link',
  'conflict',
  'store',
  'manage',
  'party-remove',
  'groups-lease',
  'groups-inactive',
  'group-new-document',
  'grid',
  'folders',
  'lease',
  'inactive',
];

const PUBLIC_ALIASES: Record<string, Location> = {
  all: { kind: 'all' },
  alerts: { kind: 'settings', section: 'account' },
  // Both names promise a sheet over the Teams list, and open it.
  join: { kind: 'teams', open: 'join' },
  groups: { kind: 'teams' },
  create: { kind: 'teams', open: 'create' },
  servers: { kind: 'settings', section: 'account' },
  settings: { kind: 'settings' },
  'servers-list': { kind: 'settings', section: 'account' },
  'servers-add': { kind: 'settings', section: 'account' },
  'settings-macs': { kind: 'devices', section: 'macs' },
  'settings-phrase': { kind: 'devices', section: 'macs' },
  'settings-keys': { kind: 'devices', section: 'keys' },
  'settings-enrol': { kind: 'devices', section: 'keys' },
  'settings-account': { kind: 'settings', section: 'account' },
  people: { kind: 'settings', section: 'account' },
  'settings-agent': { kind: 'settings', section: 'mac' },
  'settings-about': { kind: 'settings', section: 'mac' },
};

function search(state: string, params: Record<string, string> = {}): string {
  return `?${new URLSearchParams({ state, ...params })}`;
}

test('the facade exposes the leaf codecs and keeps decodeScene acceptance-compatible', () => {
  assert.equal(decodeProductionLocation, productionLocation);
  assert.equal(decodeProductionScene, productionScene);
  assert.equal(decodeScene, decodeAcceptanceScene);
  for (const name of [
    ...FIXTURE_LOCATION_NAMES,
    ...FIRST_RUN_NAMES,
    ...FIXTURE_SCENE_NAMES,
    ...Object.keys(PUBLIC_ALIASES),
  ]) {
    const query = search(name);
    assert.deepEqual(
      decodeForRuntimeScene(query, { fixtures: true }),
      decodeScene(query),
      name,
    );
    assert.deepEqual(
      decodeForRuntimeScene(query, { fixtures: false }),
      decodeProductionScene(query),
      name,
    );
  }
});

test('production never synthesizes fixture addresses or accepts bare first-run scenes', () => {
  for (const name of [
    ...FIXTURE_LOCATION_NAMES,
    ...FIRST_RUN_NAMES,
    ...FIXTURE_SCENE_NAMES,
  ]) {
    assert.equal(decodeProductionLocation(search(name)), null, name);
    assert.deepEqual(decodeProductionScene(search(name)), INITIAL_SCENE, name);
  }
  for (const name of [...FIXTURE_LOCATION_NAMES, ...FIRST_RUN_NAMES]) {
    assert.notEqual(decodeLocation(search(name)), null, name);
    assert.notDeepEqual(
      decodeForRuntimeScene(search(name), { fixtures: true }).location,
      INITIAL_SCENE.location,
      name,
    );
  }
  assert.deepEqual(decodeAcceptanceScene('?state=store').location, {
    kind: 'store',
    ref: 'team:eng',
  });
  assert.deepEqual(decodeAcceptanceScene('?state=manage').location, {
    kind: 'group-settings',
    ref: 'team:household',
    tab: 'people',
  });
});

test('production ignores reveal, demo, lease and URL attempts to enable fixtures', () => {
  for (const name of [
    'show',
    'lease',
    'groups-lease',
    'alerts',
    'password',
    'all',
  ]) {
    const query = search(name, {
      lease: 'lapsed',
      reveal: 'true',
      demo: 'password',
      fixtures: 'true',
      fixture: 'true',
      acceptance: 'true',
    });
    const scene = decodeForRuntimeScene(query, { fixtures: false });
    assert.equal(scene.reveal, false, name);
    assert.equal(scene.demo, null, name);
    assert.equal(scene.lease, 'fresh', name);
  }
  assert.equal(decodeAcceptanceScene('?state=show').reveal, true);
  assert.equal(decodeAcceptanceScene('?state=show').demo, 'password');
  assert.equal(
    decodeAcceptanceScene('?state=all&lease=lapsed').lease,
    'lapsed',
  );
  assert.equal(decodeAcceptanceScene('?state=alerts').lease, 'lapsed');
  assert.equal(decodeProductionScene('?state=alerts').lease, 'fresh');
});

test('public legacy destinations are available without fixture capabilities', () => {
  for (const [name, location] of Object.entries(PUBLIC_ALIASES)) {
    const query = search(name);
    assert.deepEqual(decodeProductionLocation(query), location, name);
    assert.deepEqual(decodeLocation(query), location, name);
    assert.deepEqual(
      decodeProductionScene(query),
      {
        ...INITIAL_SCENE,
        location,
      },
      name,
    );
  }
  assert.deepEqual(
    decodeProductionLocation(
      search('settings-enrol', {
        store: ACCOUNT,
        device: 'yubi:primary key',
      }),
    ),
    {
      kind: 'devices',
      section: 'keys',
      store: ACCOUNT,
      device: 'yubi:primary key',
    },
  );
  assert.deepEqual(
    decodeProductionLocation(
      search('servers-list', {
        store: ACCOUNT,
        profile: 'remote',
      }),
    ),
    {
      kind: 'settings',
      section: 'account',
      store: ACCOUNT,
      profile: 'remote',
    },
  );
  assert.deepEqual(
    decodeProductionLocation(search('alerts', { profile: 'ignored' })),
    { kind: 'settings', section: 'account' },
  );
});

test('canonical production routes round-trip opaque store references and explicit first-run state', () => {
  const locations: Location[] = [
    { kind: 'all' },
    { kind: 'files' },
    { kind: 'store', ref: TEAM },
    { kind: 'group-settings', ref: TEAM, tab: 'channels' },
    { kind: 'group-settings', ref: TEAM, tab: 'requests' },
    { kind: 'settings', section: 'account', store: ACCOUNT },
    { kind: 'teams', store: ACCOUNT },
    { kind: 'teams', store: ACCOUNT, open: 'create' },
    { kind: 'teams', open: 'join' },
    { kind: 'chat' },
    { kind: 'chat', ref: TEAM, channel: CHANNEL },
    {
      kind: 'devices',
      store: ACCOUNT,
      section: 'keys',
      device: 'yubi:primary key',
    },
    { kind: 'settings', store: ACCOUNT, section: 'account', profile: 'remote' },
    { kind: 'settings', store: ACCOUNT, profile: 'remote' },
    { kind: 'settings', store: ACCOUNT, section: 'preferences' },
    { kind: 'first-run', step: 'waiting', path: 'invited' },
    { kind: 'first-run', step: 'local', path: 'own' },
  ];
  for (const location of locations) {
    const query = new URL(locationHref('https://desktop.example/', location))
      .search;
    assert.deepEqual(decodeProductionLocation(query), location);
    assert.deepEqual(decodeLocation(query), location);
  }
  // A sheet name the page does not own is not an address of its own: the
  // list opens with no sheet rather than the address being refused.
  assert.deepEqual(
    decodeProductionLocation(search('teams', { open: 'nope' })),
    {
      kind: 'teams',
    },
  );
  // And the parameter does not leak onto the next page.
  assert.equal(
    new URL(
      locationHref('https://desktop.example/?state=teams&open=create', {
        kind: 'files',
      }),
    ).searchParams.get('open'),
    null,
  );
  assert.deepEqual(
    decodeProductionLocation(
      search('team-chat', {
        store: TEAM,
        channel: CHANNEL,
      }),
    ),
    { kind: 'chat', ref: TEAM, channel: CHANNEL },
  );
  assert.equal(decodeProductionLocation('?state=team-chat'), null);
  assert.deepEqual(
    decodeProductionLocation(
      search('chat', {
        store: TEAM,
        channel: 'invalid',
      }),
    ),
    { kind: 'chat', ref: TEAM },
  );
  assert.deepEqual(
    decodeProductionLocation(
      search('team-chat', { store: TEAM, channel: 'invalid' }),
    ),
    { kind: 'chat', ref: TEAM },
  );
  assert.deepEqual(
    decodeProductionLocation(
      search('chat', {
        channel: CHANNEL,
      }),
    ),
    { kind: 'chat' },
  );
});

test('folded and retired settings sections remain production deep links', () => {
  const sections: Record<string, Location> = {
    credentials: { kind: 'settings', section: 'account' },
    notifications: { kind: 'settings', section: 'preferences' },
    device: { kind: 'settings', section: 'mac' },
    about: { kind: 'settings', section: 'mac' },
    agent: { kind: 'settings', section: 'mac' },
    'security-keys': { kind: 'settings', section: 'account' },
    macs: { kind: 'devices', section: 'macs' },
    phrase: { kind: 'devices', section: 'macs' },
    keys: { kind: 'devices', section: 'keys' },
    groups: { kind: 'teams' },
    account: { kind: 'settings', section: 'account' },
  };
  for (const [section, location] of Object.entries(sections)) {
    const query = search('settings', {
      section,
      store: ACCOUNT,
      profile: 'ignored',
      account: 'ignored',
    });
    assert.deepEqual(
      decodeProductionLocation(query),
      {
        ...location,
        store: ACCOUNT,
        ...(section === 'account' ? { profile: 'ignored' } : {}),
      },
      section,
    );
    assert.deepEqual(decodeProductionLocation(query), decodeLocation(query));
  }
});

test('production preserves explicit selection, folders, filters and sorting without scene effects', () => {
  const query = search('show', {
    sel: `${TEAM}|/documents/key`,
    folder: '/documents',
    closed: '/a,/b',
    view: 'folders',
    kind: 'Resource',
    sort: 'group',
    lease: 'lapsed',
    reveal: 'true',
    demo: 'password',
  });
  const scene = decodeProductionScene(query);
  assert.deepEqual(scene, {
    ...INITIAL_SCENE,
    selection: { store: TEAM, path: '/documents/key' },
    folder: '/documents',
    closedFolders: ['/a', '/b'],
    kind: 'Document',
    sort: 'group',
  });
  assert.deepEqual(
    decodeProductionScene(
      new URL(sceneHref('https://desktop.example/', scene)).search,
    ),
    scene,
  );
  const acceptance = decodeAcceptanceScene(query);
  assert.deepEqual(acceptance.selection, scene.selection);
  assert.equal(acceptance.reveal, true);
  assert.equal(acceptance.lease, 'lapsed');
  assert.deepEqual(
    decodeProductionScene('?state=all&sel=bad&kind=bad&sort=bad'),
    INITIAL_SCENE,
  );
});
