// One-record decoder qualification. Fixture setup/assertions are not execution.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,statSync} from 'node:fs';
import {join,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
import {DatabaseSync} from 'node:sqlite';
import {Kernel} from './host.mjs';

const wasm=process.env.FOKS_DIRECTORY_WASM,root=process.env.FOKS_DIRECTORY_VECTORS;
if(!wasm||!root||statSync(wasm).size>32*1024*1024)throw new Error('reviewed directory artifacts required');
const fixture=JSON.parse(readFileSync(join(root,'directory-vectors.json'),'utf8'));
const artifact=JSON.parse(readFileSync(join(root,'page-catalog.json'),'utf8'));
assert.equal(fixture.vectors.length,8);
// Reuse the already reviewed static declaration parser; never grow memory to
// discover the limit or silently qualify an unbounded-memory Wasm artifact.
const here=dirname(fileURLToPath(import.meta.url));
execFileSync('python3',['-c','import pathlib,runpy,sys; runpy.run_path(sys.argv[1])["memory_limits"](pathlib.Path(sys.argv[2]).read_bytes())',join(here,'wasm_limits.py'),wasm]);
const module=new WebAssembly.Module(readFileSync(wasm));
const quote=s=>'"'+s.replaceAll('"','""')+'"';
function seed(exact){
  const db=new DatabaseSync(':memory:');db.exec('PRAGMA foreign_keys=ON;BEGIN;PRAGMA defer_foreign_keys=ON');
  for(const o of fixture.objects.filter(o=>o.kind==='table'))db.exec(o.sql);
  for(const t of fixture.before)for(const row of t.rows){
    const sql=`INSERT INTO ${quote(t.name)} VALUES(${row.map(v=>v!=='null'&&'integer' in v?'CAST(? AS INTEGER)':'?').join(',')})`;
    db.prepare(sql).run(...row.map(v=>v==='null'?null:'integer' in v?v.integer:'blob' in v?Uint8Array.from(v.blob):v.text));
  }
  for(const o of fixture.objects.filter(o=>o.kind!=='table'))db.exec(o.sql);
  db.prepare('UPDATE kv_directories SET exact_directory=? WHERE uid=? AND directory_id=?').run(exact,Uint8Array.from(fixture.uid),Uint8Array.from(fixture.parent));
  assert.equal(db.prepare('PRAGMA foreign_key_check').get(),undefined);db.exec('COMMIT');return db;
}
function storage(db){return {sql:{exec(sql,...params){
  const q=db.prepare(sql);q.setReadBigInts(true);const names=q.columns().map(c=>c.name);
  return {raw(){const rows=q.iterate(...params);return {next(){const row=rows.next();return row.done?row:{done:false,value:names.map(n=>row.value[n])};}};}};
}}};}

test('existing full decoder matches native key and errors through16MiB within fixed Wasm memory',()=>{
  for(const vector of fixture.vectors){
    assert.match(vector.file,/^[a-z0-9_]+\.bin$/);
    const filename=join(root,vector.file);assert.ok(statSync(filename).size<=16*1024*1024);
    const exact=readFileSync(filename),db=seed(exact),kernel=new Kernel(module,[],storage(db),artifact);
    const changes=db.prepare('SELECT total_changes() AS n').get().n;
    const before=kernel.instance.exports.memory.buffer.byteLength;
    const started=performance.now();db.exec('BEGIN');let result=null,error=null;
    try {result=kernel.runDirectory(new Uint8Array(Buffer.from(vector.command,'hex')),{deadline:Date.now()+15000});}
    catch(e){error=e;}
    if(vector.outcome==='ok'){
      assert.equal(error,null,vector.name);assert.equal(Buffer.from(result).toString('hex'),vector.reply,vector.name);
    }else{assert.equal(error?.code,vector.outcome,vector.name);assert.equal(result,null);}
    db.exec('ROLLBACK');
    const after=kernel.instance.exports.memory.buffer.byteLength;
    assert.ok(after<=64*1024*1024,vector.name);
    assert.equal(kernel.poisoned,false,vector.name);assert.equal(kernel.calls,vector.calls,vector.name);
    assert.equal(kernel.pageWorking.current,0);
    assert.equal(db.prepare('SELECT total_changes() AS n').get().n,changes);
    const stored=db.prepare('SELECT exact_directory FROM kv_directories WHERE uid=? AND directory_id=?').get(Uint8Array.from(fixture.uid),Uint8Array.from(fixture.parent));
    assert.deepEqual(stored.exact_directory,new Uint8Array(exact));
    console.log(JSON.stringify({name:vector.name,input_bytes:exact.length,wasm_before_bytes:before,wasm_peak_bytes:after,declared_host_page_peak_bytes:kernel.lastPageWorkingBytes,elapsed_ms:performance.now()-started,scope:'one-record native/Wasm decoder; excludes SQLite/JS memory and production deadlines'}));
    db.close();
  }
});
