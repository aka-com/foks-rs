import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,statSync} from 'node:fs';
import {Kernel,KernelError} from './host.mjs';
const path=process.env.FOKS_SCALAR_WASM;
if(!path||statSync(path).size>32*1024*1024)throw new Error('reviewed FOKS_SCALAR_WASM artifact required');
const module=new WebAssembly.Module(readFileSync(path));
const encoder=new TextEncoder(),decoder=new TextDecoder('utf-8',{fatal:true});
const command=(large=false)=>JSON.stringify({operation:'probe_types',integer:{integer:'-9223372036854775808'},blob:Array(large?4096:1).fill(255),text:large?'é'.repeat(2048):'x'});
class ScalarKernel extends Kernel {
  constructor(mode='ok'){super(module,[],{});this.mode=mode;this.observed=[];}
  sql(id,pointer,length,write){
    this.admit();assert.equal(id,15);assert.equal(write,0);
    const text=decoder.decode(this.range(pointer,length,65536));
    const parameters=JSON.parse(text);this.observed.push(text);
    if(this.mode==='missing')return 0;
    if(this.mode==='oversize'){assert.equal(this.invokeExport('response_alloc',65537),0);return 0;}
    if(this.mode==='malformed'){return this.raw('[{"integer":"01"}]');}
    if(this.mode==='escaped'){return this.raw('[{"integer":"-9223372036854775808"},{"blob":[255]},"null",{"text":"\\u00e9\\ud83e\\udd80"}]');}
    this.raw(JSON.stringify(parameters));
    if(this.mode==='duplicate'){assert.equal(this.invokeExport('response_alloc',1),0);return 0;}
    if(this.mode==='reentry'){assert.equal(this.invokeExport('run'),1);return 0;}
    if(this.mode==='setup'){assert.equal(this.invokeExport('read_token_setup_alloc',48),0);return 0;}
    if(this.mode==='pure'){assert.equal(this.invokeExport('small_box_ciphertext_length'),1);return 0;}
    if(this.mode==='getter'){assert.equal(this.invokeExport('output_length'),0);return 0;}
    if(this.mode==='returned-error')return 1;
    if(this.mode==='throw')throw new KernelError('conflict');
    if(this.mode==='trap')this.invokeExport('trap');
    return 0;
  }
  raw(text){const bytes=encoder.encode(text),pointer=this.invokeExport('response_alloc',bytes.length);assert.notEqual(pointer,0);this.range(pointer,bytes.length,65536).set(bytes);return 0;}
}

test('actual Wasm scalar codec preserves exact existing JSON and large/small values',()=>{
  const kernel=new ScalarKernel();
  for(const large of [false,true,false,true,false]){
    const reply=kernel.run(command(large),{deadline:Date.now()+60000});
    assert.equal(reply.values[0].integer,'-9223372036854775808');
    assert.equal(reply.values[1].blob.length,large?4096:1);
    assert.equal(reply.values[2],'null');
    assert.equal(reply.values[3].text,large?'é'.repeat(2048):'x');
    assert.equal(kernel.poisoned,false);
    assert.equal(kernel.observed.at(-1),JSON.stringify(reply.values));
    assert.ok(kernel.instance.exports.memory.buffer.byteLength<=64*1024*1024);
  }
  kernel.mode='escaped';assert.equal(kernel.run(command()).values[3].text,'é🦀');
});

test('missing malformed refused duplicate and forbidden helper replies cannot publish success',()=>{
  for(const mode of ['missing','malformed','oversize','duplicate','reentry','setup','pure','getter','returned-error']){
    const kernel=new ScalarKernel(mode);
    assert.throws(()=>kernel.run(command()),error=>error instanceof KernelError&&error.code===(mode==='oversize'?'bounds':'storage'),mode);
    assert.equal(kernel.poisoned,false,mode); // normal Rust failure unwound scopes
    kernel.mode='ok';assert.equal(kernel.run(command()).values[3].text,'x',mode);
  }
});

test('throw or trap after response allocation retires the entire instance',()=>{
  for(const mode of ['throw','trap']){
    const kernel=new ScalarKernel(mode),memory=kernel.instance.exports.memory;
    assert.throws(()=>kernel.run(command()));
    assert.equal(kernel.poisoned,true);
    assert.equal(kernel.instance,null);
    assert.equal(kernel.lastWipe.complete,true);
    assert.ok(new Uint8Array(memory.buffer).every(v=>v===0));
    kernel.mode='ok';assert.throws(()=>kernel.run(command()),/poisoned/);
    assert.throws(()=>kernel.invokeExport('input_alloc',1),/poisoned/);
    assert.equal(new ScalarKernel().run(command()).values[3].text,'x');
  }
});

test('base host does not poison a normally returned allocation refusal caught by its import',()=>{
  const kernel=Object.create(Kernel.prototype);
  kernel.poisoned=false;kernel.exportDepth=0;
  kernel.instance={exports:{allocate:()=>0}};
  assert.equal(kernel.invokeExport('allocate'),0);
  assert.equal(kernel.poisoned,false);
  assert.equal(kernel.invokePageExport('allocate'),0);
});


test('constructor activation trap wipes and discards its partially initialized instance',()=>{
  // Fixed tiny fixture module: one-page memory containing a marker and a
  // page_abi_version export that traps. No production Wasm export is added.
  const name=value=>[value.length,...encoder.encode(value)];
  const section=(id,bytes)=>[id,bytes.length,...bytes];
  const bytes=new Uint8Array([
    0,97,115,109,1,0,0,0,
    ...section(1,[1,96,0,1,127]),
    ...section(3,[1,0]),
    ...section(5,[1,1,1,1]),
    ...section(7,[2,...name('memory'),2,0,...name('page_abi_version'),0,0]),
    ...section(10,[1,3,0,0,11]),
    ...section(11,[1,0,65,0,11,6,...encoder.encode('secret')]),
  ]);
  let retired,memory;
  class SetupKernel extends Kernel{
    poisonAndDiscard(){
      retired=this;memory=this.instance.exports.memory;
      assert.equal(decoder.decode(new Uint8Array(memory.buffer,0,6)),'secret');
      super.poisonAndDiscard();
    }
  }
  assert.throws(()=>new SetupKernel(new WebAssembly.Module(bytes),[],{},{}),WebAssembly.RuntimeError);
  assert.equal(retired.instance,null);
  assert.equal(retired.poisoned,true);
  assert.equal(retired.lastWipe.complete,true);
  assert.ok(new Uint8Array(memory.buffer).every(value=>value===0));
});
