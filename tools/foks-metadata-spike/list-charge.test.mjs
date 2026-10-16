// Explicit native/Wasm fixture qualification; no celld or serving-route claim.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,statSync,writeFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {join,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
import {DatabaseSync} from 'node:sqlite';
import {ListChargeHost,validateChargeModule,MAX_CHARGE_MEMORY} from './list-charge-host.mjs';
import {KernelError} from './host.mjs';

const root=process.env.FOKS_LIST_CHARGE_VECTORS,wasm=process.env.FOKS_LIST_CHARGE_WASM,ordinary=process.env.FOKS_LIST_CHARGE_DEFAULT_WASM;
if(!root||!wasm||!ordinary)throw new Error('explicit list charge artifacts required');
const read=(path,max)=>{assert.ok(statSync(path).size<=max,path);return readFileSync(path);};
const manifestBytes=read(join(root,'vectors.json'),64*1024*1024),manifest=JSON.parse(manifestBytes);
const scalarBytes=read(join(root,'catalog.json'),1024*1024),scalar=JSON.parse(scalarBytes);
const pageBytes=read(join(root,'page-catalog.json'),1024*1024),page=JSON.parse(pageBytes);
const wasmBytes=read(wasm,32*1024*1024),defaultBytes=read(ordinary,32*1024*1024);
const module=new WebAssembly.Module(wasmBytes),defaultModule=new WebAssembly.Module(defaultBytes);
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const canonical=value=>JSON.stringify(normalize(value));
function normalize(value){return Array.isArray(value)?value.map(normalize):value!==null&&typeof value==='object'?Object.fromEntries(Object.keys(value).sort().map(k=>[k,normalize(value[k])])):value;}
const quote=s=>'"'+s.replaceAll('"','""')+'"';
const here=dirname(fileURLToPath(import.meta.url));
const records=[];
// Admission precedes every instance construction, even if a later test fails.
validateChargeModule(module,true);validateChargeModule(defaultModule,false);
for(const path of [wasm,ordinary])execFileSync('python3',['-c','import pathlib,runpy,sys;runpy.run_path(sys.argv[1])["memory_limits"](pathlib.Path(sys.argv[2]).read_bytes())',join(here,'wasm_limits.py'),path]);
assert.deepEqual(read(wasm,32*1024*1024),wasmBytes);assert.deepEqual(read(ordinary,32*1024*1024),defaultBytes);
assert.equal(manifest.version,1);assert.equal(manifest.schema_version,59);
assert.ok(Array.isArray(manifest.fixtures)&&manifest.fixtures.length>0&&manifest.fixtures.length<=64);
const fixtureNames=manifest.fixtures.map(f=>f.name);
assert.equal(new Set(fixtureNames).size,fixtureNames.length);
for(const name of ['small','maximum_page','late_malformed','large_parent_cycles','distinct_roles','role_generation_edges'])assert.ok(fixtureNames.includes(name),name);
for(const seq of ['0','127','128','255','256','65535','65536','4294967295','4294967296','18446744073709551615'])for(const delta of [-1,0,1])assert.ok(fixtureNames.includes(`frame_${seq}_${delta}`));

