/**
 * Tests for native drop routing: one drop reaches exactly one zone.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { DropRegistry } from '../src/file-drop';
import type { DropZone } from '../src/file-drop';

/** A zone recording what it was asked to handle. */
function zone(): DropZone & { hovers: boolean[]; drops: string[][] } {
  const hovers: boolean[] = [];
  const drops: string[][] = [];
  return {
    hovers,
    drops,
    onHover: (hovering) => hovers.push(hovering),
    onPaths: (paths) => drops.push(paths),
  };
}

test('the only claim receives drops', () => {
  const registry = new DropRegistry();
  const content = zone();
  registry.claim(content, 'background');
  assert.equal(registry.active(), content);
  assert.equal(registry.isActive(content), true);
});

test('a foreground claim takes the drop from the content area behind it', () => {
  const registry = new DropRegistry();
  const content = zone();
  const sheet = zone();
  registry.claim(content, 'background');
  registry.claim(sheet, 'foreground');
  assert.equal(registry.isActive(sheet), true);
  assert.equal(registry.isActive(content), false);
});

test('claim order does not let a later background claim outrank a sheet', () => {
  const registry = new DropRegistry();
  const sheet = zone();
  const content = zone();
  registry.claim(sheet, 'foreground');
  registry.claim(content, 'background');
  assert.equal(registry.isActive(sheet), true);
});

test('the newest of two equal claims wins, and releasing it restores the older', () => {
  const registry = new DropRegistry();
  const first = zone();
  const second = zone();
  registry.claim(first, 'foreground');
  const release = registry.claim(second, 'foreground');
  assert.equal(registry.isActive(second), true);
  release();
  assert.equal(registry.isActive(first), true);
});

test('no claim leaves nothing to route a drop to', () => {
  const registry = new DropRegistry();
  const content = zone();
  const release = registry.claim(content, 'background');
  release();
  assert.equal(registry.active(), null);
  assert.equal(registry.isActive(content), false);
});

test('releasing twice does not revive the claim', () => {
  const registry = new DropRegistry();
  const content = zone();
  const release = registry.claim(content, 'background');
  release();
  release();
  assert.equal(registry.active(), null);
});
