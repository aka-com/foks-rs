/**
 * Static analysis tests ensuring strict React boundaries:
 * - No raw HTML assignment sinks.
 * - No direct access to window.__TAURI__ or unbrokered IPC imports.
 * - Pure model boundaries without DOM or fixture dependencies.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { collectSourceFiles, readSource, stripComments } from './lib/source';

const RAW_HTML_SINKS = [
  /\.innerHTML\s*=/,
  /\.outerHTML\s*=/,
  /insertAdjacentHTML\s*\(/,
  /dangerouslySetInnerHTML/,
  /document\.write/,
  /createContextualFragment\s*\(/,
];

/** The app's own sources, including its presentation kit. */
async function firstPartySources(): Promise<URL[]> {
  const [src, kit] = await Promise.all([
    collectSourceFiles(new URL('../src/', import.meta.url)),
    collectSourceFiles(new URL('../kit/', import.meta.url)),
  ]);
  return [...src, ...kit];
}

test('first-party UI rendering has no raw HTML assignment sink', async () => {
  for (const file of await firstPartySources()) {
    const source = stripComments(await readSource(file.href, import.meta.url));
    for (const sink of RAW_HTML_SINKS) {
      assert.doesNotMatch(
        source,
        sink,
        `${file.pathname} contains raw HTML sink ${String(sink)}`,
      );
    }
  }
});

test('nothing reads window.__TAURI__', async () => {
  // Tauri global IPC object is disabled; verify window.__TAURI__ is never referenced.
  for (const file of await firstPartySources()) {
    const source = stripComments(await readSource(file.href, import.meta.url));
    assert.doesNotMatch(source, /__TAURI__\b/, file.pathname);
  }
});

test('only the bridge imports the Tauri API', async () => {
  const transports = [
    '/src/bridge/transport.ts',
    '/src/bridge/commands-core.ts',
    '/src/bridge/commands-vault.ts',
  ];
  for (const file of await firstPartySources()) {
    if (transports.some((path) => file.pathname.endsWith(path))) continue;
    const source = stripComments(await readSource(file.href, import.meta.url));
    assert.doesNotMatch(source, /from '@tauri-apps\//, file.pathname);
  }
  const bridge = await readSource(
    '../src/bridge/transport.ts',
    import.meta.url,
  );
  assert.match(bridge, /import \{ invoke \} from '@tauri-apps\/api\/core';/);
});

test('the model is pure: no DOM, no bridge, no fixture', async () => {
  // Model functions must remain pure without accessing global variables.
  for (const file of await collectSourceFiles(
    new URL('../src/model/', import.meta.url),
  )) {
    const source = stripComments(await readSource(file.href, import.meta.url));
    // Verify absence of DOM and bridge member property access.
    assert.doesNotMatch(
      source,
      /\b(document|window|globalThis)\s*[.[]/,
      file.pathname,
    );
    assert.doesNotMatch(
      source,
      /from '\.\.\/(fixture|bridge|mock-bridge)'/,
      file.pathname,
    );
    assert.doesNotMatch(source, /from 'react'/, file.pathname);
  }
});

test('the mock bridge is reached only by dynamic import, so it can be dropped', async () => {
  // Dynamic import ensures test fixture modules are excluded from production builds.
  const bridge = await readSource(
    '../src/bridge/selection.ts',
    import.meta.url,
  );
  assert.doesNotMatch(bridge, /^import .*mock-bridge/m);
  assert.match(bridge, /await import\('\.\.\/mock-bridge'\)/);
  const root = await readSource('../src/app-root.tsx', import.meta.url);
  assert.doesNotMatch(root, /mock-bridge|\.\/fixture/);
});

test('ordinary production modules never import the fixture graph', async () => {
  for (const file of await firstPartySources()) {
    if (
      file.pathname.endsWith('/src/fixture.ts') ||
      file.pathname.endsWith('/src/mock-bridge.ts')
    )
      continue;
    const source = stripComments(await readSource(file.href, import.meta.url));
    assert.doesNotMatch(source, /from ['"][^'"]*fixture['"]/, file.pathname);
    assert.doesNotMatch(
      source,
      /from ['"][^'"]*mock-bridge['"]/,
      file.pathname,
    );
  }
});

test('bridge verifies Tauri runtime presence and checks VITE_FOKS_MOCK flag', async () => {
  const bridge = await readSource(
    '../src/bridge/selection.ts',
    import.meta.url,
  );
  assert.match(bridge, /'__TAURI_INTERNALS__' in window/);
  assert.match(bridge, /import\.meta\.env\?\.VITE_FOKS_MOCK === '1'/);
  assert.match(
    await readSource('../src/bridge.ts', import.meta.url),
    /The only seam between the FOKS webview and the local agent/,
  );
});

test('icons are structured data, not markup strings', async () => {
  // Keep icon geometry as structured data so no raw-markup sink is needed.
  const icons = stripComments(
    await readSource('../src/icons.ts', import.meta.url),
  );
  assert.doesNotMatch(icons, /<svg|<path|<circle|<rect/);
});