function asset(value){
  assert.deepEqual(Object.keys(value).sort(),['length','path','sha256']);
  assert.match(value.sha256,/^[0-9a-f]{64}$/);assert.equal(value.path,`${value.sha256}.bin`);
  assert.ok(Number.isSafeInteger(value.length)&&value.length>1024&&value.length<=16*1024*1024);
  const bytes=read(join(root,value.path),16*1024*1024);
  assert.equal(bytes.length,value.length);assert.equal(hash(bytes),value.sha256);return bytes;
}
function value(cell){
  if(cell==='null')return null;
  assert.equal(Object.keys(cell).length,1);
  if('asset' in cell)return asset(cell.asset);
  if('blob' in cell){assert.ok(cell.blob.length<=1024&&cell.blob.every(n=>Number.isInteger(n)&&n>=0&&n<=255));return Uint8Array.from(cell.blob);}
  if('text' in cell){assert.equal(typeof cell.text,'string');return cell.text;}
  assert.match(cell.integer,/^(0|-?[1-9][0-9]*)$/);const n=BigInt(cell.integer);assert.ok(n>=-(1n<<63n)&&n<(1n<<63n));return n;
}
function seed(fixture){
  const db=new DatabaseSync(':memory:');
  try{
    db.exec('PRAGMA foreign_keys=ON;BEGIN;PRAGMA defer_foreign_keys=ON');
    for(const o of manifest.objects.filter(o=>o.kind==='table'))db.exec(o.sql);
    let zeroGenerationSeeds=0;
    for(const table of fixture.before){
      const q=db.prepare(`INSERT INTO ${quote(table.name)} (${table.columns.map(quote).join(',')}) VALUES(${table.columns.map(()=>'?').join(',')})`);
      for(const row of table.rows){
        assert.equal(row.length,table.columns.length);
        const field=name=>row[table.columns.indexOf(name)]?.integer;
        const bypass=fixture.name==='role_generation_edges'&&table.name==='shared_keys'&&field('role_type')==='1'&&field('visibility')==='1'&&field('generation')==='0';
        // Fixed corruption fixture only; all other rows retain normal CHECKs.
        if(bypass){assert.equal(++zeroGenerationSeeds,1);db.exec('PRAGMA ignore_check_constraints=ON');}
        try{q.run(...row.map(value));}
        finally{if(bypass){db.exec('PRAGMA ignore_check_constraints=OFF');assert.equal(db.prepare('PRAGMA ignore_check_constraints').get().ignore_check_constraints,0);}}
      }
    }
    assert.equal(zeroGenerationSeeds,fixture.name==='role_generation_edges'?1:0);
    assert.equal(db.prepare('PRAGMA ignore_check_constraints').get().ignore_check_constraints,0);
    for(const o of manifest.objects.filter(o=>o.kind!=='table'))db.exec(o.sql);
    assert.equal(db.prepare('PRAGMA foreign_key_check').get(),undefined);db.exec('COMMIT');
    assertState(db,fixture);return db;
  }catch(error){db.close();throw error;}
}
function snapshotCell(cell){
  if(cell===null)return 'null';
  if(typeof cell==='bigint')return {integer:String(cell)};
  if(typeof cell==='string')return {text:cell};
  assert.ok(cell instanceof Uint8Array);
  if(cell.length<=1024)return {blob:Array.from(cell)};
  const sha256=hash(cell);return {asset:{path:`${sha256}.bin`,sha256,length:cell.length}};
}
function assertState(db,fixture){
  const ddl=db.prepare("SELECT type,sql FROM sqlite_schema WHERE substr(name,1,7)!='sqlite_' COLLATE NOCASE AND sql IS NOT NULL").all().map(r=>({kind:r.type,sql:r.sql}));
  assert.deepEqual(ddl.map(canonical).sort(),manifest.objects.map(canonical).sort());
  const names=db.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND substr(name,1,7)!='sqlite_' COLLATE NOCASE ORDER BY name").all().map(r=>r.name);
  assert.deepEqual(names,fixture.before.map(t=>t.name));
  for(const table of fixture.before){
    const q=db.prepare(`SELECT * FROM ${quote(table.name)}`);q.setReadBigInts(true);
    const columns=q.columns().map(c=>c.name);assert.deepEqual(columns,table.columns);
    const rows=[];for(const row of q.iterate())rows.push(canonical(columns.map(c=>snapshotCell(row[c]))));
    assert.deepEqual(rows.sort(),table.rows.map(canonical).sort(),table.name);
  }
}
function storage(db){return {sql:{exec(sql,...parameters){
  const q=db.prepare(sql);q.setReadBigInts(true);const names=q.columns().map(c=>c.name);
  return {raw(){const rows=q.iterate(...parameters);return {next(){const row=rows.next();return row.done?row:{done:false,value:names.map(n=>row.value[n])};}};}};
}}};}
function number(text){assert.match(text,/^(0|[1-9][0-9]*)$/);const n=BigInt(text);assert.ok(n<=((1n<<64n)-1n));return n;}
function checkReport(report,host){
  assert.equal(report.version,1);assert.equal(report.target.pointer_bits,32);assert.equal(report.target.sql_value_bytes,null);
  assert.deepEqual(report.target,host.chargeLayout.target);
  const phases=['authentication','entry','parent','parent_authorization','page','decode','roles','append','growth','finish','frame','other'];
  assert.deepEqual(report.phases.map(p=>p.name),phases);
  for(const key of ['attempts','admitted_bytes','refused_calls'])assert.equal(report.phases.reduce((n,p)=>n+number(p[key]),0n),number(report.total[key]));
  assert.deepEqual(report.origins.map(o=>o.origin),['snowpack','rpc_response']);
  for(const key of ['attempts','admitted_bytes','refused_calls'])assert.equal(report.origins.reduce((n,o)=>n+number(o[key]),0n),number(report.total[key]));
  assert.equal(report.prefix.length,16);
  for(const [index,event] of report.prefix.entries()){
    if(BigInt(index)>=number(report.total.attempts)){assert.equal(event,null);continue;}
    assert.equal(number(event.ordinal),BigInt(index+1));assert.ok(phases.includes(event.phase));
    assert.ok(['snowpack','rpc_response'].includes(event.origin));assert.equal(typeof event.control,'boolean');assert.equal(typeof event.accepted,'boolean');number(event.bytes);
    if(event.origin==='snowpack')assert.equal(event.control,false);
  }
  const refused=number(report.total.refused_calls),first=number(report.first_refusal);
  assert.equal(first===0n,refused===0n);
  if(refused){assert.equal(report.outcome,'trace_refused');assert.ok(first<=number(report.total.attempts));}
  const bounds=host.chargeLayout.bounds;
  const bound=key=>{assert.ok(Number.isSafeInteger(bounds[key])&&bounds[key]>=0);return BigInt(bounds[key]);};
  const admitted=phase=>number(report.phases.find(p=>p.name===phase).admitted_bytes),pages=number(report.page_scopes);
  assert.ok(number(report.maximum_page_scratch)<=number(report.maximum_page_charge));
  assert.ok(number(report.maximum_page_scratch)<=bound('full_page_scratch'));
  assert.ok(admitted('page')<=pages*bound('page_transport'));
  assert.ok(admitted('roles')<=pages*bound('page_roles'));
  assert.ok(admitted('decode')<=4096n*bound('row_decode'));
  assert.ok(admitted('append')<=4096n*bound('row_append'));
  assert.ok(admitted('growth')<=bound('retained_growth'));assert.ok(admitted('finish')<=bound('finish'));
  if(report.outcome==='ok'){assert.match(report.list_sha256,/^[0-9a-f]{64}$/);assert.ok(number(report.response_bytes)>0n);}
  else{assert.equal(report.list_sha256,null);assert.equal(report.frame_sha256,null);assert.equal(report.response_bytes,'0');assert.equal(report.frame_bytes,'0');}
  if(report.frame_sha256!==null){assert.match(report.frame_sha256,/^[0-9a-f]{64}$/);assert.ok(number(report.frame_bytes)>0n);}
  // A refused Rust page-parameter allocation may occur after Counted observes
  // query_page but before the host import. Successful calls reach both layers.
  assert.ok(number(report.sql_calls)>=BigInt(host.calls));
  if(report.outcome==='ok')assert.equal(number(report.sql_calls),BigInt(host.calls));
  assert.ok(number(report.memory_pages)<=1024n);assert.ok(host.instance.exports.memory.buffer.byteLength<=MAX_CHARGE_MEMORY);
  assert.equal(host.pageWorking.current,0);assert.equal(host.poisoned,false);
}
function call(db,host,input){
  db.exec('BEGIN');try{return host.runCharge(JSON.stringify(input));}finally{db.exec('ROLLBACK');}
}

