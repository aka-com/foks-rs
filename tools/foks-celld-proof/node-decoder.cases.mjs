import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync,statSync} from "node:fs";
import {nodeDecoder,MAX_EXACT_NODE_BYTES} from "./node-decoder.mjs";

const wasmPath=process.env.FOKS_PROOF_NODE_DECODER_WASM;
const vectorPath=process.env.FOKS_PROOF_VECTORS;
if (!wasmPath || statSync(wasmPath).size>32*1024*1024 || !vectorPath || statSync(vectorPath).size>8*1024*1024) {
  throw new Error("set FOKS_PROOF_NODE_DECODER_WASM and FOKS_PROOF_VECTORS to reviewed native artifacts");
}
const wasmBytes=readFileSync(wasmPath);
const module=new WebAssembly.Module(wasmBytes);
const cases=JSON.parse(readFileSync(vectorPath,"utf8")).node_decoder_cases;
if (!Array.isArray(cases) || cases.length<4) throw new Error("missing native exact-box cases");

// Negative artifact fixture: redirect one export to another already compiled
// Rust function. No business decoder is implemented or replaced by JavaScript.
function aliasExport(target,source) {
  const read=(buffer,start)=>{
    let value=0,shift=0,offset=start;
    for (;;) {
      const byte=buffer[offset++];
      if(byte===undefined || shift>28)throw new Error("fixture LEB");
      value+=(byte&127)*2**shift;
      if(!(byte&128))return [value,offset];
      shift+=7;
    }
  };
  const leb=value=>{
    const bytes=[];
    do {const low=value%128;value=Math.floor(value/128);bytes.push(low|(value?128:0));}while(value);
    return Buffer.from(bytes);
  };
  const sections=[wasmBytes.subarray(0,8)];
  let found=false;
  for(let offset=8;offset<wasmBytes.length;) {
    const start=offset,id=wasmBytes[offset++];
    let size;[size,offset]=read(wasmBytes,offset);
    const end=offset+size;
    if(id!==7)sections.push(wasmBytes.subarray(start,end));
    else {
      let count;[count,offset]=read(wasmBytes,offset);
      const exports=[];
      for(let i=0;i<count;i++) {
        let length,index;[length,offset]=read(wasmBytes,offset);
        const name=wasmBytes.subarray(offset,offset+length);offset+=length;
        const kind=wasmBytes[offset++];[index,offset]=read(wasmBytes,offset);
        exports.push({name,kind,index});
      }
      const sourceEntry=exports.find(e=>e.name.toString()===source);
      const targetEntry=exports.find(e=>e.name.toString()===target);
      assert.equal(sourceEntry?.kind,0);assert.equal(targetEntry?.kind,0);
      targetEntry.index=sourceEntry.index;found=true;
      const body=Buffer.concat([leb(count),...exports.flatMap(e=>[leb(e.name.length),e.name,Buffer.from([e.kind]),leb(e.index)])]);
      sections.push(Buffer.from([7]),leb(body.length),body);
    }
    offset=end;
  }
  assert.equal(found,true);
  return new WebAssembly.Module(Buffer.concat(sections));
}

test("actual Wasm returns exact native lengths including legacy boxes above RPC write cap",()=>{
  const derive=nodeDecoder(module);
  for (const item of cases) assert.equal(derive(Buffer.from(item.exact,"base64")),item.length);
  assert.ok(cases.some(item=>BigInt(item.length)>2064n));
});

test("actual Wasm rejects corrupt and oversized input and remains usable after errors",()=>{
  const derive=nodeDecoder(module);
  for (const bytes of [new Uint8Array(),Uint8Array.of(0),Uint8Array.of(0x91,0),new Uint8Array(MAX_EXACT_NODE_BYTES+1)]) {
    assert.throws(()=>derive(bytes));
  }
  const item=cases[0];
  assert.equal(derive(Buffer.from(item.exact,"base64")),item.length);
});

test("rejected Wasm allocation clears prior request/output and pure export never imports SQL",()=>{
  let imports=0;
  const forbidden=()=>{imports++;throw new Error("unexpected import");};
  const {exports:e}=new WebAssembly.Instance(module,{foks:{sql:forbidden,domain:forbidden,sql_page:forbidden}});
  const exact=Buffer.from(cases[0].exact,"base64");
  const load=()=>{
    const pointer=e.small_box_input_alloc(exact.length);
    assert.notEqual(pointer,0);
    new Uint8Array(e.memory.buffer,pointer,exact.length).set(exact);
  };
  load();assert.equal(e.small_box_ciphertext_length(),0);
  assert.ok(e.output_length()>0);
  assert.equal(e.small_box_input_alloc(MAX_EXACT_NODE_BYTES+1),0);
  assert.equal(e.output_length(),0);
  assert.notEqual(e.small_box_ciphertext_length(),0);
  load();
  assert.equal(e.small_box_input_alloc(0),0);
  assert.notEqual(e.small_box_ciphertext_length(),0);
  load();assert.equal(e.small_box_ciphertext_length(),0);
  assert.equal(imports,0);
});

test("an actual Wasm trap or invalid output export cannot produce a derived count",()=>{
  const exact=Buffer.from(cases[0].exact,"base64");
  const trap=nodeDecoder(aliasExport("small_box_ciphertext_length","trap"));
  assert.throws(()=>trap(exact),WebAssembly.RuntimeError);
  // A second failure proves the wrapper released its in-progress guard.
  assert.throws(()=>trap(exact),WebAssembly.RuntimeError);
  const badOutput=nodeDecoder(aliasExport("output_pointer","small_box_input_alloc"));
  assert.throws(()=>badOutput(exact),/proof node derivation rejected/);
});
