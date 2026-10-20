import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {join} from 'node:path';
import {DatabaseSync} from 'node:sqlite';
import platform from './celld-directory-schema.json' with {type:'json'};
import {SchemaCase,authorize,emptyBody,snapshot,NAMES} from './schema-enumeration.mjs';
const root=process.env.FOKS_SCHEMA_ENUMERATION_VECTORS,wasm=process.env.FOKS_SCHEMA_ENUMERATION_WASM;
if(!root||!wasm)throw new Error('reviewed native schema scripts and Wasm required');
const fixture=JSON.parse(readFileSync(join(root,'vectors.json'),'utf8'));
const catalog=JSON.parse(readFileSync(join(root,'catalog.json'),'utf8'));
const pages=JSON.parse(readFileSync(join(root,'page-catalog.json'),'utf8'));
const module=new WebAssembly.Module(readFileSync(wasm));
function storage(){
 const db=new DatabaseSync(':memory:');for(const o of platform)db.exec(o.sql);
 const storage={db,calls:0,sync:async()=>{},transactionSync(fn){db.exec('BEGIN');try{const value=fn();db.exec('COMMIT');return value;}catch(error){db.exec('ROLLBACK');if(storage.rollbackError)throw storage.rollbackError;throw error;}},sql:{exec(sql,...params){storage.calls++;if(storage.failSql?.(sql))throw new Error('unexpected fixture SQL denial');const q=db.prepare(sql);q.setReadBigInts(true);const columns=q.columns().map(c=>c.name);if(!columns.length){q.run(...params);return {raw(){return [][Symbol.iterator]();}};}return {raw(){const rows=q.iterate(...params);return {next(){const row=rows.next();return row.done?row:{done:false,value:columns.map(c=>row.value[c])};},[Symbol.iterator](){return this;}};}};}}};
 return storage;
}
const instance=(s,f=fixture)=>new SchemaCase(s,f,module,catalog,pages);
test('every native full-union and negative script matches actual Wasm pages and rolls back',async()=>{
 assert.deepEqual(fixture.vectors.map(v=>v.name),NAMES);
 for(let i=0;i<NAMES.length;i++){const s=storage();try{const result=await instance(s).run(i);assert.equal(result.passed,true,NAMES[i]);assert.equal(result.complete,!['name_257','sql_65537'].includes(NAMES[i]));assert.equal(result.union_matches,NAMES[i].startsWith('full_')||NAMES[i].startsWith('suffix_'));}finally{s.db.close();}}
});
test('private capability and empty body reject before cell acquisition or SQL',async()=>{
 let acquired=0;const route=async request=>{const result=authorize(request,'private',fixture);if(result.status)return result.status;await emptyBody(request);acquired++;return 200;};
 assert.equal(await route(new Request('https://fixture/0',{method:'POST'})),403);
 assert.equal(await route(new Request('https://fixture/99',{method:'POST',headers:{authorization:'Bearer private'}})),404);
 assert.equal(await route(new Request('https://fixture/0')),405);
 await assert.rejects(route(new Request('https://fixture/0',{method:'POST',headers:{authorization:'Bearer private'},body:'not empty'})),/body must be empty/);
 assert.equal(acquired,0);assert.equal(await route(new Request('https://fixture/0',{method:'POST',headers:{authorization:'Bearer private'},body:''})),200);
});
test('wrong platform definition is rejected before FOKS seeding',async()=>{
 for(const change of ['missing','changed','extra']){const s=storage();try{if(change==='missing')s.db.exec('DROP TRIGGER _cf_WAKE_update');else if(change==='changed')s.db.exec('ALTER TABLE _cf_METADATA ADD COLUMN extra TEXT');else s.db.exec('CREATE TABLE sqliteX_unexpected(v)');await assert.rejects(instance(s).run(0),/unpinned fresh platform/);assert.equal(s.db.prepare("SELECT count(*) AS n FROM sqlite_schema WHERE name='metadata_read_dependency'").get().n,0);}finally{s.db.close();}}
});
test('full snapshot preserves literal prefix, NULLs, blobs and UTF8 names',()=>{
 const s=storage();try{s.db.exec('CREATE TABLE sqliteX_extension(v UNIQUE)');assert.ok(snapshot(s).some(o=>o.name==='sqliteX_extension'));}finally{s.db.close();}
 // NodeSQLite defensive mode forbids writable_schema corruption. Feed fixed
 // malformed raw rows to this wrapper; this is not an engine-corruption gate.
 for(const row of [['table',null,'x','CREATE TABLE x(v)'],['table',new Uint8Array([115,113,108,105,116,101,95]),'x','CREATE TABLE x(v)'],['table','x','x',null],['table','x','x',new Uint8Array([1])]])assert.throws(()=>snapshot({sql:{exec(){return {raw(){return [row][Symbol.iterator]();}};}}}),/type\/length/);
});
test('binary changes, false page metadata, wrong cursor and incomplete scripts fail',async()=>{
 for(const kind of ['byte','rows','cursor','short']){const f=structuredClone(fixture),s=storage();try{const vector=f.vectors[0];if(kind==='byte'){const p=vector.pages[0];p.reply=p.reply.slice(0,-2)+(p.reply.endsWith('00')?'01':'00');}else if(kind==='rows')vector.pages[0].rows++;else if(kind==='cursor')vector.pages[1].command=vector.pages[0].command;else vector.pages.pop();await assert.rejects(instance(s,f).run(0),/mismatch|metadata|cursor|incomplete/);}finally{s.db.close();}}
});
test('unexpected SQL, rollback and sync errors cannot become expected union failures',async()=>{
 for(const kind of ['sql','rollback','sync']){const s=storage();try{if(kind==='sql')s.failSql=sql=>sql==='DROP INDEX kv_dirent_current_name';if(kind==='rollback')s.rollbackError=new Error('unexpected rollback provenance');if(kind==='sync')s.sync=async()=>{throw new Error('unexpected sync denial');};await assert.rejects(instance(s).run(NAMES.indexOf('missing_foks')),/unexpected/);}finally{s.db.close();}}
});
test('one cumulative deadline prevents later pages instead of refreshing their budget',async()=>{
 const s=storage(),original=Date.now;let now=1000,queries=0;Date.now=()=>now;
 s.failSql=sql=>{if(sql===pages.descriptor.statements.find(s=>s.id===9).sql&&++queries===1)now+=15001;return false;};
 try{await assert.rejects(instance(s).run(0),/expired|deadline/);assert.equal(queries,1);}finally{Date.now=original;s.db.close();}
});
