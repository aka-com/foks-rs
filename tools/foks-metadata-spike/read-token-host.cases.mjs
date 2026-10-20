import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,statSync} from 'node:fs';
import {ReadTokenHost} from './read-token-host.mjs';
const wasm=process.env.FOKS_READ_TOKEN_WASM;
if(!wasm||statSync(wasm).size>32*1024*1024)throw new Error('reviewed FOKS_READ_TOKEN_WASM artifact required');
const module=new WebAssembly.Module(readFileSync(wasm));
const defaultWasm=process.env.FOKS_READ_TOKEN_DEFAULT_WASM;
if(!defaultWasm||statSync(defaultWasm).size>32*1024*1024)throw new Error('reviewed FOKS_READ_TOKEN_DEFAULT_WASM artifact required');
const defaultModule=new WebAssembly.Module(readFileSync(defaultWasm));
const source=new URL('../../crates/foks-metadata-core/src/read_token/',import.meta.url);
// Preserve exact native u64 JSON literals. Parsing and stringifying this binding
// in JavaScript would round its >2^53 namespace limit before reaching Rust.
const binding=readFileSync(new URL('binding.json',source),'utf8').trim()
  .replace('"domain":"foks-metadata-list-binding-v2",','')
  .replace('"route":"personal_list",','').replace('"count":"4096"','"requested_count":"4097"');
