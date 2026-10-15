/**
 * Recent and Favorites as stored: the cap on Favorites, and a move recorded
 * by `noteMoved` settled by `followMoves` once the destination's new dirent
 * id is in a snapshot.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { installDom } from './lib/dom-harness';

installDom({ url: 'http://localhost/' });

const lists = await import('../src/recent-favorites');

const STORE = 'acct:personal';

function ref(dirent: string) {
  return { store: STORE, dirent };
}

test.afterEach(() => window.localStorage.clear());

test('adding a favorite past the cap drops the oldest', () => {
  for (let index = 0; index <= lists.FAVORITE_LIMIT; index += 1)
    lists.toggleFavorite(ref(`d${index}`));
  const { favorites } = lists.recentFavorites();
  assert.equal(favorites.length, lists.FAVORITE_LIMIT);
  assert.deepEqual(favorites[0], ref(`d${lists.FAVORITE_LIMIT}`));
  assert.equal(
    favorites.some((seen) => lists.sameItemRef(seen, ref('d0'))),
    false,
  );
});

test('a move takes the new dirent id once a snapshot holds the destination', () => {
  lists.rememberRecent(ref('c'));
  lists.rememberRecent(ref('b'));
  lists.rememberRecent(ref('a'));
  lists.toggleFavorite(ref('b'));
  lists.noteMoved({ store: STORE, direntId: 'b' }, '/new/b');
  // A move of an item in neither list is not recorded.
  lists.noteMoved({ store: STORE, direntId: 'z' }, '/new/z');

  // The snapshot read before the move settles nothing.
  lists.followMoves([{ store: STORE, path: '/old/b', direntId: 'b' }]);
  assert.deepEqual(lists.recentFavorites().favorites, [ref('b')]);

  lists.followMoves([
    { store: STORE, path: '/new/b', direntId: 'b2' },
    { store: STORE, path: '/new/z', direntId: 'z2' },
  ]);
  // The entry keeps its place in each list.
  assert.deepEqual(lists.recentFavorites(), {
    recent: [ref('a'), ref('b2'), ref('c')],
    favorites: [ref('b2')],
  });

  // A settled move is not applied again to a later item at that path.
  lists.followMoves([{ store: STORE, path: '/new/b', direntId: 'b3' }]);
  assert.deepEqual(lists.recentFavorites().favorites, [ref('b2')]);
});

test('a move onto an id already listed leaves one entry', () => {
  lists.rememberRecent(ref('b'));
  lists.rememberRecent(ref('a'));
  lists.noteMoved({ store: STORE, direntId: 'b' }, '/a');
  lists.followMoves([{ store: STORE, path: '/a', direntId: 'a' }]);
  assert.deepEqual(lists.recentFavorites().recent, [ref('a')]);
});
