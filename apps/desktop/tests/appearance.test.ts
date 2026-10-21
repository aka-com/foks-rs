import assert from 'node:assert/strict';
import test from 'node:test';
import { installDom } from './lib/dom-harness';
import {
  applyAppearance,
  mountAppearance,
  setAppearance,
  storedAppearance,
} from '../src/appearance';
installDom({ url: 'http://localhost/' });

test('appearance persists, follows system changes only in System mode, and syncs other windows', () => {
  let dark = true;
  const media = new window.EventTarget();
  Object.defineProperty(media, 'matches', { get: () => dark });
  const original = window.matchMedia.bind(window);
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    value: () => media,
  });
  const stop = mountAppearance();
  try {
    setAppearance('dark');
    assert.equal(document.documentElement.dataset.theme, 'dark');
    assert.equal(window.localStorage.getItem('appearance'), 'dark');
    setAppearance('system');
    dark = false;
    media.dispatchEvent(new window.Event('change'));
    assert.equal(document.documentElement.dataset.theme, 'light');
    setAppearance('light');
    dark = true;
    media.dispatchEvent(new window.Event('change'));
    assert.equal(document.documentElement.dataset.theme, 'light');
    window.localStorage.setItem('appearance', 'dark');
    window.dispatchEvent(
      new window.StorageEvent('storage', { key: 'appearance' }),
    );
    assert.equal(storedAppearance(), 'dark');
    assert.equal(document.documentElement.dataset.theme, 'dark');
    window.localStorage.setItem('appearance', 'invalid');
    window.dispatchEvent(
      new window.StorageEvent('storage', { key: 'appearance' }),
    );
    assert.equal(storedAppearance(), 'light');
    applyAppearance('light');
  } finally {
    stop();
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: original,
    });
    window.localStorage.removeItem('appearance');
  }
});

test('accent colors are distinct hues and retired choices map to their nearest one', async () => {
  const { RAIL_COLORS, resolveRailColor } = await import('../src/rail-theme');
  assert.deepEqual(
    RAIL_COLORS.map((color) => color.label),
    ['Blue', 'Purple', 'Pink', 'Green', 'Graphite'],
  );
  assert.equal(new Set(RAIL_COLORS.map((color) => color.fill)).size, 5);
  // A value stored by an earlier build keeps a sensible accent.
  assert.equal(resolveRailColor('iris'), 'purple');
  assert.equal(resolveRailColor('lavender'), 'purple');
  assert.equal(resolveRailColor('sky'), 'default');
  assert.equal(resolveRailColor('slate'), 'graphite');
  assert.equal(resolveRailColor('pink'), 'pink');
  assert.equal(resolveRailColor('nonsense'), 'default');
  assert.equal(resolveRailColor(null), 'default');
});

test('a retired accent stored by an earlier build is drawn as its current choice', async () => {
  const { applyRailColor, storedRailColor } = await import('../src/rail-theme');
  const root = document.documentElement;
  try {
    for (const [stored, drawn] of [
      ['iris', 'purple'],
      ['lavender', 'purple'],
      ['slate', 'graphite'],
    ] as const) {
      window.localStorage.setItem('railColor', stored);
      assert.equal(storedRailColor(), drawn);
      applyRailColor(storedRailColor());
      assert.equal(root.dataset.rail, drawn, stored);
    }
    // Sky maps to the default, which tokens.css draws with no attribute.
    window.localStorage.setItem('railColor', 'sky');
    applyRailColor(storedRailColor());
    assert.equal(root.dataset.rail, undefined);
    // A retired id passed straight in is resolved too.
    applyRailColor('iris');
    assert.equal(root.dataset.rail, 'purple');
  } finally {
    window.localStorage.removeItem('railColor');
    delete root.dataset.rail;
  }
});