test('immutable feature/default artifacts expose exactly the reviewed functions and fixed memory',()=>{
  validateChargeModule(module,true);validateChargeModule(defaultModule,false);
  assert.throws(()=>validateChargeModule(defaultModule,true));assert.throws(()=>validateChargeModule(module,false));
});

for(const fixture of manifest.fixtures)test(`same-instance native/Wasm trace parity: ${fixture.name}`,()=>{
  const db=seed(fixture),host=new ListChargeHost(module,scalar,storage(db),page);
  try{
    const layout=host.layout();assert.equal(layout.version,1);assert.equal(layout.target.pointer_bits,32);assert.equal(layout.bounds.complete,false);
    const changes=db.prepare('SELECT total_changes() AS n').get().n;
    for(const vector of fixture.cases){
      const before=host.instance.exports.memory.buffer.byteLength,start=performance.now();let report,error;
      try{report=call(db,host,vector.input);checkReport(report,host);}catch(e){error=e;}
      records.push({name:vector.name,native:vector.expected,wasm:report??null,error:error?.message??null,wasm_before_bytes:before,wasm_after_bytes:host.instance?.exports.memory.buffer.byteLength??null,wipe:host.lastWipe,host_page_declared_peak:host.lastPageWorkingBytes,elapsed_ms:performance.now()-start});
      assert.equal(error,undefined,vector.name);
      for(const key of ['outcome','response_bytes','list_sha256','frame_bytes','frame_sha256'])assert.deepEqual(report[key],vector.expected[key],`${vector.name}:${key}`);
      // Target layouts/charge sites differ; counts are reported independently,
      // never coerced to native values to manufacture a memory proof.
      if(vector.input.fail_at==='0')assert.equal(report.sql_calls,vector.expected.sql_calls);
      assert.equal(db.prepare('SELECT total_changes() AS n').get().n,changes);
      assertState(db,fixture);
    }
  }finally{host.poisonAndDiscard();db.close();}
});

