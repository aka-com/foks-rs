/**
 * The production build's startup JavaScript budget: which chunks count, and
 * the build failing once they exceed it.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  STARTUP_BUDGET_BYTES,
  startupBudget,
  startupJavaScript,
  type BundleEntries,
} from '../bundle-budget';

function chunk(code: string, imports: string[] = [], isEntry = false) {
  return { type: 'chunk' as const, isEntry, imports, code };
}

const BUNDLE: BundleEntries = {
  'index.js': chunk('a'.repeat(100), ['react.js', 'shared.js'], true),
  'react.js': chunk('b'.repeat(40)),
  'shared.js': chunk('c'.repeat(10), ['react.js']),
  // Reached only by dynamic import, so loaded after startup.
  'chat-tab.js': chunk('d'.repeat(500), ['shared.js']),
  'index.css': { type: 'asset' },
};

test('startup JavaScript is the entry chunk and the chunks it imports statically', () => {
  assert.deepEqual(startupJavaScript(BUNDLE), {
    files: ['index.js', 'react.js', 'shared.js'],
    bytes: 150,
  });
});

test('startup JavaScript counts bytes, not UTF-16 code units', () => {
  assert.equal(startupJavaScript({ 'i.js': chunk('›', [], true) }).bytes, 3);
});

/** Runs the plugin's `generateBundle` hook, returning what it reported. */
function runBudget(budget: number) {
  const plugin = startupBudget(budget);
  const reported: { info: string[]; error?: string } = { info: [] };
  const context = {
    info: (message: string) => reported.info.push(message),
    error: (message: string): never => {
      reported.error = message;
      throw new Error(message);
    },
  };
  const hook = plugin.generateBundle as unknown as (
    this: typeof context,
    options: unknown,
    bundle: BundleEntries,
  ) => void;
  try {
    hook.call(context, {}, BUNDLE);
  } catch {
    // The error is recorded in `reported`.
  }
  return reported;
}

test('a build within the budget reports its startup size', () => {
  const reported = runBudget(150);
  assert.equal(reported.error, undefined);
  assert.deepEqual(reported.info, [
    'startup JavaScript is 150 bytes in 3 chunks; the budget is 150 bytes',
  ]);
});

test('a build over the budget fails and names where the budget is set', () => {
  const reported = runBudget(149);
  assert.match(reported.error ?? '', /^startup JavaScript is 150 bytes/);
  assert.match(reported.error ?? '', /STARTUP_BUDGET_BYTES/);
});

test('the budget applies only to builds and is below the unsplit bundle', () => {
  assert.equal(startupBudget().apply, 'build');
  assert.ok(STARTUP_BUDGET_BYTES < 1_331_910);
});
