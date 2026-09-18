import assert from 'node:assert/strict';
import test from 'node:test';

import { collectSourceFiles, readSource, stripComments } from './lib/source';

test('navigation internals never import the compatibility facade', async () => {
  const files = await collectSourceFiles(
    new URL('../src/navigation/', import.meta.url),
  );
  for (const file of files) {
    const source = stripComments(await readSource(file.href, import.meta.url));
    assert.doesNotMatch(
      source,
      /from ['"][^'"]*\/location(?:\.ts)?['"]/,
      file.pathname,
    );
  }
});

test('production codecs and store dependencies do not import acceptance or React', async () => {
  for (const name of [
    'types',
    'routes',
    'transition',
    'legacy-routes',
    'production-codec',
    'scene-codec',
    'chat-tab-memory',
    'location-store',
  ]) {
    const source = stripComments(
      await readSource(`../src/navigation/${name}.ts`, import.meta.url),
    );
    assert.doesNotMatch(
      source,
      /from ['"][^'"]*(?:acceptance-codec|runtime-scene|fixture|mock-bridge|react)['"]/,
      name,
    );
    assert.doesNotMatch(source, /\b(?:window|document)\s*[.[]/, name);
    assert.doesNotMatch(
      source,
      /['"](?:team:eng|team:household|team:homelab|acct:personal|acct:work)['"]/,
      name,
    );
  }
});