test('every observed small-call refusal and malformed input leaves same instance reusable',()=>{
  const fixture=manifest.fixtures.find(f=>f.name==='small'),db=seed(fixture),host=new ListChargeHost(module,scalar,storage(db),page);
  try{
    host.layout();
    // Out-of-import normal refusal is not an exception or a poisoning event.
    assert.equal(host.instance.exports.page_response_alloc(0),0);assert.equal(host.poisoned,false);
    const input=fixture.cases.find(v=>v.input.count==='1').input,baseline=call(db,host,input);checkReport(baseline,host);
    const attempts=number(baseline.total.attempts);assert.ok(attempts>0n&&attempts<=10000n);
    for(let site=1n;site<=attempts;site++){
      const report=call(db,host,{...input,fail_at:String(site)});checkReport(report,host);
      assert.equal(report.outcome,'trace_refused');assert.equal(report.first_refusal,String(site));
    }
    assert.throws(()=>host.runCharge('{'),/invalid|syntax|expected/);
    assert.equal(host.poisoned,false);
    const again=call(db,host,input);checkReport(again,host);assert.equal(again.list_sha256,baseline.list_sha256);assertState(db,fixture);
  }finally{host.poisonAndDiscard();db.close();}
});

test('maximum-page transport, retained growth and final-output refusal preserve reusable failure',()=>{
  const fixture=manifest.fixtures.find(f=>f.name==='maximum_page'),db=seed(fixture),host=new ListChargeHost(module,scalar,storage(db),page);
  try{
    host.layout();
    const input={...fixture.cases.find(v=>v.input.count==='15'&&v.input.load_small&&v.input.fail_at==='0').input,frame:false};
    const baseline=call(db,host,input);checkReport(baseline,host);assert.equal(baseline.outcome,'ok');
    const attempts=number(baseline.total.attempts);assert.ok(attempts>0n&&attempts<=10000n);
    const seen=new Set();
    const refuse=site=>{
      const report=call(db,host,{...input,fail_at:String(site)});checkReport(report,host);
      assert.equal(report.outcome,'trace_refused');assert.equal(report.first_refusal,String(site));
      const refused=report.phases.filter(p=>number(p.refused_calls)>0n);assert.equal(refused.length,1);
      assert.equal(refused[0].first_refusal,String(site));seen.add(refused[0].name);
    };
    // Discover actual sites rather than freezing target/layout-dependent indices.
    // Stop once both page allocation and retained encoder growth were refused.
    for(let site=1n;site<=attempts&&(!seen.has('page')||!seen.has('growth'));site++)refuse(site);
    refuse(attempts); // No framing: the final observed admission is finish output.
    for(const phase of ['page','growth','finish'])assert.ok(seen.has(phase),phase);
    const again=call(db,host,input);checkReport(again,host);assert.equal(again.list_sha256,baseline.list_sha256);assertState(db,fixture);
  }finally{host.poisonAndDiscard();db.close();}
});

test('import failure and Wasm trap discard the actual instance and fresh construction recovers',()=>{
  const fixture=manifest.fixtures.find(f=>f.name==='small'),db=seed(fixture),input=fixture.cases[0].input;
  try{
    for(const controls of [{failAfter:1},{trapAfter:1}]){
      const host=new ListChargeHost(module,scalar,storage(db),page),memory=host.instance.exports.memory;
      db.exec('BEGIN');try{assert.throws(()=>host.runCharge(JSON.stringify(input),controls));}finally{db.exec('ROLLBACK');}
      assert.equal(host.instance,null);assert.equal(host.poisoned,true);assert.equal(host.lastWipe.complete,true);
      assert.ok(new Uint8Array(memory.buffer).every(b=>b===0));assert.throws(()=>host.runCharge(JSON.stringify(input)),/poisoned/);assert.throws(()=>host.layout(),/poisoned/);
      const replacement=new ListChargeHost(module,scalar,storage(db),page);
      try{replacement.layout();checkReport(call(db,replacement,input),replacement);}finally{replacement.poisonAndDiscard();}
      assertState(db,fixture);
    }
  }finally{db.close();}
});

