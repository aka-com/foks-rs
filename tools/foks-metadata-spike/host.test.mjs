import test from 'node:test';
import assert from 'node:assert/strict';
import {bind,sameData,Kernel,KernelError} from './host.mjs';
const bareKernel=()=>Object.assign(Object.create(Kernel.prototype),{poisoned:false,exportDepth:0,lastWipe:null});

test('all signed integers use exact decimal text at the JS SQL boundary',()=>{
  for(const value of ['-9223372036854775808','9223372036854775807','9007199254740993','0'])assert.equal(bind({integer:value},'integer'),value);
  for(const value of ['9223372036854775808','-9223372036854775809','01','-0','1.0','1e3',9007199254740992])assert.throws(()=>bind({integer:value},'integer'));
});
test('blob types and lengths are validated before allocating typed copies',()=>{
  assert.deepEqual(bind({blob:[0,255,1]},'blob'),Uint8Array.of(0,255,1));
  assert.throws(()=>bind({blob:Array(4097).fill(0)},'blob'));
  assert.throws(()=>bind({blob:[-1]},'blob'));
  assert.throws(()=>bind({blob:[256]},'blob'));
  assert.throws(()=>bind({blob:[1.5]},'blob'));
  assert.equal(bind('null','nullable_blob'),null);
  assert.throws(()=>bind('null','blob'));
  assert.throws(()=>bind({integer:'1',blob:[]},'integer'));
});
test('fixture comparison ignores object key order but preserves all fields and exact values',()=>{
  const a=[{name:'table',columns:['a','b'],rows:[[{integer:'9007199254740993'},{blob:[1,2]}]]}];
  const b=[{columns:['a','b'],name:'table',rows:[[{integer:'9007199254740993'},{blob:[1,2]}]]}];
  assert.equal(sameData(a,b),true);
  for(const changed of [
    [{...b[0],rows:[[{integer:'9007199254740992'},{blob:[1,2]}]]}],
    [{...b[0],rows:[[{integer:'9007199254740993'},{blob:[2,1]}]]}],
    [{...b[0],columns:['b','a']}],
    [{...b[0],extra:1}],
    [],
  ])assert.equal(sameData(a,changed),false);
});

