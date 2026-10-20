import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,statSync} from 'node:fs';
import {DatabaseSync} from 'node:sqlite';
import {Kernel,KernelError,sameData} from './host.mjs';
import {activatePages,decodeParameters,PageWriter,PageWorkingBudget,validateSpec,HEADER,MAX_PAGE,MAX_PARAMETERS} from './page-codec.mjs';

const paths={wasm:process.env.FOKS_PAGE_WASM,vectors:process.env.FOKS_PAGE_VECTORS,catalog:process.env.FOKS_PAGE_CATALOG};
for(const [kind,path] of Object.entries(paths))if(!path||statSync(path).size>(kind==='catalog'?65536:64*1024*1024))throw new Error(`reviewed FOKS_PAGE_${kind.toUpperCase()} artifact required`);
const module=new WebAssembly.Module(readFileSync(paths.wasm));
const fixture=JSON.parse(readFileSync(paths.vectors,'utf8')),artifact=JSON.parse(readFileSync(paths.catalog,'utf8'));
assert.equal(fixture.vectors.length,37);
const quote=s=>'"'+s.replaceAll('"','""')+'"';
function seeded(vector){
  const db=new DatabaseSync(':memory:');db.exec('PRAGMA foreign_keys=ON;BEGIN;PRAGMA defer_foreign_keys=ON');
  for(const o of fixture.objects.filter(o=>o.kind==='table'))db.exec(o.sql);
  const malformedNode=vector.page===true&&['list_negative_node_type','list_zero_node_type','list_mismatched_positive_node_type'].includes(vector.name);
  if(malformedNode)db.exec('PRAGMA ignore_check_constraints=ON');
  try {
  for(const t of vector.before)for(const row of t.rows){
    const sql=`INSERT INTO ${quote(t.name)} VALUES(${row.map(v=>v!=='null'&&'integer' in v?'CAST(? AS INTEGER)':'?').join(',')})`;
    db.prepare(sql).run(...row.map(v=>v==='null'?null:'integer' in v?v.integer:'blob' in v?Uint8Array.from(v.blob):v.text));
  }
  } finally {if(malformedNode)db.exec('PRAGMA ignore_check_constraints=OFF');}
  assert.equal(db.prepare('PRAGMA ignore_check_constraints').get().ignore_check_constraints,0);
  for(const o of fixture.objects.filter(o=>o.kind!=='table'))db.exec(o.sql);
  db.exec('COMMIT');
  return db;
}
function storage(db){return {sql:{exec(sql,...params){
  const statement=db.prepare(sql);statement.setReadBigInts(true);
  const columns=statement.columns().map(c=>c.name);
  return {raw(){const rows=statement.iterate(...params);return {next(){const row=rows.next();return row.done?row:{done:false,value:columns.map(c=>row.value[c])};}};}};
}}};}
function snapshot(db,vector){return vector.before.map(t=>{
  const q=db.prepare(`SELECT * FROM ${quote(t.name)}`);q.setReadBigInts(true);
  const rows=Array.from(q.iterate(),row=>t.columns.map(c=>{
    const v=row[c];return v===null?'null':typeof v==='bigint'?{integer:v.toString()}:typeof v==='string'?{text:v}:{blob:Array.from(v)};
  }));rows.sort((a,b)=>JSON.stringify(a)<JSON.stringify(b)?-1:JSON.stringify(a)>JSON.stringify(b)?1:0);
  return {...t,rows};
});}
const command=v=>new Uint8Array(Buffer.from(v.command,'hex'));
const probe=fixture.vectors.find(v=>v.name==='type_probe_1_maximum');
const newKernel=(store)=>new Kernel(module,[],store,artifact);
const rejectsBounds=fn=>assert.throws(fn,e=>e instanceof KernelError&&e.code==='bounds');

