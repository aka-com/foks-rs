/**
 * The ⌘K palette's index and its ranking, as pure functions over the fixture.
 *
 * The index is what the palette can find at all; `searchResults` is the order
 * it finds it in, the scope chips, the `in:` and `kind:` tokens, the per-group
 * cap, and the Recent, Favorites and Actions lists an empty query shows. The
 * module is loaded
 * through Vite because the palette it lives in imports its own stylesheet and
 * the kit by absolute alias; neither resolves under a bare node loader.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer, type ViteDevServer } from 'vite';

import { FIXTURE } from '../src/fixture';
import type {
  SearchAction,
  SearchChannel,
  SearchEntry,
} from '../src/shell/search-palette';

type Palette = typeof import('../src/shell/search-palette');

let palette: Palette;
let vite: ViteDevServer;
let INDEX: readonly SearchEntry[];

const CHANNELS: readonly SearchChannel[] = [
  { store: 'team:eng', name: 'deploys', id: 'a'.repeat(32) },
  { store: 'team:eng', name: 'general', id: 'b'.repeat(32) },
  { store: 'team:household', name: 'general' },
  // A channel on a store this Mac does not hold is not indexed.
  { store: 'team:missing', name: 'ghosts' },
];

test.before(async () => {
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  palette = (await vite.ssrLoadModule(
    '/src/shell/search-palette.tsx',
  )) as Palette;
  INDEX = palette.searchIndex(FIXTURE, CHANNELS);
});
test.after(async () => vite.close());

function names(entries: readonly SearchEntry[]): string[] {
  return entries.map((entry) => entry.name);
}

function scopes(entries: readonly SearchEntry[]): string[] {
  return [...new Set(entries.map((entry) => entry.scope))];
}

test('the index holds every kind the palette searches', () => {
  assert.deepEqual(scopes(INDEX).sort(), [
    'channels',
    'items',
    'people',
    'stores',
  ]);

  // Items: every fixture node, folders included, named by its last segment.
  const items = INDEX.filter((entry) => entry.scope === 'items');
  assert.equal(items.length, FIXTURE.items.length);
  const ssh = items.find((entry) => entry.name === 'ssh');
  assert.ok(ssh, 'a folder is a result');
  assert.equal(ssh.detail, 'Folder · Personal');
  assert.deepEqual(ssh.glyph, { kind: 'folder' });
  // A folder opens in the tree rather than in the details panel.
  assert.deepEqual(ssh.target, {
    kind: 'folder',
    store: 'acct:personal',
    path: '/ssh',
  });
  assert.equal(ssh.copies, undefined, 'a folder has nothing to copy');
  const database = items.find((entry) => entry.name === 'DATABASE_URL');
  assert.ok(database);
  // The kind rule reads the item, so a connection string is a Note.
  assert.equal(database.detail, 'Document · env/prod · Personal');
  assert.equal(database.where, 'Personal server');
  assert.deepEqual(database.target, {
    kind: 'item',
    store: 'acct:personal',
    path: '/env/prod/DATABASE_URL',
  });
  // A path is searchable beside the name, so a folder reaches what is in it.
  assert.deepEqual(database.terms, ['DATABASE_URL', '/env/prod/DATABASE_URL']);
  // Recent and Favorites keep the store and the dirent id, never the path.
  assert.deepEqual(database.ref, {
    store: 'acct:personal',
    dirent: '00000000000000000000000000000003',
  });
  // ⌘C copies a text Document's value, a login's password first and its user
  // name on ⇧⌘C, and nothing from a file.
  assert.deepEqual(
    database.copies?.map((copy) => copy.field),
    ['value'],
  );
  assert.deepEqual(
    items
      .find((entry) => entry.name === 'github.com')
      ?.copies?.map((copy) => copy.field),
    ['password', 'username'],
  );
  assert.equal(
    items.find((entry) => entry.name === 'id_ed25519')?.copies,
    undefined,
  );
  // ⌘O opens a Password's website; a Document, a file and a folder have none.
  assert.equal(
    items.find((entry) => entry.name === 'github.com')?.website,
    true,
  );
  assert.equal(database.website, undefined);
  assert.equal(
    items.find((entry) => entry.name === 'id_ed25519')?.website,
    undefined,
  );
  assert.equal(ssh.website, undefined);

  // Stores: the two accounts, then the named groups, then the ad-hoc share.
  assert.deepEqual(names(INDEX.filter((entry) => entry.scope === 'stores')), [
    'Personal',
    'Work (Acme)',
    'Engineering',
    'Household',
    'Homelab',
  ]);
  const homelab = INDEX.find((entry) => entry.name === 'Homelab');
  assert.ok(homelab);
  assert.equal(homelab.detail, 'Share · Setup incomplete');
  assert.deepEqual(homelab.target, { kind: 'store', ref: 'team:homelab' });
  const engineering = INDEX.find((entry) => entry.name === 'Engineering');
  assert.ok(engineering);
  assert.equal(engineering.detail, 'Team · 4 people · 1 machine · 1 team');
  // A store answers to its alias as well as its name.
  assert.deepEqual(engineering.terms, ['Engineering', 'engineering']);

  // People: one row per person per server, not one per membership.
  const people = INDEX.filter((entry) => entry.scope === 'people');
  assert.deepEqual(names(people), [
    'sam.ortiz',
    'vitalik',
    'priya.n',
    'dana.okafor',
    'deploy-bot',
    'satoshi',
    'sam',
  ]);
  const bot = people.find((entry) => entry.name === 'deploy-bot');
  assert.ok(bot);
  assert.equal(bot.detail, 'Engineering · Machine');
  assert.deepEqual(bot.target, {
    kind: 'person',
    ref: 'team:eng',
    username: 'deploy-bot',
  });
  assert.equal(
    people.find((entry) => entry.name === 'priya.n')?.detail,
    'Engineering · Admin',
  );

  // Channels: named by group and channel, and only where the store is held.
  const channels = INDEX.filter((entry) => entry.scope === 'channels');
  assert.deepEqual(names(channels), [
    'engineering#deploys',
    'engineering#general',
    'household#general',
  ]);
  assert.deepEqual(channels[0].target, {
    kind: 'channel',
    ref: 'team:eng',
    channel: 'a'.repeat(32),
  });
  // Without an id the result still opens the team's inbox.
  assert.deepEqual(channels[2].target, {
    kind: 'channel',
    ref: 'team:household',
  });
});

test('a person on two rosters of one server is one result naming both', () => {
  const snapshot = {
    ...FIXTURE,
    parties: [
      ...FIXTURE.parties,
      {
        store: 'team:homelab',
        username: 'sam',
        party_kind: 'user' as const,
        generation: 1,
        locally_manageable: true,
        party_id_hex: '01'.padEnd(66, 'f'),
        source_role: { role: 'Member', visibility: 0 },
        destination_role: { role: 'Member', visibility: 0 },
      },
    ],
  };
  const sam = palette
    .searchIndex(snapshot)
    .filter((entry) => entry.name === 'sam');
  assert.equal(sam.length, 1);
  assert.equal(sam[0].detail, 'Household, Homelab · Member');
  // The first roster is where the result opens.
  assert.deepEqual(sam[0].target, {
    kind: 'person',
    ref: 'team:household',
    username: 'sam',
  });
});

test('the palette works with no channels at all', () => {
  const bare = palette.searchIndex(FIXTURE);
  assert.deepEqual(scopes(bare).sort(), ['items', 'people', 'stores']);
  assert.deepEqual(palette.searchResults(bare, 'general', 'channels'), []);
});

test('an empty query lists the head of every group', () => {
  const results = palette.searchResults(INDEX, '');
  // Groups come out in one order, whatever order the index was built in.
  assert.deepEqual(scopes(results), ['items', 'stores', 'people', 'channels']);
  for (const scope of scopes(results))
    assert.ok(
      results.filter((entry) => entry.scope === scope).length <=
        palette.SEARCH_GROUP_LIMIT,
      `${scope} is capped`,
    );
  // The fixture's items and people both run past the cap.
  assert.equal(
    results.filter((entry) => entry.scope === 'items').length,
    palette.SEARCH_GROUP_LIMIT,
  );
  assert.equal(
    results.filter((entry) => entry.scope === 'people').length,
    palette.SEARCH_GROUP_LIMIT,
  );
});

test('a prefix beats a word start, which beats a substring', () => {
  assert.deepEqual(names(palette.searchResults(INDEX, 'pass', 'items')), [
    // A prefix of the name.
    'passport-scan.pdf',
    // A word start inside the name, after the hyphen.
    'guest-password',
  ]);
  // `sam` prefixes one username and sits inside another's group line, which is
  // not searched: only the username is.
  assert.deepEqual(names(palette.searchResults(INDEX, 'sam', 'people')), [
    'sam.ortiz',
    'sam',
  ]);
  // A name match outranks the same kind of match reached through a path.
  assert.deepEqual(names(palette.searchResults(INDEX, 'token', 'items')), [
    'production-token',
    'staging-token',
  ]);
  // A path-only match still lands, behind every name match.
  const deploy = names(palette.searchResults(INDEX, 'deploy', 'items'));
  assert.deepEqual(deploy, ['production-token', 'staging-token']);
});

test('a store matches its alias and a channel its hash', () => {
  assert.deepEqual(names(palette.searchResults(INDEX, 'acme', 'stores')), [
    'Work (Acme)',
  ]);
  assert.deepEqual(
    names(palette.searchResults(INDEX, 'engineering', 'stores')),
    ['Engineering'],
  );
  assert.deepEqual(
    names(palette.searchResults(INDEX, '#deploys', 'channels')),
    ['engineering#deploys'],
  );
  // The group name reaches every channel in it.
  assert.deepEqual(
    names(palette.searchResults(INDEX, 'household', 'channels')),
    ['household#general'],
  );
});

test('a scope chip admits only its own group', () => {
  for (const chip of palette.SEARCH_SCOPES) {
    if (chip.id === 'all') continue;
    const results = palette.searchResults(INDEX, '', chip.id);
    assert.ok(results.length, `${chip.id} has results`);
    assert.deepEqual(scopes(results), [chip.id]);
  }
  // `All` reaches across the groups a single chip would cut.
  assert.ok(scopes(palette.searchResults(INDEX, 'e')).length > 1);
});

test('a query that matches nothing returns nothing', () => {
  assert.deepEqual(palette.searchResults(INDEX, 'zzzznothing'), []);
  // A query is trimmed, not taken literally.
  assert.deepEqual(
    names(palette.searchResults(INDEX, '  netflix  ', 'items')),
    ['netflix'],
  );
});

test('in: keeps the results inside a store, and kind: keeps one kind', () => {
  assert.deepEqual(palette.parseSearchQuery('in:eng  token kind:Pass'), {
    needle: 'token',
    within: ['eng'],
    kinds: ['pass'],
  });
  // A quoted value carries its spaces; a token still being typed filters
  // nothing yet.
  assert.deepEqual(palette.parseSearchQuery('in:"work (acme)" kind:'), {
    needle: '',
    within: ['work (acme)'],
    kinds: [],
  });

  // `token` matches in both teams' paths; `in:` keeps Engineering's.
  assert.deepEqual(names(palette.searchResults(INDEX, 'in:eng token')), [
    'production-token',
    'staging-token',
  ]);
  // A token alone is a filter: the head of each group it admits, across
  // items, people on the roster and the store's channels, but not the store.
  const engineering = palette.searchResults(INDEX, 'in:engineering');
  assert.deepEqual(scopes(engineering), ['items', 'people', 'channels']);
  assert.ok(
    engineering
      .filter((entry) => entry.scope === 'items')
      .every((entry) => entry.detail.endsWith('Engineering')),
  );
  // The store's alias is matched as well as its name.
  assert.deepEqual(
    names(palette.searchResults(INDEX, 'in:acme', 'items')),
    names(palette.searchResults(INDEX, 'in:work', 'items')),
  );

  // `kind:` admits items only, by the kind a person reads them as.
  assert.deepEqual(names(palette.searchResults(INDEX, 'kind:password')), [
    'github.com',
    'fastmail.com',
    'guest-password',
    'netflix',
  ]);
  assert.deepEqual(names(palette.searchResults(INDEX, 'kind:folder')), ['ssh']);
  assert.deepEqual(
    names(palette.searchResults(INDEX, 'in:household kind:doc')),
    ['emergency.pdf'],
  );
  // Two values of one key widen each other.
  assert.equal(
    palette.searchResults(INDEX, 'in:household in:engineering kind:pass')
      .length,
    2,
  );
  assert.deepEqual(palette.searchResults(INDEX, 'kind:nothing'), []);
});

const ACTIONS: readonly SearchAction[] = [
  {
    id: 'lock',
    label: 'Lock',
    detail: 'Lock the app',
    icon: 'lock',
    run: () => undefined,
  },
  {
    id: 'new-password',
    label: 'New password',
    detail: 'In Personal',
    icon: 'key',
    keys: '⌘N',
    run: () => undefined,
  },
];

test('actions are searched under All and have no chip of their own', () => {
  const index = palette.searchIndex(FIXTURE, CHANNELS, ACTIONS);
  const lock = palette.searchResults(index, 'lock');
  assert.deepEqual(names(lock), ['Lock']);
  assert.equal(lock[0].group, 'Actions');
  assert.deepEqual(lock[0].target, { kind: 'action', id: 'lock' });
  assert.equal(
    palette.SEARCH_SCOPES.some((chip) => chip.id === 'actions'),
    false,
  );
  assert.deepEqual(palette.searchResults(index, 'lock', 'items'), []);
  // An action is in no store and has no kind, so a token excludes it.
  assert.deepEqual(palette.searchResults(index, 'in:personal lock'), []);
});

test('an empty query lists every favorite, past the ten Recent keeps', () => {
  const index = palette.searchIndex(FIXTURE, CHANNELS, ACTIONS);
  const favorites = index.flatMap((entry) => (entry.ref ? [entry.ref] : []));
  assert.ok(favorites.length > 10, 'the fixture has more than ten items');
  const results = palette.searchResults(
    index,
    '',
    'all',
    palette.SEARCH_GROUP_LIMIT,
    { recent: [], favorites },
  );
  assert.equal(
    results.filter((entry) => entry.group === 'Favorites').length,
    favorites.length,
  );
});

test('an empty query lists Recent, Favorites, then Actions', () => {
  const index = palette.searchIndex(FIXTURE, CHANNELS, ACTIONS);
  const netflix = { store: 'team:household', dirent: '0'.repeat(31) + 'e' };
  const ssh = { store: 'acct:personal', dirent: '0'.repeat(31) + '5' };
  const gone = { store: 'acct:personal', dirent: 'f'.repeat(32) };
  const results = palette.searchResults(
    index,
    '  ',
    'all',
    palette.SEARCH_GROUP_LIMIT,
    { recent: [gone, netflix, ssh], favorites: [netflix] },
  );
  // An id the snapshot no longer holds is left out, not drawn by an old name.
  assert.deepEqual(
    results.map((entry) => [entry.group, entry.name]),
    [
      ['Recent', 'netflix'],
      ['Recent', 'ssh'],
      ['Favorites', 'netflix'],
      ['Actions', 'Lock'],
      ['Actions', 'New password'],
    ],
  );
  // One item in two lists is two rows with their own ids.
  assert.equal(new Set(results.map((entry) => entry.id)).size, results.length);
  // Recent and Favorites are the start of All only: a chip, a query or a token
  // ranks the index as before.
  assert.equal(
    palette
      .searchResults(index, '', 'items', palette.SEARCH_GROUP_LIMIT, {
        recent: [netflix],
        favorites: [],
      })
      .some((entry) => entry.group === 'Recent'),
    false,
  );
  assert.deepEqual(
    names(
      palette.searchResults(index, 'kind:folder', 'all', 6, {
        recent: [netflix],
        favorites: [],
      }),
    ),
    ['ssh'],
  );
});
