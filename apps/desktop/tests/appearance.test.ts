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
  const original = window.matchMedia;
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
