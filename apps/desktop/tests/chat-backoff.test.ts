import assert from 'node:assert/strict';
import test from 'node:test';
import {
  FALLBACK_BASE_MS,
  FALLBACK_MAX_MS,
  GeometricInterval,
  REQUEST_RETRY,
  SYNC_RETRY,
  retryDelay,
} from '../src/chat/backoff';

test('request retries start at the 2 s check interval and double to 30 s', () => {
  assert.deepEqual(
    [1, 2, 3, 4, 5, 6, 40].map((failures) =>
      retryDelay(failures, REQUEST_RETRY),
    ),
    [2_000, 4_000, 8_000, 16_000, 30_000, 30_000, 30_000],
  );
});

test('synchronization retries add up to a quarter of the delay without passing the cap', () => {
  const none = () => 0;
  const most = () => 0.999;
  assert.deepEqual(
    [1, 2, 3, 7, 8, 2_000].map((failures) =>
      retryDelay(failures, SYNC_RETRY, none),
    ),
    [250, 500, 1_000, 16_000, 30_000, 30_000],
  );
  assert.equal(retryDelay(1, SYNC_RETRY, most), 312);
  assert.equal(retryDelay(7, SYNC_RETRY, most), 19_996);
  assert.equal(retryDelay(8, SYNC_RETRY, most), 30_000);
});

test('a policy without jitter never reads the random source', () => {
  retryDelay(3, REQUEST_RETRY, () => assert.fail('random was read'));
});

test('a fallback schedule doubles on quiet reads and returns to the base on movement', () => {
  let schedule = GeometricInterval.base(0);
  assert.equal(schedule.due(false), FALLBACK_BASE_MS);
  const intervals: number[] = [];
  for (let read = 1; read <= 6; read++) {
    schedule = schedule.next(false, read * 1_000);
    intervals.push(schedule.interval);
  }
  assert.deepEqual(intervals, [
    10_000,
    20_000,
    40_000,
    FALLBACK_MAX_MS,
    FALLBACK_MAX_MS,
    FALLBACK_MAX_MS,
  ]);
  assert.equal(schedule.at, 6_000);
  assert.equal(schedule.due(false), 6_000 + FALLBACK_MAX_MS);
  const moved = schedule.next(true, 7_000);
  assert.equal(moved.interval, FALLBACK_BASE_MS);
  assert.equal(moved.due(false), 7_000 + FALLBACK_BASE_MS);
});

test('the presented channel is due at the base while its stored interval keeps its backoff', () => {
  const schedule = GeometricInterval.base(0)
    .next(false, 1_000)
    .next(false, 2_000);
  assert.equal(schedule.due(true), 2_000 + FALLBACK_BASE_MS);
  assert.equal(schedule.due(false), 2_000 + 4 * FALLBACK_BASE_MS);
  // A schedule that was never read is due at once, presented or not.
  const fresh = GeometricInterval.base(-Infinity);
  assert.equal(fresh.due(true), -Infinity);
  assert.equal(fresh.due(false), -Infinity);
});
