import test from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { readFileSync, statSync } from "node:fs";
import { Authority, MAX_BYTES, integer, typed, boundedJson } from "./authority.mjs";
import {nodeDecoder} from "./node-decoder.mjs";

const path=process.env.FOKS_PROOF_VECTORS;
if (!path || statSync(path).size>MAX_BYTES) throw new Error("set FOKS_PROOF_VECTORS to native-generated vectors.json");
const vectors=JSON.parse(readFileSync(path,"utf8"));
const decoderPath=process.env.FOKS_PROOF_NODE_DECODER_WASM;
if (!decoderPath || statSync(decoderPath).size>32*1024*1024) throw new Error("set FOKS_PROOF_NODE_DECODER_WASM to the reviewed Rust Wasm artifact");
const decoderModule=new WebAssembly.Module(readFileSync(decoderPath));
const owner="native_owner_00000001";
const nextOwner="native_owner_00000002";
const outputHash="b".repeat(64);

function fixture(derive=nodeDecoder(decoderModule)) {
  const db=new DatabaseSync(":memory:");
  const statements=[];
  const storage={
    sql:{exec(sql,...bindings) { statements.push(sql); return db.prepare(sql).all(...bindings); }},
    transactionSync(operation) {
      db.exec("BEGIN IMMEDIATE");
      try { const value=operation(); db.exec("COMMIT"); return value; }
      catch(error) { db.exec("ROLLBACK"); throw error; }
    }
  };
  const authority=new Authority(storage,vectors.schema,derive);
  authority.initialize({snapshot:vectors.before,host_id:vectors.host_id,key_bundle:"ZW5jcnlwdGVkLXRlc3Qtb25seQ=="});
  authority.claim({expected_epoch:"0",owner});
  const request={epoch:"1",owner,revision:"0",attempt:"mutation_attempt_0001",output_hash:outputHash,delta:vectors.delta};
  statements.length=0;
  return {db,authority,request,statements};
}

test("actual Rust decoder rejects forged/NULL/corrupt/oversized node derivatives before effects",()=>{
  const columns=vectors.schema.tables.find(t=>t.name==="kv_nodes").columns;
  const length=columns.findIndex(c=>c.name==="ciphertext_bytes");
  const exact=columns.findIndex(c=>c.name==="exact_node");
  assert.ok(length>=0 && exact>=0);
  for (const mode of ["count","null","corrupt","oversized","later-invalid"]) {
    const {db,authority,request,statements}=fixture();
    try {
      const forged=structuredClone(request);
      let row=forged.delta.changes.find(c=>c.table==="kv_nodes").new;
      if(mode==="count") row[length]={kind:"integer",value:(BigInt(row[length].value)+1n).toString()};
      if(mode==="null") row[length]={kind:"null"};
      if(mode==="corrupt") row[exact]={kind:"blob",value:"AA=="};
      if(mode==="oversized") row[exact]={kind:"blob",value:Buffer.alloc(65537).toString("base64")};
      if(mode==="later-invalid") {
        row=structuredClone(row);
        const id=columns.findIndex(c=>c.name==="node_id");
        const bytes=Buffer.from(row[id].value,"base64");bytes[1]^=1;
        row[id]={kind:"blob",value:bytes.toString("base64")};row[length]={kind:"null"};
        forged.delta.changes.splice(1,0,{table:"kv_nodes",old:null,new:row});
      }
      assert.throws(()=>authority.apply(forged),mode);
      assert.equal(statements.some(s=>/^(INSERT|UPDATE|DELETE)\b/.test(s)),false,mode);
      assert.deepEqual(authority.read({epoch:"1",owner}).snapshot,vectors.before);
      assert.equal(authority.meta().revision,"0");
      assert.equal(db.prepare("SELECT count(*) AS n FROM proof_receipts").get().n,0);
    } finally {db.close();}
  }
});

test("decoder failure aborts the whole replay without a receipt",()=>{
  const {db,authority,request}=fixture(()=>{throw new WebAssembly.RuntimeError("injected decoder trap");});
  try {
    assert.throws(()=>authority.apply(request),/injected decoder trap/);
    assert.deepEqual(authority.read({epoch:"1",owner}).snapshot,vectors.before);
    assert.equal(authority.meta().revision,"0");
    assert.equal(db.prepare("SELECT count(*) AS n FROM proof_receipts").get().n,0);
  } finally {db.close();}
});

