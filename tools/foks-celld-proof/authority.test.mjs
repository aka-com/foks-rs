import test from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { readFileSync, statSync } from "node:fs";
import { Authority, MAX_BYTES, hash, integer, typed, boundedJson } from "./authority.mjs";

const path=process.env.FOKS_PROOF_VECTORS;
if (!path || statSync(path).size>MAX_BYTES) throw new Error("set FOKS_PROOF_VECTORS to native-generated vectors.json");
const vectors=JSON.parse(readFileSync(path,"utf8"));
const owner="native_owner_00000001";
const nextOwner="native_owner_00000002";
const outputHash="b".repeat(64);

function fixture() {
  const db=new DatabaseSync(":memory:");
  const storage={
    sql:{exec(sql,...bindings) { return db.prepare(sql).all(...bindings); }},
    transactionSync(operation) {
      db.exec("BEGIN IMMEDIATE");
      try { const value=operation(); db.exec("COMMIT"); return value; }
      catch(error) { db.exec("ROLLBACK"); throw error; }
    }
  };
  const authority=new Authority(storage,vectors.schema);
  authority.initialize({snapshot:vectors.before,host_id:vectors.host_id,key_bundle:"ZW5jcnlwdGVkLXRlc3Qtb25seQ=="});
  authority.claim({expected_epoch:"0",owner});
  const request={epoch:"1",owner,revision:"0",attempt:"mutation_attempt_0001",output_hash:outputHash,delta:vectors.delta};
  return {db,authority,request};
}

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
    assert.equal(hash(vectors.before),vectors.delta.before_hash);
    assert.equal(hash(vectors.after),vectors.delta.after_hash);
    assert.deepEqual(authority.read({epoch:"1",owner}).snapshot,vectors.before);
    const receipt=authority.apply(request);
    assert.equal(receipt.revision,"1");
    assert.deepEqual(authority.read({epoch:"1",owner}).snapshot,vectors.after);
    assert.equal(db.prepare("SELECT CAST(creation_time AS TEXT) AS value FROM kv_dirents").get().value,"9007199254740993");
    // Trigger counters are compared against native SQL, never replayed twice.
    assert.equal(db.prepare("SELECT accounted_objects AS n FROM kv_namespaces").get().n,4);
    assert.deepEqual(authority.apply(request),receipt);
    assert.equal(db.prepare("SELECT count(*) AS n FROM proof_receipts").get().n,1);
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
    assert.deepEqual(authority.read({epoch:"1",owner}).snapshot,vectors.after);
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
    assert.throws(()=>authority.prove({epoch:"1",owner,revision:"1",state_hash:hash(vectors.after)}));
    assert.deepEqual(authority.read({epoch:"2",owner:nextOwner}).snapshot,vectors.after);
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