test('actual Wasm/SQLite pages equal all native exact-byte vectors and retain complete state',()=>{
  for(const vector of fixture.vectors){
    const db=seeded(vector),kernel=newKernel(storage(db));
    db.exec('BEGIN');
    const actual=kernel.runPage(command(vector));
    assert.equal(Buffer.from(actual).toString('hex'),vector.reply,vector.name);
    assert.equal(kernel.calls,1);assert.ok(kernel.lastPageWorkingBytes>0&&kernel.lastPageWorkingBytes<=8*MAX_PAGE);
    assert.equal(kernel.pageWorking.current,0);db.exec('COMMIT');
    assert.equal(sameData(snapshot(db,vector),vector.after),true,vector.name);db.close();
  }
});

test('activation binds complete ABI/catalog and never falls back to old scalar hosts',()=>{
  const noSQL=()=>{throw new Error('not queried');};
  const instance=new WebAssembly.Instance(module,{foks:{sql:noSQL,domain:noSQL,sql_page:noSQL}});
  assert.throws(()=>new WebAssembly.Instance(module,{foks:{sql:noSQL,domain:noSQL}}),WebAssembly.LinkError);
  for(const mutate of [a=>a.descriptor.abi_version++,a=>a.descriptor.statements[0].sql+=' ',a=>a.descriptor.statements[0].columns[0].nullable=true,a=>a.sha256='0'.repeat(64)]){
    const changed=structuredClone(artifact);mutate(changed);assert.throws(()=>activatePages(instance,changed));
  }
  const noPages=new Kernel(module,[],{sql:{exec:noSQL}});assert.throws(()=>noPages.runPage(command(probe)),/activation/);
});

test('malformed inputs, rejected allocation and preexpired budgets cannot reuse an earlier page',()=>{
  const db=seeded(probe),kernel=newKernel(storage(db)),bytes=command(probe);
  assert.ok(kernel.runPage(bytes).length>HEADER);
  for(const bad of [new Uint8Array(),bytes.subarray(0,bytes.length-1),new Uint8Array(MAX_PARAMETERS+1)])rejectsBounds(()=>kernel.runPage(bad));
  const changed=bytes.slice();changed[20]=1;rejectsBounds(()=>kernel.runPage(changed));
  assert.throws(()=>kernel.runPage(bytes,{deadline:0}),e=>e.code==='expired');assert.equal(kernel.calls,0);
  const e=kernel.instance.exports;
  assert.equal(e.page_input_alloc(MAX_PARAMETERS+1),0);assert.equal(e.output_length(),0);
  assert.notEqual(e.run_page(),0);assert.equal(kernel.calls,0);
  assert.equal(e.page_response_alloc(MAX_PAGE+1),0);
  assert.equal(kernel.poisoned,false);
  assert.equal(Buffer.from(kernel.runPage(bytes)).toString('hex'),probe.reply);db.close();
});

test('host admission precedes SQL; escaping host errors poison the instance and release JS admission',()=>{
  const db=seeded(probe),base=storage(db);let queries=0;
  const denied=newKernel({sql:{exec(...args){queries++;return base.sql.exec(...args);}}});
  rejectsBounds(()=>denied.runPage(command(probe),{workingLimit:1}));assert.equal(queries,0);assert.equal(denied.pageWorking.current,0);
  assert.equal(denied.poisoned,true);
  assert.throws(()=>denied.runPage(command(probe)),/poisoned/);assert.throws(()=>denied.run('{}'),/poisoned/);
  for(const controls of [{failAfter:1},{trapAfter:1}]){
    const kernel=newKernel(base);
    db.exec('BEGIN');db.exec('CREATE TABLE rollback_marker(n INTEGER)');
    assert.throws(()=>kernel.runPage(command(probe),controls));db.exec('ROLLBACK');
    assert.equal(db.prepare("SELECT count(*) AS n FROM sqlite_schema WHERE name='rollback_marker'").get().n,0);
    assert.equal(kernel.running,false);assert.equal(kernel.pageWorking.current,0);assert.equal(kernel.poisoned,true);
    assert.throws(()=>kernel.runPage(command(probe)),/poisoned/);assert.throws(()=>kernel.run('{}'),/poisoned/);
    // Recovery creates a fresh instance; the original is never reset/reused.
    assert.equal(Buffer.from(newKernel(base).runPage(command(probe))).toString('hex'),probe.reply);
  }
  const injected=new KernelError('permission'),kernel=newKernel({sql:{exec(){throw injected;}}});
  assert.throws(()=>kernel.runPage(command(probe)),e=>e===injected);
  assert.equal(kernel.pageWorking.current,0);assert.equal(kernel.poisoned,true);db.close();
});