// These tests isolate the public nested-transaction contract. Actual Wasm/SQLite
// equivalence is tested by the retained celld vector suites, not by this mock.
function domainHarness({status=null,failure=null,rollbackError=null,calls=0}={}) {
  const kernel=bareKernel(), events=[];
  Object.assign(kernel,{running:true,authority:true,domainActive:false,calls,deadline:Date.now()+60000});
  kernel.range=()=>new TextEncoder().encode('{}');
  kernel.invoke=(_command,_authority,statusError)=>{kernel.admit();events.push('effect');if(failure)throw failure;if(status)throw statusError(status);return {values:[]};};
  kernel.storage={transactionSync(callback){events.push('begin');try {const result=callback();events.push('release');return result;}catch(error){events.push('rollback');throw rollbackError??error;}}};
  kernel.respond=result=>{events.push('response');kernel.result=result;return 0;};
  return {kernel,events};
}
test('nested domain rejects only its own allowlisted error after rollback',()=>{
  for(const code of ['conflict','permission','quota']) {
    const {kernel,events}=domainHarness({status:code});
    kernel.domain(0,2);
    assert.deepEqual(events,['begin','effect','rollback','response']);
    assert.deepEqual(kernel.result,{rejected:code});
    assert.equal(kernel.calls,4);
    assert.equal(kernel.domainActive,false);
  }
  for(const error of [new KernelError('conflict'),new KernelError('permission'),new KernelError('quota'),new KernelError('bounds'),new KernelError('expired'),new KernelError('invalid'),new Error('conflict'),new WebAssembly.RuntimeError('trap')]) {
    const {kernel,events}=domainHarness({failure:error});
    assert.throws(()=>kernel.domain(0,2),value=>value===error);
    assert.deepEqual(events,['begin','effect','rollback']);
    assert.equal(kernel.domainActive,false);
  }
  const rollbackError=new KernelError('permission');
  const {kernel,events}=domainHarness({status:'conflict',rollbackError});
  assert.throws(()=>kernel.domain(0,2),value=>value===rollbackError);
  assert.deepEqual(events,['begin','effect','rollback']);
});
test('domain shares original call budget including transaction controls',()=>{
  for(const rejected of [false,true]) {
    const cost=rejected?4:3;
    for(const extra of [0,1]) {
      const {kernel}=domainHarness({calls:2048-cost+extra,status:rejected?'conflict':null});
      if(extra)assert.throws(()=>kernel.domain(0,2),value=>value instanceof KernelError&&value.code==='bounds');
      else {kernel.domain(0,2);assert.equal(kernel.calls,2048);}
    }
  }
});
test('domain cannot reset deadline or enter recursively or outside authority',()=>{
  const {kernel,events}=domainHarness();
  kernel.deadline=0;assert.throws(()=>kernel.domain(0,2),value=>value instanceof KernelError&&value.code==='expired');assert.deepEqual(events,[]);
  kernel.deadline=Date.now()+60000;
  const calls=kernel.calls;
  assert.throws(()=>kernel.run('{}'),/outer entry guard/);assert.equal(kernel.calls,calls);
  kernel.invoke=()=>kernel.domain(0,2);
  assert.throws(()=>kernel.domain(0,2),/domain entry guard/);
  assert.equal(kernel.domainActive,false);
  kernel.authority=false;assert.throws(()=>kernel.domain(0,2),/domain entry guard/);
  kernel.running=false;kernel.invoke=()=>{throw new Error('must not decode');};
  assert.throws(()=>kernel.run('{}',{deadline:0}),value=>value instanceof KernelError&&value.code==='expired');
  assert.equal(kernel.calls,0);assert.equal(kernel.running,false);
});
test('domain copies input and nested output across reentrant memory growth',()=>{
  const kernel=bareKernel(), memory=new WebAssembly.Memory({initial:1,maximum:3});
  const encoder=new TextEncoder(), decoder=new TextDecoder(), command='{"operation":"probe"}';
  const reply={values:[{integer:'9007199254740993'}]}, replyBytes=encoder.encode(JSON.stringify(reply));
  Object.assign(kernel,{running:true,authority:true,domainActive:false,calls:0,deadline:Date.now()+60000});
  new Uint8Array(memory.buffer,128,command.length).set(encoder.encode(command));
  kernel.storage={transactionSync:callback=>callback()};
  kernel.instance={exports:{memory,input_alloc(){memory.grow(1);return 256;},run(){assert.equal(decoder.decode(new Uint8Array(memory.buffer,256,command.length)),command);new Uint8Array(memory.buffer,1024,replyBytes.length).set(replyBytes);return 0;},output_pointer:()=>1024,output_length:()=>replyBytes.length,response_alloc(){memory.grow(1);return 2048;}}};
  kernel.domain(128,command.length);
  const expected=JSON.stringify({success:reply});
  assert.equal(decoder.decode(new Uint8Array(memory.buffer,2048,expected.length)),expected);
  assert.equal(memory.buffer.byteLength,3*65536);
});
test('only nonzero export status invokes the domain rejection factory',()=>{
  const kernel=bareKernel(), memory=new WebAssembly.Memory({initial:1});
  const imported=new KernelError('conflict'), token={}, encoded=new TextEncoder().encode('"conflict"');
  let conversions=0;
  kernel.instance={exports:{memory,input_alloc:()=>128,run(){throw imported;},output_pointer:()=>1024,output_length:()=>encoded.length}};
  const factory=code=>{assert.equal(code,'conflict');conversions++;return token;};
  assert.throws(()=>kernel.invoke('{}',false,factory),value=>value===imported);
  assert.equal(conversions,0);
  assert.equal(kernel.instance,null);assert.equal(kernel.poisoned,true);
  assert.equal(kernel.lastWipe.complete,true);assert.ok(new Uint8Array(memory.buffer).every(v=>v===0));
  const fresh=bareKernel(), freshMemory=new WebAssembly.Memory({initial:1});
  new Uint8Array(freshMemory.buffer,1024,encoded.length).set(encoded);
  fresh.instance={exports:{memory:freshMemory,input_alloc:()=>128,run:()=>1,output_pointer:()=>1024,output_length:()=>encoded.length}};
  assert.throws(()=>fresh.invoke('{}',false,factory),value=>value===token);
  assert.equal(fresh.poisoned,false);
  assert.equal(conversions,1);
});

test('personal entry is distinct and never converts typed outer failures to domain rejections',()=>{
  const kernel=bareKernel(), memory=new WebAssembly.Memory({initial:1});
  const payload={kind:'stale_cache',exact_hex:'0102ff'}, bytes=new TextEncoder().encode(JSON.stringify(payload));
  let called=[];
  kernel.instance={exports:{memory,input_alloc:()=>128,run_personal(){called.push('personal');return 1;},read_personal(){called.push('point');return 1;},run_authority(){throw new Error('synthetic selector must not run');},output_pointer:()=>1024,output_length:()=>bytes.length}};
  new Uint8Array(memory.buffer,1024,bytes.length).set(bytes);
  for(const personal of ['authority','point'])assert.throws(()=>kernel.run('{}',{personal}),error=>!(error instanceof KernelError)&&sameData(error.payload,payload));
  assert.deepEqual(called,['personal','point']);
  assert.throws(()=>kernel.run('{}',{personal:'authority',authority:true}),/mixed entry selectors/);
  assert.throws(()=>kernel.run('{}',{personal:'controller'}),/personal entry selector/);
});