test("identical legacy NULL node is revalidated through exact bytes and trigger counters",()=>{
  const {db,authority,request}=fixture();
  try {
    authority.apply(request);
    const table=vectors.schema.tables.find(t=>t.name==="kv_nodes");
    const selected=vectors.after.tables.find(t=>t.name==="kv_nodes").rows[0];
    const nodeId=table.columns.findIndex(c=>c.name==="node_id");
    db.prepare("UPDATE kv_nodes SET ciphertext_bytes=NULL WHERE node_id=?").run(Buffer.from(selected[nodeId].value,"base64"));
    const before=authority.read({epoch:"1",owner}).snapshot;
    const index=table.columns.findIndex(c=>c.name==="ciphertext_bytes");
    const old=before.tables.find(t=>t.name==="kv_nodes").rows[0];
    const next=vectors.after.tables.find(t=>t.name==="kv_nodes").rows[0];
    assert.deepEqual(old[index],{kind:"null"});
    const repair={...request,revision:"1",attempt:"revalidation_attempt",delta:{before_hash:authority.stateHash(before),after_hash:authority.stateHash(vectors.after),changes:[{table:"kv_nodes",old,new:next}]}};
    authority.apply(repair);
    assert.equal(authority.stateHash(authority.read({epoch:"1",owner}).snapshot),authority.stateHash(vectors.after));
    assert.equal(authority.meta().revision,"2");
    assert.equal(db.prepare("SELECT usage_unvalidated_small_count AS n FROM kv_namespaces").get().n,0);
  } finally {db.close();}
});

test("typed values preserve i64 boundaries and reject lossy or noncanonical inputs",()=>{
  for (const value of ["-9223372036854775808","9223372036854775807","9007199254740993"]) assert.equal(integer(value),value);
  for (const value of ["9223372036854775808","-9223372036854775809","01","-0",9007199254740992]) assert.throws(()=>integer(value));
  assert.deepEqual(typed({kind:"blob",value:"AAEC/w=="}),{kind:"blob",value:"AAEC/w=="});
  assert.throws(()=>typed({kind:"text",value:"/w=="}));
  assert.throws(()=>typed({kind:"blob",value:"AAEC/w"}));
  assert.throws(()=>typed({kind:"null",value:null}));
});

test("native schema and exact FOKS rows round trip through authoritative SQLite",()=>{
  const {db,authority,request}=fixture();
  try {
    assert.equal(authority.stateHash(vectors.before),vectors.delta.before_hash);
    assert.equal(authority.stateHash(vectors.after),vectors.delta.after_hash);
    assert.deepEqual(authority.read({epoch:"1",owner}).snapshot,vectors.before);
    const receipt=authority.apply(request);
    assert.equal(receipt.revision,"1");
    assert.equal(authority.stateHash(authority.read({epoch:"1",owner}).snapshot),authority.stateHash(vectors.after));
    assert.equal(db.prepare("SELECT CAST(creation_time AS TEXT) AS value FROM kv_dirents").get().value,"9007199254740993");
    // Trigger counters are compared against native SQL, never replayed twice.
    assert.equal(db.prepare("SELECT accounted_objects AS n FROM kv_namespaces").get().n,5);
    assert.deepEqual(db.prepare("SELECT node_type FROM kv_nodes ORDER BY node_type").all().map(r=>r.node_type),[3,4]);
    assert.deepEqual(authority.apply(request),receipt);
    assert.equal(db.prepare("SELECT count(*) AS n FROM proof_receipts").get().n,1);
  } finally {db.close();}
});

test("current heads are rebuilt from native PutDirents and derived-only tampering is rejected",()=>{
  const {db,authority,request}=fixture();
  try {
    const projection=vectors.after.tables.find(t=>t.name==="kv_dirent_current");
    assert.equal(projection.rows.length,1);
    assert.ok(!request.delta.changes.some(c=>c.table==="kv_dirent_current"));
    const tampered=structuredClone(vectors.after);
    const table=tampered.tables.find(t=>t.name==="kv_dirent_current");
    const column=vectors.schema.tables.find(t=>t.name===table.name).columns.findIndex(c=>c.name==="creation_time");
    table.rows[0][column]={kind:"integer",value:"1"};
    const forged=structuredClone(request);
    forged.delta.after_hash=authority.stateHash(tampered);
    assert.throws(()=>authority.apply(forged));
    assert.deepEqual(authority.read({epoch:"1",owner}).snapshot,vectors.before);
    assert.equal(db.prepare("SELECT count(*) AS n FROM proof_receipts").get().n,0);
    const direct=structuredClone(request);
    direct.delta.changes.push({table:"kv_dirent_current",old:null,new:projection.rows[0]});
    assert.throws(()=>authority.apply(direct));
    assert.deepEqual(authority.read({epoch:"1",owner}).snapshot,vectors.before);
    authority.apply(request);
    assert.equal(authority.stateHash(authority.read({epoch:"1",owner}).snapshot),authority.stateHash(vectors.after));
    const derivedOnly={...request,revision:"1",attempt:"derived_only_attempt",delta:{before_hash:authority.stateHash(vectors.after),after_hash:authority.stateHash(tampered),changes:[]}};
    assert.throws(()=>authority.apply(derivedOnly));
    assert.equal(authority.stateHash(authority.read({epoch:"1",owner}).snapshot),authority.stateHash(vectors.after));
    assert.equal(db.prepare("SELECT count(*) AS n FROM proof_receipts").get().n,1);
  } finally {db.close();}
});

