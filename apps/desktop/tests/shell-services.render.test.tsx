/**
 * The services `VaultShell` provides to its screens and sheets: what they
 * hold, which of them keep their identity across shell renders, and what a
 * component sees outside the providers.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useRef } from 'react';
import type { RefObject } from 'react';
import type { CommandErrorHandler } from '../src/app/catalog-runtime';
import type { AgentSnapshot, Item } from '../src/model';
import type { ShellServices } from '../src/shell-services';
import { installDom } from './lib/dom-harness';
import { loadTestShell } from './lib/test-shell';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div>',
  timers: true,
  act: true,
});

let ui: typeof import('@testing-library/react');
test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());

/**
 * Renders the services hook with a catalog whose members keep their identity,
 * as the catalog runtime's do, and a command-error ref the test reassigns, as
 * the shell runtime does on each render.
 */
async function servicesHarness() {
  const { useShellServicesRuntime } =
    await import('../src/app/services-runtime');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { FIXTURE } = await import('../src/mock/fixture');
  const bridge = mockBridge(FIXTURE);
  const accessNow = () => 1;
  const latestRef: RefObject<AgentSnapshot> = { current: FIXTURE };
  const profileRefresh: RefObject<(profile: string) => Promise<void> | null> = {
    current: () => null,
  };
  const catalog = {
    latestRef,
    profileRefresh,
    refresh: async () => {},
    refreshSnapshot: async () => FIXTURE,
    reloadCatalog: async () => FIXTURE,
  };
  const seen: ShellServices[] = [];
  function Probe({ handler }: { handler: CommandErrorHandler }) {
    const commandErrorRef = useRef(handler);
    commandErrorRef.current = handler;
    seen.push(
      useShellServicesRuntime({
        bridge,
        accessNow,
        catalog: { ...catalog, commandErrorRef },
      }),
    );
    return null;
  }
  return { Probe, seen, bridge, accessNow };
}

test('the shell services keep their identity when the command-error handler changes', async () => {
  const { Probe, seen, bridge, accessNow } = await servicesHarness();
  const first: unknown[] = [];
  const second: unknown[] = [];
  const view = ui.render(
    createElement(Probe, { handler: (error) => first.push(error) }),
  );
  view.rerender(
    createElement(Probe, { handler: (error) => second.push(error) }),
  );
  assert.equal(seen.length, 2);
  const [before, after] = seen;
  assert.equal(before, after, 'a new handler replaced the services object');
  assert.equal(after.bridge, bridge);
  assert.equal(after.accessNow, accessNow);

  after.commandError('direct');
  assert.deepEqual(first, []);
  assert.deepEqual(second, ['direct']);
});

test('the shell command-error service passes the item and draft to the current handler', async () => {
  const { Probe, seen } = await servicesHarness();
  const calls: [unknown, Item | undefined, string | undefined][] = [];
  ui.render(
    createElement(Probe, {
      handler: (error, item, draft) => calls.push([error, item, draft]),
    }),
  );
  const { FIXTURE } = await import('../src/mock/fixture');
  const item = FIXTURE.items[0];
  seen[0].commandError('conflict', item, 'draft');
  assert.deepEqual(calls, [['conflict', item, 'draft']]);
});

test('the shell mutation-error service reports through the current handler', async () => {
  const { Probe, seen } = await servicesHarness();
  const first: unknown[] = [];
  const second: unknown[] = [];
  const view = ui.render(
    createElement(Probe, { handler: (error) => first.push(error) }),
  );
  view.rerender(
    createElement(Probe, { handler: (error) => second.push(error) }),
  );
  const failure = Object.assign(new Error('rejected'), {
    code: 'invalid-argument',
    retryable: false,
    fatal: false,
    ambiguous: false,
  });
  await seen[1].mutationError(failure);
  assert.deepEqual(first, []);
  assert.equal(second.length, 1);
});

test('a component outside the shell providers cannot read the services or the snapshot', async () => {
  const { useShellServices, useShellSnapshot } =
    await import('../src/shell-services');
  function Services() {
    useShellServices();
    return null;
  }
  function Snapshot() {
    useShellSnapshot();
    return null;
  }
  const quiet = console.error;
  console.error = () => {};
  try {
    assert.throws(
      () => ui.render(createElement(Services)),
      /outside ShellServicesProvider/,
    );
    assert.throws(
      () => ui.render(createElement(Snapshot)),
      /outside ShellSnapshotProvider/,
    );
  } finally {
    console.error = quiet;
  }
});

test('the shell snapshot is the one workflow availability reads without a snapshot of its own', async () => {
  const { useShellSnapshot } = await import('../src/shell-services');
  const { useWorkflowAccess } = await import('../src/workflow-context');
  const { FIXTURE } = await import('../src/mock/fixture');
  const TestShell = await loadTestShell();
  const stopped: AgentSnapshot = {
    ...FIXTURE,
    agent: { state: 'bootstrap', step: 'create-state' },
  };
  const read: { snapshot?: AgentSnapshot; available?: boolean } = {};
  function Probe() {
    read.snapshot = useShellSnapshot();
    read.available = useWorkflowAccess().availability('backup-revoke', {
      profile: 'personal',
      account: 'personal',
    }).available;
    return null;
  }
  const view = ui.render(
    createElement(TestShell, {
      snapshot: FIXTURE,
      children: createElement(Probe),
    }),
  );
  assert.equal(read.snapshot, FIXTURE);
  assert.equal(read.available, true);
  view.rerender(
    createElement(TestShell, {
      snapshot: stopped,
      children: createElement(Probe),
    }),
  );
  assert.equal(read.snapshot, stopped);
  assert.equal(read.available, false);
});