test('typed rows reject Number precision, wrong type, extra rows and recursive imports',()=>{
  const row=['0','9223372036854775807',new Uint8Array(),'é'.repeat(32),new Uint8Array([0,255])];
  const make=rows=>newKernel({sql:{exec(){return {raw:()=>rows[Symbol.iterator]()};}}});
  for(const change of [r=>r[1]=9223372036854775807,r=>r[2]='not bytes',r=>r[3]='é'.repeat(33),r=>r[3]='\uD800',r=>r.pop()]){
    const invalid=[...row];change(invalid);rejectsBounds(()=>make([invalid]).runPage(command(probe)));
  }
  rejectsBounds(()=>make(Array.from({length:65},()=>row)).runPage(command(probe)));
  const recursive=make([]);recursive.storage={sql:{exec(){recursive.runPage(command(probe));}}};
  assert.throws(()=>recursive.runPage(command(probe)),/entry guard/);assert.equal(recursive.pageWorking.current,0);
});

test('SQL cursor growth cannot detach the copied parameters or response destination',()=>{
  const db=seeded(probe),base=storage(db);let kernel;
  kernel=newKernel({sql:{exec(...args){kernel.instance.exports.memory.grow(1);return base.sql.exec(...args);}}});
  assert.equal(Buffer.from(kernel.runPage(command(probe))).toString('hex'),probe.reply);db.close();
});

test('binary envelope caps, UTF8 bytes, NULL and exact integer limits remain explicit',()=>{
  const base=structuredClone(artifact.descriptor.statements[0]);
  const huge={...base,columns:[{kind:'blob',maximum_bytes:MAX_PAGE-HEADER-5,nullable:false}],integer_ranges:[],limit_parameter:null};
  assert.equal(validateSpec(huge).rows,1);
  const writer=new PageWriter(huge,1);writer.push([new Uint8Array(MAX_PAGE-HEADER-5)]);assert.equal(writer.finish().length,MAX_PAGE);
  assert.throws(()=>new PageWriter({...huge,maximum_bytes:MAX_PAGE-1},1));
  const parsed=decodeParameters(command(probe),base);assert.equal(parsed.values[1],'9223372036854775807');
  assert.equal(parsed.values[3],'\uFEFF'+'é'.repeat(30)+'x');
  const budget=new PageWorkingBudget(10),release=budget.reserve(10);assert.throws(()=>budget.reserve(1));release();assert.equal(budget.current,0);assert.equal(budget.peak,10);
});


test('an allocation export trap poisons before request entry; a normal zero rejection does not',()=>{
  const db=seeded(probe),kernel=newKernel(storage(db));
  const instance=kernel.instance;
  kernel.instance={exports:{...instance.exports,page_input_alloc:()=>0}};
  rejectsBounds(()=>kernel.runPage(command(probe)));assert.equal(kernel.poisoned,false);
  kernel.instance=instance;
  assert.equal(Buffer.from(kernel.runPage(command(probe))).toString('hex'),probe.reply);
  const trap=new WebAssembly.RuntimeError('allocation trap fixture');
  kernel.instance={exports:{...instance.exports,page_input_alloc(){throw trap;}}};
  assert.throws(()=>kernel.runPage(command(probe)),error=>error===trap);
  assert.equal(kernel.poisoned,true);assert.equal(kernel.running,false);
  assert.throws(()=>kernel.runPage(command(probe)),/poisoned/);assert.throws(()=>kernel.run('{}'),/poisoned/);
  db.close();
});

