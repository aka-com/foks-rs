import test from 'node:test';
import assert from 'node:assert/strict';
import {DatabaseSync} from 'node:sqlite';
import {DENIED,runProbe} from './configuration.mjs';

function fixture({deny=true, override, rollbackFailure=false}={}) {
  const db = new DatabaseSync(':memory:');
  const storage = {
    sql: {exec(sql) {
      if (deny && DENIED.some(([,fixed]) => fixed === sql)) throw new Error('test authorizer shim');
      const replacement = override?.(sql,db);
      if (replacement !== undefined) return {toArray:()=>replacement};
      const rows = db.prepare(sql).all();
      return {toArray:()=>rows};
    }},
    transactionSync(callback) {
      db.exec('BEGIN');
      try { const result = callback(); db.exec('COMMIT'); return result; }
      catch(error) { db.exec('ROLLBACK'); if (rollbackFailure) throw new Error('foreign rollback failure'); throw error; }
    },
  };
  return {db,storage};
}

test('fixed settings and behavioral assertions pass with explicit denial shim, then rollback all main fixture objects',()=>{
  const {db,storage}=fixture();
  try {
    const result=runProbe(storage);
    assert.equal(result.passed,true);
    assert.deepEqual(result.settings,[1,0,1]);
    assert.deepEqual(result.denied,DENIED.map(([name])=>name));
    assert.equal(result.trusted_schema,'unqualified');
    assert.equal(result.production_admission,false);
    assert.equal(db.prepare("SELECT count(*) AS n FROM sqlite_schema WHERE name GLOB 'foks_connection_probe_*'").get().n,0);
  } finally {db.close();}
});

test('every denial candidate is valid and succeeds under unrestricted SQLite',()=>{
  for (const [,sql] of DENIED) {
    const {db,storage}=fixture({deny:false});
    try {
      db.exec('CREATE TABLE foks_connection_probe_parent(id INTEGER PRIMARY KEY)');
      assert.doesNotThrow(()=>storage.sql.exec(sql).toArray(),sql);
    } finally {db.close();}
  }
  const {db,storage}=fixture({deny:false});
  try {const result=runProbe(storage);assert.equal(result.passed,false);assert.match(result.error,/forbidden operation accepted/);}
  finally {db.close();}
});

test('malformed or ignored settings fail before fixture behavior is called',()=>{
  for (const value of [[],[{x:'1'}],[{x:2}],[{x:1,y:1}],[{x:null}]]) {
    const {db,storage}=fixture({override:sql=>sql==='PRAGMA foreign_keys'?value:undefined});
    try {assert.equal(runProbe(storage).passed,false);} finally {db.close();}
  }
  const {db,storage}=fixture({override:sql=>sql==='PRAGMA recursive_triggers=1'?[]:undefined});
  try {const result=runProbe(storage);assert.equal(result.passed,false);assert.equal(result.stage,'configure');}
  finally {db.close();}
});

test('spoofed settings cannot hide missing FK, CHECK or recursive enforcement',()=>{
  for (const [name,off,reported] of [['foreign_keys',0,1],['ignore_check_constraints',1,0],['recursive_triggers',0,1]]) {
    const {db,storage}=fixture({override:(sql,db)=>{
      if(sql==='PRAGMA '+name) return [{value:reported}];
      if(sql.startsWith('PRAGMA '+name+'=')){db.exec('PRAGMA '+name+'='+off);return [];}
    }});
    try {const result=runProbe(storage);assert.equal(result.passed,false);assert.equal(result.stage,'behavior');}
    finally {db.close();}
  }
});

test('unexpected SQL and rollback failures retain failed provenance',()=>{
  for (const mode of ['sql','rollback']) {
    const {db,storage}=fixture({rollbackFailure:mode==='rollback',override:sql=>{
      if(mode==='sql'&&sql==='INSERT INTO foks_connection_probe_parent VALUES(1)')throw new Error('unexpected fixture SQL failure');
    }});
    try {const result=runProbe(storage);assert.equal(result.passed,false);assert.match(result.error,/failure/);}
    finally {db.close();}
  }
});

test('denial that damages main schema cannot pass cookie restoration checks',()=>{
  const {db,storage}=fixture({deny:false,override:(sql,db)=>{
    if(DENIED.some(([,fixed])=>fixed===sql)){db.exec('CREATE TABLE damaged(v)');throw new Error('denied after mutation');}
  }});
  try {const result=runProbe(storage);assert.equal(result.passed,false);assert.match(result.error,/changed main cookie/);}
  finally {db.close();}
});


test('accepted qualified TEMP records real shadowing without changing failed qualification',()=>{
  const {db,storage}=fixture({deny:false,override:sql=>{
    if(DENIED.some(([name,fixed])=>fixed===sql && name!=='qualified_temp_table'))throw new Error('test authorizer shim');
  }});
  try {
    const result=runProbe(storage);
    assert.equal(result.passed,false);
    assert.match(result.error,/forbidden operation accepted: qualified_temp_table/);
    assert.equal(result.accepted_observation.main_count,'0');
    assert.equal(result.accepted_observation.temp_count,'1');
    assert.equal(result.accepted_observation.shadow,'confirmed');
    assert.equal(db.prepare("SELECT count(*) AS n FROM sqlite_schema WHERE name GLOB 'foks_connection_probe_*'").get().n,0);
  } finally {db.close();}
});

test('unavailable diagnostics cannot convert accepted operation into denial',()=>{
  const {db,storage}=fixture({deny:false,override:sql=>{
    if(sql.includes('sqlite_temp_schema'))throw new Error('diagnostic denied');
  }});
  try {
    const result=runProbe(storage);
    assert.equal(result.passed,false);
    assert.match(result.error,/forbidden operation accepted/);
    assert.equal(result.accepted_observation.temp_count,'unavailable');
  } finally {db.close();}
});