const golden=readFileSync(new URL('token.hex',source),'utf8').trim();
const bundle=Array(16).fill(0x33);
const entropy=()=>Uint8Array.from([...Array(32).fill(0x11),...Array(16).fill(0x22)]);
const command=(kind,fields)=>`{"kind":${JSON.stringify(kind)},"binding":${binding},${JSON.stringify(fields).slice(1)}`;
const issue=()=>command('issue',{role:'1',visibility:-7,generation:'9007199254740993',bundle,now:'1'});
const verify=(token,extra={})=>command('verify',{token:Array.from(token),bundle,ordinal:0,now:'1',...extra});
function host(){return new ReadTokenHost(module,[],{sql:{exec(){throw new Error('token codec must not query SQL');}}});}
function ready(){const h=host(),bytes=entropy();h.installHostEntropy(bytes);assert.ok(bytes.every(v=>v===0));return h;}
function poisonAssertions(h,memory){
  assert.equal(h.poisoned,true);assert.equal(h.tokenReady,false);assert.equal(h.instance,null);
  assert.ok(new Uint8Array(memory.buffer).every(v=>v===0));
  assert.equal(h.lastWipe.bytes,memory.buffer.byteLength);assert.equal(h.lastWipe.complete,true);
  for(const call of [()=>h.run('{}'),()=>h.runPage(new Uint8Array(24)),()=>h.runFixture(issue()),()=>h.clearToken(),()=>h.rotateRandom()])assert.throws(call,/poisoned/);
}
test('actual Wasm token matches independent native golden and validates exact binding',()=>{
  const h=ready(),token=h.runFixture(issue());assert.equal(Buffer.from(token).toString('hex'),golden);
  assert.deepEqual(h.runFixture(verify(token)),Uint8Array.of(0,0));
  assert.throws(()=>h.runFixture(verify(token,{now:'15000'})),/Expired/);
  assert.throws(()=>h.runFixture(verify(token,{ordinal:1})),/BundleLost/);
  assert.throws(()=>h.runFixture(verify(token).replace('"revision":"3"','"revision":"4"')),/BundleLost/);
  for(let i=0;i<180;i++){const bad=token.slice();bad[i]^=1;assert.throws(()=>h.runFixture(verify(bad)));}
  assert.equal(h.poisoned,false);assert.deepEqual(h.runFixture(verify(token)),Uint8Array.of(0,0));
});
test('setup buffers are wiped, invalid setup loses key, and another instance cannot resume',()=>{
  const h=host(),pointer=h.tokenExport('read_token_setup_alloc',48),memory=h.instance.exports.memory;
  new Uint8Array(memory.buffer,pointer,48).set(entropy());assert.equal(h.tokenExport('read_token_setup'),0);h.tokenReady=true;
  assert.ok(new Uint8Array(memory.buffer,pointer,48).every(v=>v===0));const token=h.runFixture(issue());
  const invalid=new Uint8Array(47).fill(19);assert.throws(()=>h.installHostEntropy(invalid),/shape/);assert.ok(invalid.every(v=>v===0));assert.equal(h.tokenReady,false);
  assert.throws(()=>h.runFixture(verify(token)),/lifecycle/);
  const fresh=host(),other=entropy();other[0]++;fresh.installHostEntropy(other);assert.throws(()=>fresh.runFixture(verify(token)),/BundleLost/);
  fresh.rotateRandom();assert.throws(()=>fresh.runFixture(verify(token)),/BundleLost/);
  assert.throws(()=>fresh.runFixture('{"kind":"setup","key":[1]}'),/InvalidBinding/);
});
test('fixture input allocation failure clears prior input and cannot replay issuer',()=>{
  const h=ready(),encoded=new TextEncoder().encode(issue());let ptr=h.tokenExport('read_token_fixture_alloc',encoded.length);
  new Uint8Array(h.instance.exports.memory.buffer,ptr,encoded.length).set(encoded);
  assert.equal(h.tokenExport('read_token_fixture_alloc',32769),0);
  assert.notEqual(h.tokenExport('run_read_token_fixture'),0);assert.equal(h.poisoned,false);
  assert.equal(h.runFixture(issue()).length,180);
});
test('actual trap wipes current linear memory after unwinding and refuses every entry',()=>{
  const h=ready(),memory=h.instance.exports.memory;h.runFixture(issue());
  assert.throws(()=>h.tokenExport('trap'),WebAssembly.RuntimeError);poisonAssertions(h,memory);
  const fresh=ready();assert.equal(fresh.runFixture(issue()).length,180);
});
test('setup trap wipes owned entropy and memory; no Rust cleanup is attempted',()=>{
  const h=ready(),memory=h.instance.exports.memory,exports=h.instance.exports,bytes=entropy();
  // Deliberate fixture alias enters an actual trapping Wasm export during setup.
  h.instance={exports:{...exports,read_token_setup_alloc:exports.trap}};
  assert.throws(()=>h.installHostEntropy(bytes),WebAssembly.RuntimeError);
  assert.ok(bytes.every(v=>v===0));poisonAssertions(h,memory);
});
test('bounded wipe cost is measured separately from token work without latency promises',t=>{
  for(const pages of [32,256,1024]){
    const h=ready(),memory=h.instance.exports.memory,current=memory.buffer.byteLength/65536;
    if(pages>current)memory.grow(pages-current);
    new Uint8Array(memory.buffer).fill(91); // No Rust export after overwriting heap.
    h.poisonAndDiscard();poisonAssertions(h,memory);
    t.diagnostic(JSON.stringify({scope:'explicit current linear-memory wipe only',...h.lastWipe}));
  }
});

test('ordinary artifact cannot select the fixture issuer',()=>{
  const names=WebAssembly.Module.exports(defaultModule).map(e=>e.name);
  assert.ok(names.includes('read_token_setup'));
  assert.equal(names.includes('run_read_token_fixture'),false);
  assert.equal(names.includes('read_token_fixture_alloc'),false);
});
test('actual Wasm continuation retries are identical and final tokens cannot advance',()=>{
  const h=ready(),initial=h.runFixture(issue());
  const advance=command('advance',{token:Array.from(initial),consumed:1,limit:15,last:{kind:'time',value:'9007199254740993',id:Array(16).fill(1)},now:'2'});
  const next=h.runFixture(advance);assert.deepEqual(next,h.runFixture(advance));
  assert.deepEqual(h.runFixture(verify(next,{ordinal:1,now:'3'})),Uint8Array.of(1,1));
  assert.throws(()=>h.runFixture(command('advance',{token:Array.from(next),consumed:1,limit:15,last:{kind:'time',value:'9007199254740993',id:Array(16).fill(2)},now:'3'})),/InvalidToken/);
});