// Delivery-only fault harness; exact expected bytes come from the immutable
// native vectors above. The SQLite parity test separately exercises actual SQL.
class PageDeliveryKernel extends Kernel {
  constructor(){super(module,[],{},artifact);this.mode='ok';this.vector=probe;}
  sqlPage(id,pointer,length){
    this.admit();
    assert.equal(id,new DataView(command(this.vector).buffer).getUint32(8,true));
    assert.equal(Buffer.from(this.range(pointer,length,MAX_PARAMETERS)).toString('hex'),this.vector.command);
    if(this.mode==='missing')return 0;
    if(this.mode==='zero'||this.mode==='oversize'){
      assert.equal(this.invokeExport('page_response_alloc',this.mode==='zero'?0:MAX_PAGE+1),0);return 0;
    }
    const bytes=new Uint8Array(Buffer.from(this.vector.reply,'hex'));
    const destination=this.invokeExport('page_response_alloc',bytes.length);
    assert.notEqual(destination,0);
    this.range(destination,bytes.length,MAX_PAGE).set(bytes);
    if(this.mode==='malformed')this.range(destination,bytes.length,MAX_PAGE)[0]=0;
    if(this.mode==='duplicate')assert.equal(this.invokeExport('page_response_alloc',24),0);
    if(this.mode==='wrong-scalar')assert.equal(this.invokeExport('response_alloc',1),0);
    if(this.mode==='setup')assert.equal(this.invokeExport('read_token_setup_alloc',48),0);
    if(this.mode==='getter')assert.equal(this.invokeExport('output_length'),0);
    if(this.mode==='reentry')assert.equal(this.invokeExport('run_page'),1);
    if(this.mode==='returned-error')return 1;
    if(this.mode==='throw')throw new KernelError('bounds');
    if(this.mode==='trap')this.invokeExport('trap');
    return 0;
  }
  deliver(vector=probe){this.vector=vector;return this.runPage(command(vector),{deadline:Date.now()+60000});}
}

test('page delivery transfers large-small-large exact bytes without a second response owner',()=>{
  const large=fixture.vectors.reduce((a,b)=>a.reply.length>b.reply.length?a:b);
  assert.ok(large.reply.length>probe.reply.length);
  const kernel=new PageDeliveryKernel();
  try{
    for(const vector of [large,probe,large,probe]){
      assert.equal(Buffer.from(kernel.deliver(vector)).toString('hex'),vector.reply);
      assert.equal(kernel.poisoned,false);
      assert.ok(kernel.instance.exports.memory.buffer.byteLength<=64*1024*1024);
    }
  }finally{kernel.poisonAndDiscard();}
});

test('ordinary page delivery failures cannot retain or publish a prior successful page',()=>{
  for(const mode of ['missing','zero','oversize','malformed','duplicate','wrong-scalar','setup','getter','reentry','returned-error']){
    const kernel=new PageDeliveryKernel();
    try{
      assert.equal(Buffer.from(kernel.deliver()).toString('hex'),probe.reply);
      kernel.mode=mode;
      const expected=['zero','oversize','malformed'].includes(mode)?'bounds':'storage';
      assert.throws(()=>kernel.deliver(),error=>error instanceof KernelError&&error.code===expected,mode);
      assert.equal(kernel.poisoned,false,mode);
      kernel.mode='ok';assert.equal(Buffer.from(kernel.deliver()).toString('hex'),probe.reply,mode);
    }finally{kernel.poisonAndDiscard();}
  }
});

test('page import failures and real callback traps wipe and discard before fresh recovery',()=>{
  for(const mode of ['throw','trap','allocator-trap']){
    const kernel=new PageDeliveryKernel(),memory=kernel.instance.exports.memory;
    if(mode==='allocator-trap'){
      const exports=kernel.instance.exports;
      kernel.instance={exports:{...exports,page_response_alloc:()=>exports.trap()}};
    }else kernel.mode=mode;
    assert.throws(()=>kernel.deliver(),error=>mode==='throw'?error instanceof KernelError&&error.code==='bounds':error instanceof WebAssembly.RuntimeError);
    assert.equal(kernel.poisoned,true);assert.equal(kernel.instance,null);
    assert.equal(kernel.lastWipe.complete,true);assert.ok(new Uint8Array(memory.buffer).every(v=>v===0));
    assert.throws(()=>kernel.deliver(),/poisoned/);
    const fresh=new PageDeliveryKernel();
    try{assert.equal(Buffer.from(fresh.deliver()).toString('hex'),probe.reply);}finally{fresh.poisonAndDiscard();}
  }
});
