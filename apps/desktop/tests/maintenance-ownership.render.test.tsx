import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, StrictMode, useEffect } from 'react';
import { MaintenanceOwnership } from '../src/app/maintenance-ownership';
import { installDom } from './lib/dom-harness';

const dom = installDom({ act: true });

test.after(() => dom.window.close());

test('StrictMode replay retires the first owner and cleans up late registration', async () => {
  const ui = await import('@testing-library/react');
  const ownership = new MaintenanceOwnership();
  const registrations: (() => void)[] = [];
  const listeners = new Set<() => void>();
  const delivered: number[] = [];
  const stopped: number[] = [];
  const events: string[] = [];
  let epoch = 0;

  function Runtime() {
    useEffect(() => {
      const owner = ownership.acquire();
      const generation = ++epoch;
      const apply = () => {
        if (owner.isCurrent()) delivered.push(generation);
      };
      const pending = new Promise<() => void>((resolve) => {
        registrations.push(() => {
          listeners.add(apply);
          events.push(`listen:${generation}`);
          resolve(() => {
            listeners.delete(apply);
            stopped.push(generation);
          });
        });
      });
      void pending.then((stop) => {
        owner.install(stop);
        if (!owner.isCurrent()) return;
        events.push(`snapshot:${generation}`);
        apply();
      });
      return () => owner.retire();
    }, []);
    return null;
  }

  const rendered = ui.render(
    createElement(StrictMode, null, createElement(Runtime)),
  );
  try {
    assert.equal(registrations.length, 2);
    await ui.act(async () => {
      registrations[1]();
      registrations[0]();
    });
    assert.deepEqual(stopped, [1]);
    assert.deepEqual(delivered, [2]);
    assert.equal(listeners.size, 1);
    assert.ok(events.indexOf('listen:2') < events.indexOf('snapshot:2'));
    assert.equal(events.includes('snapshot:1'), false);
    for (const listener of listeners) listener();
    assert.deepEqual(delivered, [2, 2]);
  } finally {
    rendered.unmount();
  }
  assert.equal(listeners.size, 0);
  assert.deepEqual(stopped, [1, 2]);
});