test('entropy source failure cannot keep the previous authority alive',t=>{
  const h=ready(),token=h.runFixture(issue());
  t.mock.method(globalThis.crypto,'getRandomValues',()=>{throw new Error('injected entropy failure');});
  assert.throws(()=>h.rotateRandom(),/injected entropy/);
  assert.equal(h.tokenReady,false);assert.equal(h.poisoned,false);
  assert.throws(()=>h.runFixture(verify(token)),/lifecycle/);
  const rejected=new Uint8Array(48);assert.throws(()=>h.installHostEntropy(rejected),/setup rejected/);
  assert.equal(h.tokenReady,false);
});

// This selector exists only in the already feature-gated fixture export. It
// returns the digest, never a token and never proof against current SQL state.
test('strict binding wire decoder has native/Wasm canonical and rejection parity',()=>{
  const h=ready(),raw=readFileSync(new URL('binding.json',source),'utf8').trim();
  const decode=exact=>h.runFixture(JSON.stringify({kind:'decode_binding',exact}));
  const expected=readFileSync(new URL('binding.sha256',source),'utf8').trim();
  assert.equal(Buffer.from(decode(raw)).toString('hex'),expected);
  const replaceFirst=(input,before,after)=>{assert.ok(input.includes(before),`missing fixture field: ${before}`);return input.replace(before,after);};
  const cursor=value=>{const found=raw.match(/"cursor":\{[^}]+\}/);assert.ok(found);return replaceFirst(raw,found[0],`"cursor":${value}`);};
  for(const value of ['{"kind":"none"}',`{"kind":"mac","value":${JSON.stringify(Array(32).fill(7))}}`,'{"kind":"time","value":"9223372036854775807"}'])assert.equal(decode(cursor(value)).length,32);
  for(const [prefix,field] of [
    ['{','"count":"4096",'],['"fence":{','"epoch":"2",'],
    ['"identity":{','"host":[],'],['"activation":{','"schema":"58",'],
    ['"limits":{','"dirents":9000,'],['"dependency":{','"revision":"1",'],
    ['"peer":{','"exact_der":[1],'],['"cursor":{','"kind":"time",'],
    ['"cursor":{','"value":"1",'],
  ]){
    assert.throws(()=>decode(replaceFirst(raw,prefix,prefix+field)),/InvalidBinding/);
    assert.throws(()=>decode(replaceFirst(raw,prefix,prefix+'"unknown":0,')),/InvalidBinding/);
  }
  for(const [before,after] of [
    ['"count":"4096"','"count":"0"'],['"count":"4096"','"count":"4097"'],
    ['"count":"4096"','"count":"04096"'],['"count":"4096"','"count":4096'],
    ['"domain":"foks-metadata-list-binding-v2"','"domain":"other"'],
    ['"route":"personal_list"','"route":"other"'],
    ['"blob_bytes":16777216','"blob_bytes":4294967296'],
    ['"dirent_bytes":4096','"dirent_bytes":18446744073709551615'],
  ])assert.throws(()=>decode(replaceFirst(raw,before,after)),/InvalidBinding/);
  for(const value of ['{"kind":{"none":null}}','{"kind":["none"]}','{"kind":null}','{"kind":0}','{"kind":"none","value":null}','{"kind":"mac"}','{"kind":"time","value":null}','{"kind":"time","value":"-1"}','{"kind":"time","value":"01"}','{"kind":"time","value":"9223372036854775808"}','{"kind":"time","value":{"kind":"time","value":"1"}}'])assert.throws(()=>decode(cursor(value)),/InvalidBinding/);
  assert.equal(decode(replaceFirst(replaceFirst(raw,'"blob_bytes":16777216','"blob_bytes":4294967295'),'"namespace_bytes":9007199254740993','"namespace_bytes":18446744073709551615')).length,32);
  assert.throws(()=>decode(raw+'{}'),/InvalidBinding/);
  assert.equal(h.poisoned,false);assert.equal(Buffer.from(decode(raw)).toString('hex'),expected);
});
