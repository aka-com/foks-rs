import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChatClock } from '../src/chat/clock';
import { DeadlineTimer } from '../src/chat/deadline-timer';

class Clock implements ChatClock {
  time = 0;
  id = 0;
  tasks = new Map<number, { due: number; fn: () => void }>();
  now = () => this.time;
  random = () => 0;
  later = (fn: () => void, delay: number) => {
    const id = ++this.id;
    this.tasks.set(id, { due: this.time + delay, fn });
    return id;
  };
  cancel = (id: unknown) => {
    this.tasks.delete(id as number);
  };
  advance(ms: number) {
    const end = this.time + ms;
    for (;;) {
      const next = [...this.tasks].sort((a, b) => a[1].due - b[1].due)[0];
      if (!next || next[1].due > end) break;
      this.time = next[1].due;
      this.tasks.delete(next[0]);
      next[1].fn();
    }
    this.time = end;
  }
  dues() {
    return [...this.tasks.values()].map((task) => task.due);
  }
}

test('armAt only brings the deadline forward', () => {
  const clock = new Clock();
  const fired: number[] = [];
  const timer = new DeadlineTimer(clock, () => fired.push(clock.now()));
  timer.armAt(500);
  timer.armAt(800);
  assert.equal(timer.deadline, 500);
  timer.armAt(200);
  assert.equal(timer.deadline, 200);
  assert.deepEqual(clock.dues(), [200]);
  clock.advance(1_000);
  assert.deepEqual(fired, [200]);
  assert.equal(timer.deadline, Infinity);
});

test('reset moves the deadline in either direction and Infinity arms nothing', () => {
  const clock = new Clock();
  const timer = new DeadlineTimer(clock, () => {});
  timer.armAt(200);
  timer.reset(900);
  assert.deepEqual(clock.dues(), [900]);
  timer.reset(Infinity);
  assert.equal(timer.deadline, Infinity);
  assert.deepEqual(clock.dues(), []);
  timer.armAt(Infinity);
  timer.armAt(Number.NaN);
  assert.deepEqual(clock.dues(), []);
});

test('a past deadline fires on the next turn and cancel disarms', () => {
  const clock = new Clock();
  clock.time = 1_000;
  let fired = 0;
  const timer = new DeadlineTimer(clock, () => fired++);
  timer.armAt(400);
  assert.deepEqual(clock.dues(), [1_000]);
  timer.cancel();
  assert.deepEqual(clock.dues(), []);
  clock.advance(10);
  assert.equal(fired, 0);
});

test('the callback may arm the timer again', () => {
  const clock = new Clock();
  const fired: number[] = [];
  const timer = new DeadlineTimer(clock, () => {
    fired.push(clock.now());
    if (fired.length < 3) timer.armAt(clock.now() + 100);
  });
  timer.armAt(100);
  clock.advance(1_000);
  assert.deepEqual(fired, [100, 200, 300]);
  assert.equal(timer.deadline, Infinity);
});