test("all semantic Usage counters remain derived and cannot change through a hash-only delta",()=>{
  const {db,authority,request}=fixture();
  try {
    authority.apply(request);
    const schema=vectors.schema.tables.find(t=>t.name==="kv_namespaces");
    for(const name of ["usage_small_count","usage_small_bytes","usage_large_count","usage_large_bytes","usage_large_chunks","usage_unvalidated_small_count"]) {
      const forged=structuredClone(vectors.after);
      const table=forged.tables.find(t=>t.name==="kv_namespaces");
      const index=schema.columns.findIndex(c=>c.name===name);
      assert.ok(index>=0);
      table.rows[0][index]={kind:"integer",value:(BigInt(table.rows[0][index].value)+1n).toString()};
      const delta={before_hash:authority.stateHash(vectors.after),after_hash:authority.stateHash(forged),changes:[]};
      assert.throws(()=>authority.apply({...request,revision:"1",attempt:"usage_forgery_attempt",delta}),name);
      assert.equal(authority.stateHash(authority.read({epoch:"1",owner}).snapshot),authority.stateHash(vectors.after));
      assert.equal(authority.meta().revision,"1");
      assert.equal(db.prepare("SELECT count(*) AS n FROM proof_receipts").get().n,1);
    }
  } finally {db.close();}
});

test("failure after dirent insertion rolls back node, dirent, head, counters and receipt",()=>{
  const {db,authority,request}=fixture();
  try {
    db.exec("CREATE TRIGGER proof_inject_failure BEFORE INSERT ON kv_dirent_heads BEGIN SELECT RAISE(ABORT,'injected'); END");
    assert.throws(()=>authority.apply(request));
    assert.deepEqual(authority.read({epoch:"1",owner}).snapshot,vectors.before);
    assert.equal(db.prepare("SELECT count(*) AS n FROM proof_receipts").get().n,0);
    db.exec("DROP TRIGGER proof_inject_failure");
    authority.apply(request);
    assert.equal(authority.stateHash(authority.read({epoch:"1",owner}).snapshot),authority.stateHash(vectors.after));
  } finally {db.close();}
});

test("stale owner and revision, conflicting retry and forged derived state fail closed",()=>{
  const {db,authority,request}=fixture();
  try {
    const forged=structuredClone(request); forged.delta.after_hash="0".repeat(64);
    assert.throws(()=>authority.apply(forged));
    assert.deepEqual(authority.read({epoch:"1",owner}).snapshot,vectors.before);
    authority.apply(request);
    assert.throws(()=>authority.apply({...request,attempt:"mutation_attempt_0002"}));
    assert.throws(()=>authority.apply({...request,output_hash:"c".repeat(64)}));
    authority.claim({expected_epoch:"1",owner:nextOwner});
    assert.throws(()=>authority.apply(request));
    assert.throws(()=>authority.prove({epoch:"1",owner,revision:"1",state_hash:authority.stateHash(vectors.after)}));
    assert.equal(authority.stateHash(authority.read({epoch:"2",owner:nextOwner}).snapshot),authority.stateHash(vectors.after));
    assert.equal(authority.claim({expected_epoch:"1",owner:nextOwner}).epoch,"2");
  } finally {db.close();}
});

test("real schema constraints and bridge row checks reject partial invalid mutation",()=>{
  const {db,authority,request}=fixture();
  try {
    const bad=structuredClone(request);
    const change=bad.delta.changes.find(c=>c.table==="kv_dirents");
    // Missing parent directory; foreign keys must remain active at commit.
    change.new[1]={kind:"blob",value:Buffer.alloc(16,7).toString("base64")};
    assert.throws(()=>authority.apply(bad));
    assert.deepEqual(authority.read({epoch:"1",owner}).snapshot,vectors.before);
    const unknown=structuredClone(request); unknown.delta.changes[0].table="users";
    assert.throws(()=>authority.apply(unknown));
    const duplicate=structuredClone(request); duplicate.delta.changes.splice(1,0,duplicate.delta.changes[0]);
    assert.throws(()=>authority.apply(duplicate));
  } finally {db.close();}
});

test("body limits reject streaming overflow even without content length",async()=>{
  const stream=new ReadableStream({start(controller){controller.enqueue(new Uint8Array(MAX_BYTES));controller.enqueue(new Uint8Array(1));controller.close();}});
  await assert.rejects(boundedJson(new Request("http://proof.invalid/",{method:"POST",body:stream,duplex:"half"})));
});
