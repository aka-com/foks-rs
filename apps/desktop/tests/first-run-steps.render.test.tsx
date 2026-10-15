/**
 * The first-run step registry: every checkpoint state has a step, states that
 * share a page share a component, and a step reads its session from the
 * provider FirstRunSession renders.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';

installDom({ timers: true, act: true });

let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;

test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
});
test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

async function load() {
  const steps = (await vite.ssrLoadModule(
    '/src/screens/first-run/steps.tsx',
  )) as typeof import('../src/screens/first-run/steps');
  const state = (await vite.ssrLoadModule(
    '/src/first-run-state.ts',
  )) as typeof import('../src/first-run-state');
  return { ...steps, ...state };
}

test('every checkpoint state names a step', async () => {
  const { FIRST_RUN_STEPS, FIRST_RUN_STATES } = await load();
  assert.deepEqual(
    Object.keys(FIRST_RUN_STEPS).sort(),
    [...FIRST_RUN_STATES].sort(),
  );
  for (const state of FIRST_RUN_STATES)
    assert.equal(typeof FIRST_RUN_STEPS[state], 'function', state);
});

test('states drawn as one page share one step, so moving between them keeps it mounted', async () => {
  const { FIRST_RUN_STEPS: steps } = await load();
  for (const group of [
    ['boot', 'who'],
    ['address', 'no-address', 'error'],
    ['checked', 'compare'],
    ['account', 'existing'],
    ['protect', 'phrase'],
    ['checklist-invited', 'checklist-own'],
  ] as const)
    for (const state of group)
      assert.equal(steps[state], steps[group[0]], `${state} and ${group[0]}`);
  assert.notEqual(steps.account, steps.protect);
});

test('a step outside the setup session fails instead of rendering without one', async () => {
  const { FIRST_RUN_STEPS } = await load();
  const original = console.error;
  console.error = () => {};
  try {
    assert.throws(
      () => ui.render(createElement(FIRST_RUN_STEPS.local)),
      /outside FirstRunSession/,
    );
  } finally {
    console.error = original;
  }
});
