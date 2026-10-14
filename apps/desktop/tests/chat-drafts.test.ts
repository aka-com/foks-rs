import assert from 'node:assert/strict';
import test from 'node:test';
import { DraftStore } from '../src/chat/send/drafts';

/** Counts the calls each subscription receives. */
function counter(store: DraftStore, team: string, channel: string) {
  const counts = { calls: 0 };
  const stop = store.subscribe(team, channel, () => counts.calls++);
  return Object.assign(counts, { stop });
}

test('a draft change notifies only the listeners of its channel', () => {
  const store = new DraftStore(1024);
  const general = counter(store, 'team:a', 'general');
  const random = counter(store, 'team:a', 'random');
  const elsewhere = counter(store, 'team:b', 'general');
  assert.equal(store.set('team:a', 'general', 'h'), true);
  assert.equal(store.set('team:a', 'general', 'hi'), true);
  assert.equal(store.get('team:a', 'general'), 'hi');
  assert.equal(general.calls, 2);
  assert.equal(random.calls, 0);
  assert.equal(elsewhere.calls, 0);
  // Storing the same text changes nothing, so it notifies no one.
  store.set('team:a', 'general', 'hi');
  assert.equal(general.calls, 2);
  // An empty text removes the draft.
  store.set('team:a', 'general', '');
  assert.equal(store.get('team:a', 'general'), '');
  assert.equal(general.calls, 3);
  // Removing a draft that does not exist notifies no one.
  assert.equal(store.delete('team:a', 'general'), false);
  assert.equal(general.calls, 3);
  general.stop();
  store.set('team:a', 'general', 'after');
  assert.equal(general.calls, 3);
  random.stop();
  elsewhere.stop();
});

test('the byte total follows each change without measuring other drafts', () => {
  const store = new DraftStore(16);
  store.set('team:a', 'one', 'abcd');
  // Two bytes per character: the quota counts UTF-8, not characters.
  store.set('team:b', 'two', 'ééé');
  assert.equal(store.bytes(), 10);
  assert.equal(store.size('team:b', 'two'), 6);
  assert.equal(store.size('team:b', 'none'), 0);
  store.set('team:a', 'one', 'ab');
  assert.equal(store.bytes(), 8);
  // A draft that would take the total past the quota is refused, and the
  // channel keeps its previous text.
  assert.equal(store.set('team:a', 'one', 'abcdefghijk'), false);
  assert.equal(store.get('team:a', 'one'), 'ab');
  assert.equal(store.bytes(), 8);
  // The draft being replaced does not count against its replacement: the
  // other team's six bytes and these ten fill the quota exactly.
  assert.equal(store.set('team:a', 'one', 'abcdefghij'), true);
  assert.equal(store.bytes(), 16);
  store.delete('team:b', 'two');
  assert.equal(store.bytes(), 10);
  store.set('team:a', 'one', '');
  assert.equal(store.bytes(), 0);
});

test('clearing a team or keeping its live channels notifies the drafts it removes', () => {
  const store = new DraftStore(1024);
  store.set('team:a', 'live', 'kept');
  store.set('team:a', 'gone', 'dropped');
  store.set('team:b', 'other', 'untouched');
  const live = counter(store, 'team:a', 'live');
  const gone = counter(store, 'team:a', 'gone');
  const other = counter(store, 'team:b', 'other');
  store.retain('team:a', new Set(['live']));
  assert.equal(store.get('team:a', 'live'), 'kept');
  assert.equal(store.get('team:a', 'gone'), '');
  assert.deepEqual([live.calls, gone.calls, other.calls], [0, 1, 0]);
  assert.equal(store.clear('team:a'), true);
  assert.equal(store.clear('team:a'), false);
  assert.equal(store.get('team:a', 'live'), '');
  assert.deepEqual([live.calls, gone.calls, other.calls], [1, 1, 0]);
  store.clearAll();
  assert.equal(store.get('team:b', 'other'), '');
  assert.equal(other.calls, 1);
  assert.equal(store.bytes(), 0);
});

test('a stale unsubscribe does not remove a later subscription', () => {
  const store = new DraftStore(1024);
  const first = counter(store, 'team:a', 'general');
  first.stop();
  const second = counter(store, 'team:a', 'general');
  // Unsubscribing twice is allowed and must not reach the new listener.
  first.stop();
  store.set('team:a', 'general', 'text');
  assert.equal(second.calls, 1);
  second.stop();
});

test('the drafted channels change only when a draft appears or goes', () => {
  const store = new DraftStore(1024);
  let calls = 0;
  const stop = store.subscribeDrafted(() => calls++);
  const empty = store.drafted();
  assert.equal(empty.size, 0);
  store.set('team:a', 'general', 'h');
  const first = store.drafted();
  assert.notEqual(first, empty);
  assert.deepEqual([...(first.get('team:a') ?? [])], ['general']);
  // Editing a draft that stays non-empty keeps the snapshot.
  store.set('team:a', 'general', 'hi');
  store.set('team:a', 'general', 'hi there');
  assert.equal(store.drafted(), first);
  assert.equal(calls, 1);
  store.set('team:a', 'random', 'x');
  store.set('team:b', 'general', 'y');
  assert.deepEqual(
    [...store.drafted()].map(([team, channels]) => [team, [...channels]]),
    [
      ['team:a', ['general', 'random']],
      ['team:b', ['general']],
    ],
  );
  // The previous snapshot was replaced, not changed in place.
  assert.deepEqual([...(first.get('team:a') ?? [])], ['general']);
  store.set('team:a', 'general', '');
  store.clear('team:b');
  assert.deepEqual(
    [...store.drafted()].map(([team, channels]) => [team, [...channels]]),
    [['team:a', ['random']]],
  );
  assert.equal(calls, 5);
  stop();
  store.clearAll();
  assert.equal(store.drafted().size, 0);
  assert.equal(calls, 5);
});
