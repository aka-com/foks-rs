import test from 'node:test';
import platformSchema from './celld-directory-schema.json' with {type:'json'};
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {join} from 'node:path';
import {DatabaseSync} from 'node:sqlite';
import {authorize,readBody,seed,assertState,DirectoryCase,SLICE} from './directory-fixture.mjs';
const root=process.env.FOKS_DIRECTORY_VECTORS;if(!root)throw new Error('reviewed directory fixtures required');
const fixture=JSON.parse(readFileSync(join(root,'directory-vectors.json'),'utf8'));
const sha=async bytes=>Buffer.from(await crypto.subtle.digest('SHA-256',bytes)).toString('hex');
const stream=bytes=>new ReadableStream({start(c){for(let i=0;i<bytes.length;i+=31)c.enqueue(bytes.subarray(i,i+31));c.close();}});
function storage(){const db=new DatabaseSync(':memory:');return {db,sync:async()=>{},transactionSync(fn){db.exec('BEGIN');try{const v=fn();db.exec('COMMIT');return v;}catch(e){db.exec('ROLLBACK');throw e;}},sql:{exec(sql,...params){const q=db.prepare(sql);q.setReadBigInts(true);const columns=q.columns().map(c=>c.name);if(!columns.length){q.run(...params);return {raw(){return [][Symbol.iterator]();}};}return {raw(){const rows=q.iterate(...params);return {next(){const row=rows.next();return row.done?row:{done:false,value:columns.map(c=>row.value[c])};},[Symbol.iterator](){return this;}};}};}}};}
test('body length/digest/type failures are rejected before SQL and consume the case',async()=>{
 const exact=new Uint8Array([1,2,3]),v={...fixture.vectors[0],length:3,sha256:await sha(exact)};
 for(const input of [null,stream(new Uint8Array([1,2])),stream(new Uint8Array([1,2,3,4])),stream(new Uint8Array([1,2,4]))]){
  let calls=0;const s={sql:{exec(){calls++;throw new Error('unexpected SQL');}}},c=new DirectoryCase(s,{...fixture,vectors:[v]},()=>{throw new Error('unexpected kernel');});
  await assert.rejects(c.run(0,input),/fixture/);assert.equal(calls,0);await assert.rejects(c.run(0,stream(exact)),/already used/);assert.equal(calls,0);
 }
 assert.deepEqual(await readBody(stream(exact),v),exact);
});
test('private capability/index/method are checked without consuming body',()=>{
 const init={method:'POST',headers:{authorization:'Bearer private'}};
 assert.deepEqual(authorize(new Request('https://fixture/0',init),'private',fixture),{index:0});
 assert.equal(authorize(new Request('https://fixture/0',init),'wrong',fixture).status,403);
 assert.equal(authorize(new Request('https://fixture/8',init),'private',fixture).status,404);
 assert.equal(authorize(new Request('https://fixture/0'),'private',fixture).status,405);
});
test('complete binary comparison catches middle/last byte and metadata/other-table/schema changes',()=>{
 for(const mutation of ['middle','last','metadata','head','other','schema']){
  const s=storage(),exact=new Uint8Array(SLICE*2+7).fill(91);seed(s,fixture,exact);assertState(s,fixture,exact);
  if(mutation==='middle'||mutation==='last'){
   const changed=exact.slice();changed[mutation==='middle'?SLICE+1:changed.length-1]^=1;
   s.db.prepare('UPDATE kv_directories SET exact_directory=?').run(changed);
  }else if(mutation==='metadata')s.db.exec('UPDATE kv_directories SET key_generation=key_generation+1');
  else if(mutation==='head')s.db.exec('DELETE FROM kv_directory_heads');
  else if(mutation==='other')s.db.exec('UPDATE kv_namespaces SET accounted_bytes=accounted_bytes+1');
  else s.db.exec('CREATE TABLE unexpected_fixture_table(value INTEGER)');
  assert.throws(()=>assertState(s,fixture,exact),/fixture/);s.db.close();
 }
});
test('exact slice multiples and empty terminal slice cover the full retained payload',()=>{
 for(const size of [1,SLICE,SLICE+1,SLICE*2]){const s=storage(),exact=new Uint8Array(size).fill(37);seed(s,fixture,exact);assertState(s,fixture,exact);s.db.close();}
});

test('schema label does not require forbidden fixture user_version SQL',()=>{
 const s=storage(),original=s.sql.exec;let calls=0;
 s.sql.exec=(sql,...params)=>{assert.doesNotMatch(sql,/^\s*PRAGMA\s+user_version/i);calls++;return original(sql,...params);};
 const exact=new Uint8Array(1024).fill(7);seed(s,fixture,exact);assertState(s,fixture,exact);assert.ok(calls>0);
 assert.equal(s.db.prepare('PRAGMA user_version').get().user_version,0);s.db.close();
});

test('pinned platform schema is exact and never a reserved-name wildcard',()=>{
 assert.equal(platformSchema.length,10);
 for(const mutation of ['none','missing','changed','extra']){
  const s=storage();for(const object of platformSchema)s.db.exec(object.sql);
  const exact=new Uint8Array(1024).fill(13);seed(s,fixture,exact);assertState(s,fixture,exact,platformSchema);
  if(mutation==='missing')s.db.exec('DROP TABLE _cf_METADATA');
  else if(mutation==='changed')s.db.exec('ALTER TABLE _cf_METADATA ADD COLUMN unexpected TEXT');
  else if(mutation==='extra')s.db.exec('CREATE TABLE _cf_EXTRA(value INTEGER)');
  if(mutation!=='none')assert.throws(()=>assertState(s,fixture,exact,platformSchema),/schema changed/);
  else assert.throws(()=>assertState(s,fixture,exact),/schema changed/);
  s.db.close();
 }
});
