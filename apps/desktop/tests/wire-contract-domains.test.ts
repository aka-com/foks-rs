import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import {
  loadDomains,
  mergeDomains,
  parseUniqueJson,
} from '../src-tauri/wire-contract/assemble.mjs';

test('domain golden records exactly reproduce the native-compatible aggregate', async () => {
  const { aggregate } = await loadDomains();
  const existing = parseUniqueJson(
    await readFile(
      new URL('../src-tauri/wire-contract.json', import.meta.url),
      'utf8',
    ),
  );
  assert.deepEqual(aggregate, existing);
});

test('fixture assembly rejects duplicate record ownership and nested JSON keys', () => {
  assert.throws(
    () =>
      mergeDomains({
        first: { record: { value: 1 } },
        second: { record: { value: 1 } },
      }),
    /duplicate record record/,
  );
  assert.throws(
    () => parseUniqueJson('{"record":1,"record":2}'),
    /duplicate key record/,
  );
  assert.throws(
    () => parseUniqueJson('{"record":{"value":1,"value":2}}'),
    /duplicate key value/,
  );
  assert.throws(
    () => parseUniqueJson('{"record":[{"value":1,"value":2}]}'),
    /duplicate key value/,
  );
  assert.throws(
    () => parseUniqueJson('{"record":1,"\\u0072ecord":2}'),
    /duplicate key record/,
  );
  assert.deepEqual(parseUniqueJson('{"a":{"value":1},"b":{"value":2}}'), {
    a: { value: 1 },
    b: { value: 2 },
  });
});

test('every golden record has explicit domain and shape or semantic-assertion coverage', async () => {
  const { inventory, aggregate } = await loadDomains();
  const represented = new Set(
    Object.values(inventory.representedShapes).flat(),
  );
  const semanticAssertions = ['serverStatusRequiredFields'];
  for (const key of semanticAssertions) represented.add(key);
  assert.deepEqual([...represented].sort(), Object.keys(aggregate).sort());
  assert.equal(inventory.generation.status, 'deferred');
  assert.match(inventory.generation.reason, /semantic normalization/);
  assert.ok(
    inventory.uncoveredPublicShapes.application.includes('AgentProcessInfo'),
  );
  assert.ok(
    inventory.uncoveredPublicShapes.application.includes('MaintenanceSnapshot'),
  );
  assert.ok(
    inventory.uncoveredPublicShapes.invitations.includes(
      'InvitationRow / InvitationReply',
    ),
  );
  assert.ok(
    inventory.uncoveredPublicShapes.chat.includes('ChatAction / ChatReply'),
  );
  assert.ok(
    inventory.uncoveredPublicShapes.vault.includes('CatalogFailureDto'),
  );
});
