import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  decodeAgentProcessInfo,
  decodeAgentStatus,
  decodeAppLockState,
  decodeMaintenanceSnapshot,
} from '../src/bridge/core';
import { array, record, string } from '../src/bridge/validation';

const fixture = record(
  JSON.parse(readFileSync(new URL('../src-tauri/wire-contract.json', import.meta.url), 'utf8')),
  'fixture',
);

const decoders: Record<string, (value: unknown) => unknown> = {
  agentStatusCases: decodeAgentStatus,
  agentProcessInfoCases: decodeAgentProcessInfo,
  appLockStateCases: decodeAppLockState,
  maintenanceCases: decodeMaintenanceSnapshot,
};

for (const [name, decode] of Object.entries(decoders)) {
  const cases = record(fixture[name], name);
  const valid = record(cases.valid, `${name}.valid`);
  test(`${name}: every shared native payload decodes without losing lifecycle facts`, () => {
    assert.ok(Object.keys(valid).length > 0);
    for (const [label, wire] of Object.entries(valid)) {
      assert.deepEqual(decode(wire), wire, label);
    }
  });

  test(`${name}: shared malformed combinations fail closed`, () => {
    const invalid = array(cases.invalid, `${name}.invalid`, record);
    assert.ok(invalid.length > 0);
    for (const [index, mutation] of invalid.entries()) {
      const base = string(mutation.base, 'base');
      assert.ok(Object.hasOwn(valid, base), base);
      const wire = structuredClone(record(valid[base], base));
      const path = array(mutation.path, 'path', string);
      assert.ok(path.length > 0);
      const field = path.pop()!;
      let target = wire;
      for (const part of path) target = record(target[part], part);
      if (Object.hasOwn(mutation, 'value')) target[field] = mutation.value;
      else {
        assert.ok(Object.hasOwn(target, field), field);
        delete target[field];
      }
      assert.throws(() => decode(wire), `${name}[${index}]: ${base}.${[...path, field].join('.')}`);
    }
  });

  test(`${name}: omitted lifecycle facts are never inferred`, () => {
    for (const [label, wire] of Object.entries(valid)) {
      for (const field of Object.keys(record(wire, label))) {
        const incomplete = { ...record(wire, label) };
        delete incomplete[field];
        assert.throws(() => decode(incomplete), `${label}.${field}`);
      }
    }
  });

  test(`${name}: unrelated extension fields retain existing normalization`, () => {
    for (const wire of Object.values(valid)) {
      assert.deepEqual(decode({ ...record(wire, 'wire'), extension: true }), wire);
    }
  });

  test(`${name}: non-object input cannot manufacture a lifecycle state`, () => {
    for (const wire of [undefined, null, [], true, 1, 'ready', {}]) {
      assert.throws(() => decode(wire));
    }
  });
}

test('process numeric fields reject non-JSON numbers and preserve explicit unknown metadata', () => {
  const valid = record(record(fixture.agentProcessInfoCases, 'cases').valid, 'valid');
  const owned = record(valid.owned, 'owned');
  for (const field of ['pid', 'startedAt']) {
    for (const value of [NaN, Infinity, -Infinity, undefined]) {
      assert.throws(() => decodeAgentProcessInfo({ ...owned, [field]: value }), field);
    }
  }
  assert.deepEqual(decodeAgentProcessInfo(valid.absent), {
    pid: null,
    executable: null,
    startedAt: null,
    owned: false,
  });
  assert.deepEqual(decodeAgentProcessInfo(valid.uninspectable), {
    pid: 44,
    executable: null,
    startedAt: null,
    owned: true,
  });
});

test('maintenance counters reject non-JSON numbers without inferring completion', () => {
  const valid = record(record(fixture.maintenanceCases, 'cases').valid, 'valid');
  for (const wire of Object.values(valid)) {
    for (const field of ['generation', 'revision']) {
      for (const value of [NaN, Infinity, -Infinity, -1, 0.5, undefined]) {
        assert.throws(() => decodeMaintenanceSnapshot({ ...record(wire, 'wire'), [field]: value }));
      }
    }
  }
});

test('maintenance operation and restoration errors remain independent', () => {
  const valid = record(record(fixture.maintenanceCases, 'cases').valid, 'valid');
  const snapshot = decodeMaintenanceSnapshot(valid.failedRestorationFailed);
  assert.equal(snapshot.state, 'complete');
  if (snapshot.state !== 'complete') assert.fail('Expected complete snapshot');
  assert.equal(snapshot.operation.status, 'failed');
  assert.equal(snapshot.disposition.status, 'restoration-failed');
  if (snapshot.operation.status !== 'failed' || snapshot.disposition.status !== 'restoration-failed') {
    assert.fail('Expected both operation and restoration failures');
  }
  assert.equal(snapshot.operation.error.message, 'Destination full');
  assert.equal(snapshot.disposition.error.message, 'Restore failed');
  assert.equal(snapshot.disposition.error.ambiguous, true);
  assert.equal(snapshot.disposition.error.fatal, true);
});