test('same-code imported bounds failure is never mistaken for the private allocator refusal',()=>{
  const fixture=manifest.fixtures.find(f=>f.name==='small'),db=seed(fixture);
  const backing=storage(db),failure=new KernelError('bounds');
  const broken={sql:{exec(sql,...parameters){
    // Scalar authentication still succeeds, then the actual page SQL import
    // throws a matching-code host failure before any allocator return.
    if(sql.includes('kv_dirent_current'))throw failure;
    return backing.sql.exec(sql,...parameters);
  }}};
  const host=new ListChargeHost(module,scalar,broken,page);
  try{
    const input=fixture.cases.find(v=>v.input.count==='1').input;
    db.exec('BEGIN');try{assert.throws(()=>host.runCharge(JSON.stringify(input)),error=>error===failure);}finally{db.exec('ROLLBACK');}
    assert.equal(host.poisoned,true);assert.equal(host.instance,null);assert.equal(host.lastWipe.complete,true);assertState(db,fixture);
  }finally{host.poisonAndDiscard();db.close();}
});

test('scalar same-code import failure and allocator trap cannot impersonate ordinary refusal',()=>{
  const fixture=manifest.fixtures.find(f=>f.name==='small');
  for(const mode of ['import','allocator']){
    const db=seed(fixture),failure=new KernelError('bounds');
    const backing=mode==='import'?{sql:{exec(){throw failure;}}}:storage(db);
    const host=new ListChargeHost(module,scalar,backing,page),memory=host.instance.exports.memory;
    if(mode==='allocator'){
      const exports=host.instance.exports;
      exports.response_alloc=()=>exports.trap();
    }
    try{
      const input=fixture.cases.find(v=>v.input.count==='1').input;
      db.exec('BEGIN');try{
        assert.throws(()=>host.runCharge(JSON.stringify(input)),error=>mode==='import'?error===failure:error instanceof WebAssembly.RuntimeError);
      }finally{db.exec('ROLLBACK');}
      assert.equal(host.poisoned,true);assert.equal(host.instance,null);assert.equal(host.lastWipe.complete,true);
      assert.ok(new Uint8Array(memory.buffer).every(value=>value===0));assertState(db,fixture);
    }finally{host.poisonAndDiscard();db.close();}
  }
});

test('ignored scalar phase violation produces only failure diagnostics and permits normal reuse',()=>{
  const fixture=manifest.fixtures.find(f=>f.name==='small'),db=seed(fixture);
  class ViolatingHost extends ListChargeHost{
    inject=true;
    sql(...args){
      const status=super.sql(...args);
      if(this.inject){
        this.inject=false;
        assert.equal(this.invokeExport('small_box_ciphertext_length'),1);
      }
      return status;
    }
  }
  const host=new ViolatingHost(module,scalar,storage(db),page);
  try{
    host.layout();
    const input=fixture.cases.find(v=>v.input.count==='1').input;
    const report=call(db,host,input);
    assert.equal(report.outcome,'storage');
    assert.equal(report.response_bytes,'0');assert.equal(report.frame_bytes,'0');
    assert.equal(report.list_sha256,null);assert.equal(report.frame_sha256,null);
    assert.equal(host.poisoned,false);
    const again=call(db,host,input);checkReport(again,host);assert.equal(again.outcome,'ok');assertState(db,fixture);
  }finally{host.poisonAndDiscard();db.close();}
});

test('a trap during the allocator callback still discards the instance',()=>{
  const fixture=manifest.fixtures.find(f=>f.name==='small'),db=seed(fixture),host=new ListChargeHost(module,scalar,storage(db),page);
  const exports=host.instance.exports;
  // Test-only callback replacement invokes the artifact's real trap export;
  // this is an exception, not a normal allocation-status refusal.
  exports.page_response_alloc=()=>exports.trap();
  try{
    const input=fixture.cases.find(v=>v.input.count==='1').input;
    db.exec('BEGIN');try{assert.throws(()=>host.runCharge(JSON.stringify(input)),WebAssembly.RuntimeError);}finally{db.exec('ROLLBACK');}
    assert.equal(host.poisoned,true);assert.equal(host.instance,null);assert.equal(host.lastWipe.complete,true);assertState(db,fixture);
  }finally{host.poisonAndDiscard();db.close();}
});

test.after(()=>{
  const output=process.env.FOKS_LIST_CHARGE_REPORT;
  const result={scope:'qualification cumulative charge observations, not a complete allocation allowance or celld qualification',artifacts:{wasm:hash(wasmBytes),default_wasm:hash(defaultBytes),manifest:hash(manifestBytes),scalar:hash(scalarBytes),page:hash(pageBytes)},records};
  if(output)writeFileSync(output,JSON.stringify(result,null,2)+'\n',{flag:'wx',mode:0o600});
  console.log(JSON.stringify({trace_records:records.length,scope:result.scope}));
});
