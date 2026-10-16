import test from 'node:test';
import assert from 'node:assert/strict';
import {DatabaseSync} from 'node:sqlite';
import {ATTACH,ATTACHED_PRESENT,CANDIDATES,TEMP_PRESENT,runProbe} from './connection-state.mjs';

function fixture({denyAttach=true, denyCandidates=[], override, rollback='normal', attachThenThrow=false}={}) {
  const db = new DatabaseSync(':memory:');
  const storage = {
    sql:{exec(sql) {
      if (attachThenThrow && sql === ATTACH) {db.exec(sql);return {toArray(){throw new Error('completion failure after ATTACH');}};}
      if ((denyAttach && sql === ATTACH) || denyCandidates.includes(sql)) throw new Error('explicit test denial shim');
      const replacement = override?.(sql,db);
      return {toArray:()=>replacement === undefined ? db.prepare(sql).all() : replacement};
    }},
    transactionSync(callback) {
      db.exec('BEGIN');
      try {const result=callback();db.exec('COMMIT');return result;}
      catch(error) {
        if (rollback === 'commit') db.exec('COMMIT'); else db.exec('ROLLBACK');
        if (rollback === 'foreign') throw new Error('foreign rollback error');
        if (rollback === 'return') return;
        throw error;
      }
    },
  };
  return {db,storage};
}

test('explicit TEMP query detects tables, views and main-targeting triggers before owned rollback',()=>{
  const {db,storage}=fixture();
  try {
    const r=runProbe(storage);
    assert.equal(r.passed,true);
    assert.deepEqual(r.outcomes,CANDIDATES.map(([name])=>({name,outcome:'detected'})));
    assert.equal(r.production_admission,false);assert.equal(r.trusted_schema,'unqualified');
    assert.equal(r.attach_denied,true);assert.equal(r.final_temp_empty,true);
    assert.equal(r.final_attached_absent,true);
    assert.equal(db.prepare(TEMP_PRESENT).get().value,'0');
    assert.equal(db.prepare('SELECT count(*) AS n FROM main.sqlite_schema').get().n,0);
  } finally {db.close();}
});

test('denied candidates are distinct from positive inspection; universal denial cannot qualify',()=>{
  for (const all of [false,true]) {
    const {db,storage}=fixture({denyCandidates:CANDIDATES.filter(([name])=>all||name!=='qualified_temp_table').map(([,sql])=>sql)});
    try {
      const r=runProbe(storage);assert.equal(r.passed,!all);
      assert.deepEqual(r.outcomes.map(o=>o.outcome),all?['denied','denied','denied','denied']:['denied','detected','denied','denied']);
    } finally {db.close();}
  }
});

test('preexisting TEMP objects fail admission before mutation',()=>{
  for(const sql of ['CREATE TEMP TABLE prior(value)','CREATE TEMP VIEW prior AS SELECT 1','CREATE TABLE target(value); CREATE TEMP TRIGGER prior AFTER INSERT ON main.target BEGIN SELECT 1; END']) {
    const {db,storage}=fixture();
    try {db.exec(sql);const r=runProbe(storage);assert.equal(r.passed,false);assert.equal(r.stage,'initial-temp');assert.deepEqual(r.outcomes,[]);}
    finally {db.close();}
  }
});

test('malformed, denied or spoofed TEMP inspection cannot produce a pass',()=>{
  for(const replacement of [[],[{value:0}],[{value:'2'}],[{value:'0',extra:1}],[{value:'0'}],null]) {
    const {db,storage}=fixture({override:sql=>{if(sql===TEMP_PRESENT){if(replacement===null)throw new Error('inspection unavailable');return replacement;}}});
    try {assert.equal(runProbe(storage).passed,false);} finally {db.close();}
  }
});

test('foreign rollback, swallowed rollback and committed fixture cannot pass',()=>{
  for(const rollback of ['foreign','return','commit']) {
    const {db,storage}=fixture({rollback});
    try {const r=runProbe(storage);assert.equal(r.passed,false);assert.match(r.error,/rollback/);}
    finally {db.close();}
  }
});

test('setup and post-candidate errors are never classified as valid denial',()=>{
  for(const failed of ['CREATE TABLE main.foks_connection_state_target(value)',"SELECT 'alive' AS value"]) {
    const {db,storage}=fixture({override:sql=>{if(sql===failed)throw new Error('unrelated failure');}});
    try {const r=runProbe(storage);assert.equal(r.passed,false);assert.match(r.error,/unrelated failure/);assert.deepEqual(r.outcomes,[]);}
    finally {db.close();}
  }
});

test('accepted ATTACH fails independently after all TEMP candidates and is not called rolled back',()=>{
  const {db,storage}=fixture({denyAttach:false});
  try {const r=runProbe(storage);assert.equal(r.passed,false);assert.equal(r.stage,'attach');assert.equal(r.outcomes.length,4);assert.match(r.error,/ATTACH accepted/);assert.equal(r.rollback_restored,undefined);assert.ok(db.prepare('PRAGMA database_list').all().some(row=>row.name==='foks_connection_state_attached'));}
  finally {db.close();}
});

test('post-ATTACH cursor failure cannot masquerade as denial',()=>{
  const {db,storage}=fixture({attachThenThrow:true});
  try {const r=runProbe(storage);assert.equal(r.passed,false);assert.equal(r.stage,'attach');assert.match(r.error,/attachment survived/);assert.equal(db.prepare(ATTACHED_PRESENT).get().value,'1');}
  finally {db.close();}
});

test('unavailable or malformed attachment inspection fails before any candidate',()=>{
  for(const replacement of [null,[],[{value:0}],[{value:'2'}],[{value:'0',extra:1}]]) {
    const {db,storage}=fixture({override:sql=>{if(sql===ATTACHED_PRESENT){if(replacement===null)throw new Error('attachment inventory unavailable');return replacement;}}});
    try {const r=runProbe(storage);assert.equal(r.passed,false);assert.equal(r.stage,'initial-attachments');assert.deepEqual(r.outcomes,[]);}
    finally {db.close();}
  }
});

test('preexisting attachment is rejected independently of TEMP',()=>{
  const {db,storage}=fixture();
  try {db.exec("ATTACH ':memory:' AS prior");const r=runProbe(storage);assert.equal(r.passed,false);assert.equal(r.stage,'initial-attachments');assert.match(r.error,/initial attached/);}
  finally {db.close();}
});
