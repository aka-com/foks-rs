import assert from 'node:assert/strict';
import test from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import { COOKIE_SQL, runProbe } from './probe.mjs';
import worker, { SchemaCookieProbe } from './worker.mjs';

function native(override) {
  const database = new DatabaseSync(':memory:');
  const storage = {
    sql: {},
    transactionSync(callback) {
      database.exec('BEGIN');
      try { const result = callback(); database.exec('COMMIT'); return result; }
      catch (error) { database.exec('ROLLBACK'); throw error; }
    },
    sync: async () => {},
  };
  // celld exec starts SQL immediately even if the result is never enumerated.
  storage.sql.exec = (sql) => {
    if (override) { const result = override(sql); if (result !== undefined) return { toArray: () => result }; }
    const result = database.prepare(sql).all();
    return { toArray: () => result };
  };
  return { storage, database };
}
test('native exact candidate preserves rollback and committed DDL relations', () => {
  const { storage, database } = native();
  try {
    const result = runProbe(storage);
    assert.equal(result.passed, true);
    assert.equal(result.phases.length, 11);
    assert.ok(result.phases.every((p) => p.type === 'string'));
    assert.equal(database.prepare("SELECT count(*) AS n FROM sqlite_schema WHERE name='foks_cookie_probe_only'").get().n, 0);
  } finally { database.close(); }
});
test('candidate denial remains failed even when read-only PRAGMA succeeds', () => {
  const { storage, database } = native((sql) => { if (sql === COOKIE_SQL) throw new Error('authorizer denied candidate'); });
  try {
    const result = runProbe(storage);
    assert.equal(result.passed, false);
    assert.equal(result.stage, 'initial');
    assert.equal(result.pragma_diagnostic.allowed, true);
  } finally { database.close(); }
});
test('cookie shape, type and canonical uint32 bounds fail closed', () => {
  for (const bad of [[], [{ a: '1', b: '2' }], [{ a: 1 }], [{ a: '01' }], [{ a: '-1' }], [{ a: '4294967296' }]]) {
    const { storage, database } = native((sql) => sql === COOKIE_SQL ? bad : undefined);
    try { assert.equal(runProbe(storage).passed, false); } finally { database.close(); }
  }
});
test('unrelated transaction exception is not the owned rollback sentinel', () => {
  const { storage, database } = native();
  const transaction = storage.transactionSync;
  let calls = 0;
  storage.transactionSync = (callback) => { if (++calls === 2) throw new Error('foreign rollback failure'); return transaction(callback); };
  try { const result = runProbe(storage); assert.equal(result.passed, false); assert.match(result.error, /foreign rollback failure/); }
  finally { database.close(); }
});
test('capability checked before outer cell lookup and inner SQL', async () => {
  const env = { PROBE_TOKEN: 'x'.repeat(40), COOKIE_PROBE: { idFromName() { throw new Error('cell acquired'); } } };
  const request = new Request('https://probe.test/', { method: 'POST', headers: { authorization: 'Bearer wrong' } });
  assert.equal((await worker.fetch(request, env)).status, 403);
  const cell = new SchemaCookieProbe({ storage: { sql: { exec() { throw new Error('SQL executed'); } } } }, env);
  assert.equal((await cell.fetch(request)).status, 403);
});
test('one attempt and post-transaction sync failure never return success', async () => {
  const { storage, database } = native();
  storage.sync = async () => { throw new Error('sync failed'); };
  const env = { PROBE_TOKEN: 'x'.repeat(40) };
  const request = () => new Request('https://probe.test/', { method: 'POST', headers: { authorization: `Bearer ${env.PROBE_TOKEN}` } });
  try {
    const cell = new SchemaCookieProbe({ storage }, env);
    const response = await cell.fetch(request());
    assert.equal(response.status, 500);
    assert.equal((await response.json()).passed, false);
    assert.equal((await cell.fetch(request())).status, 409);
  } finally { database.close(); }
});
