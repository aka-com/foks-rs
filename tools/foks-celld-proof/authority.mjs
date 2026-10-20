// Local qualification only. The deployment supplies a trusted native schema;
// requests never supply DDL. Actual FOKS rows, constraints and triggers live here.
import { createHash } from "node:crypto";
import { Buffer } from "node:buffer";
import { MAX_EXACT_NODE_BYTES } from "./node-decoder.mjs";

export const MAX_BYTES = 8 * 1024 * 1024;
const MAX_ROWS = 10000;
const MAX_RECEIPTS = 10000;
const BASE = ["kv_nodes", "kv_dirents", "kv_dirent_heads", "kv_locks"];
// Trigger-owned counters that count trigger firings rather than describe the
// resulting rows. A replay that writes a row in two statements (kv_nodes
// below) fires more triggers than the native write did, so these values are
// left out of the state hash; apply checks that the revision advanced.
// No proof covers their values, so an output routed through the proof must
// not include them (the read-dependency revision is a read token's read
// coordinate). Must match UNHASHED_COUNTERS in foks-server-testkit's
// celld_proof.rs, which a Rust test compares with this line.
const UNHASHED_COUNTERS = [["metadata_read_dependency","revision"]];
const fail = () => { throw new Error("proof request rejected"); };
const canonical = value => {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === "object") return Object.fromEntries(Object.keys(value).sort().map(key=>[key,canonical(value[key])]));
  return value;
};
export const hash = value => createHash("sha256").update(JSON.stringify(canonical(value))).digest("hex");
const identifier = value => {
  if (typeof value !== "string" || !/^[a-z0-9_]{1,128}$/.test(value) || /^(?:_cf_|proof_)/.test(value)) fail();
  return `"${value}"`;
};
const fields = (value, names) => {
  if (!value || Array.isArray(value) || typeof value !== "object" ||
      Object.keys(value).sort().join(",") !== [...names].sort().join(",")) fail();
};
export function integer(value) {
  if (typeof value !== "string" || !/^(?:0|-?[1-9][0-9]*)$/.test(value) || value.length > 20) fail();
  const n = BigInt(value);
  if (n < -(1n << 63n) || n >= (1n << 63n)) fail();
  return value;
}
function base64(value) {
  if (typeof value !== "string" || value.length > MAX_BYTES) fail();
  const bytes = Buffer.from(value, "base64");
  if (bytes.toString("base64") !== value) fail();
  return bytes;
}
export function typed(value) {
  fields(value, value?.kind === "null" ? ["kind"] : ["kind", "value"]);
  switch (value.kind) {
    case "null": return {kind:"null"};
    case "integer": return {kind:"integer",value:integer(value.value)};
    case "blob": base64(value.value); return {kind:"blob",value:value.value};
    case "text": {
      const bytes = base64(value.value);
      new TextDecoder("utf-8", {fatal:true}).decode(bytes);
      return {kind:"text",value:value.value};
    }
    default: return fail();
  }
}
function bind(value) {
  switch (value.kind) {
    case "null": return null;
    case "integer": return integer(value.value);
    case "text": return new TextDecoder("utf-8", {fatal:true}).decode(base64(value.value));
    case "blob": return base64(value.value);
    default: return fail();
  }
}
const placeholder = value => value.kind === "integer" ? "CAST(? AS INTEGER)" : "?";
const token = value => {
  if (typeof value !== "string" || !/^[a-zA-Z0-9_-]{16,128}$/.test(value)) fail();
  return value;
};
const fingerprint = value => {
  if (typeof value !== "string" || !/^[a-f0-9]{64}$/.test(value)) fail();
  return value;
};

export async function boundedJson(request) {
  const header = request.headers.get("content-length");
  if (header !== null && (!/^[0-9]+$/.test(header) || Number(header) > MAX_BYTES)) fail();
  if (!request.body) fail();
  const reader = request.body.getReader();
  // A fixed byte budget also bounds metadata for arbitrarily fragmented input;
  // retaining one object per incoming chunk would defeat the byte-only cap.
  const bytes = new Uint8Array(MAX_BYTES);
  let length = 0;
  try {
    for (;;) {
      const {done,value} = await reader.read();
      if (done) break;
      if (value.byteLength > MAX_BYTES-length) fail();
      bytes.set(value,length);
      length += value.byteLength;
    }
  } finally {
    await reader.cancel();
    reader.releaseLock();
  }
  return JSON.parse(new TextDecoder("utf-8",{fatal:true}).decode(bytes.subarray(0,length)));
}

export class Authority {
  #deriveNode;
  constructor(storage, schema, deriveNode) {
    if (typeof deriveNode !== "function") fail();
    this.#deriveNode = deriveNode;
    this.storage = storage;
    fields(schema,["application_id","version","objects","tables"]);
    integer(schema.application_id); integer(schema.version);
    if (!Array.isArray(schema.tables) || schema.tables.length === 0 || schema.tables.length > 128 ||
        !Array.isArray(schema.objects) || schema.objects.length > 512 ||
        Buffer.byteLength(JSON.stringify(schema)) > MAX_BYTES) fail();
    let previous = "";
    for (const table of schema.tables) {
      fields(table,["name","columns"]); identifier(table.name);
      if (table.name <= previous || !Array.isArray(table.columns) || table.columns.length === 0 || table.columns.length > 50) fail();
      previous = table.name;
      const names = new Set();
      for (const column of table.columns) {
        fields(column,["name","kind","primary_key"]); identifier(column.name);
        if (names.has(column.name) || !["INTEGER","TEXT","BLOB"].includes(column.kind) ||
            !Number.isInteger(column.primary_key) || column.primary_key < 0 || column.primary_key > 64) fail();
        names.add(column.name);
      }
    }
    for (const object of schema.objects) {
      fields(object,["kind","name","table","sql"]);
      identifier(object.name); identifier(object.table);
      if (!["table","index","trigger"].includes(object.kind) || typeof object.sql !== "string" ||
          !object.sql.startsWith("CREATE ") || /\bVIRTUAL\b/i.test(object.sql)) fail();
    }
    this.schema = schema;
    this.schemaHash = hash(schema);
    // A listed table without its counter column is refused, as the native
    // side refuses it; a schema without the table has nothing to leave out.
    this.unhashed = new Map(UNHASHED_COUNTERS.flatMap(([table,column]) => {
      const found = schema.tables.find(t=>t.name===table);
      if (!found) return [];
      const index = found.columns.findIndex(c=>c.name===column);
      if (index < 0) fail();
      return [[table,index]];
    }));
    this.tables = new Map(schema.tables.map(t => [t.name,t]));
    // This must happen outside transactionSync: SQLite ignores a foreign_keys
    // change in an active transaction. Verify instead of assuming the default.
    this.sql("PRAGMA foreign_keys=ON");
    if (this.sql("PRAGMA foreign_keys")[0]?.foreign_keys !== 1) fail();
    this.sql("CREATE TABLE IF NOT EXISTS proof_meta (singleton INTEGER PRIMARY KEY CHECK(singleton=1), schema_hash TEXT NOT NULL, init_hash TEXT NOT NULL, host_id TEXT NOT NULL, epoch INTEGER NOT NULL CHECK(epoch>=0), owner TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision>=0), key_bundle TEXT NOT NULL) STRICT");
    this.sql("CREATE TABLE IF NOT EXISTS proof_receipts (attempt TEXT PRIMARY KEY, epoch INTEGER NOT NULL, owner TEXT NOT NULL, request_hash TEXT NOT NULL, revision INTEGER NOT NULL, state_hash TEXT NOT NULL, output_hash TEXT NOT NULL) STRICT");
  }
  sql(query,...bindings) { return [...this.storage.sql.exec(query,...bindings)]; }
  /** The hash of `snapshot` without the trigger-firing counters. */
  stateHash(snapshot) {
    return hash({...snapshot,tables:snapshot.tables.map(table => {
      const index = this.unhashed.get(table.name);
      return index === undefined ? table : {...table,rows:table.rows.map(row=>row.map((value,i)=>i===index ? {kind:"null"} : value))};
    })});
  }
  /** The trigger-firing counters, as integers, in table order. */
  counters() {
    return [...this.unhashed.keys()].map(table => {
      const column = identifier(this.tables.get(table).columns[this.unhashed.get(table)].name);
      return this.sql(`SELECT CAST(${column} AS TEXT) AS value FROM ${identifier(table)}`).map(row=>BigInt(integer(row.value)));
    });
  }
  meta() {
    const rows = this.sql("SELECT schema_hash,init_hash,host_id,CAST(epoch AS TEXT) AS epoch,owner,CAST(revision AS TEXT) AS revision FROM proof_meta WHERE singleton=1");
    if (rows.length !== 1 || rows[0].schema_hash !== this.schemaHash) fail();
    return rows[0];
  }
  owner(request) {
    integer(request.epoch); token(request.owner);
    const meta = this.meta();
    if (meta.epoch !== request.epoch || meta.owner !== request.owner) fail();
    return meta;
  }
  normalizeSnapshot(snapshot) {
    fields(snapshot,["schema_hash","tables"]);
    if (snapshot.schema_hash !== this.schemaHash || !Array.isArray(snapshot.tables) || snapshot.tables.length !== this.schema.tables.length) fail();
    let count = 0;
    const tables = snapshot.tables.map((data,index) => {
      fields(data,["name","rows"]);
      const table = this.schema.tables[index];
      if (data.name !== table.name || !Array.isArray(data.rows)) fail();
      count += data.rows.length;
      if (count > MAX_ROWS) fail();
      let previous = null;
      const rows = data.rows.map(row => {
        if (!Array.isArray(row) || row.length !== table.columns.length) fail();
        const normalized = row.map(typed);
        const encoded = JSON.stringify(normalized);
        if (previous !== null && previous >= encoded) fail();
        previous = encoded;
        return normalized;
      });
      return {name:table.name,rows};
    });
    const result = {schema_hash:this.schemaHash,tables};
    if (Buffer.byteLength(JSON.stringify(result)) > MAX_BYTES) fail();
    return result;
  }
  snapshot() {
    let rowsSeen = 0;
    let bytesSeen = 0;
    const tables = this.schema.tables.map(table => {
      const projection = table.columns.map((c,index) => {
        const name = identifier(c.name);
        // typeof plus CAST preserve SQLite i64 without a JavaScript double.
        return `typeof(${name}) AS "p${index}_type", CASE WHEN typeof(${name})='integer' THEN CAST(${name} AS TEXT) ELSE ${name} END AS "p${index}"`;
      }).join(",");
      const rows = [];
      for (const stored of this.storage.sql.exec(`SELECT ${projection} FROM ${identifier(table.name)}`)) {
        if (++rowsSeen > MAX_ROWS) fail();
        const row = table.columns.map((_column,index) => {
          const value = stored[`p${index}`];
          const kind = stored[`p${index}_type`];
          if (kind === "null") return {kind:"null"};
          if (kind === "integer") return {kind:"integer",value:integer(value)};
          if (kind !== "text" && kind !== "blob") fail();
          const length = kind === "text" ? Buffer.byteLength(value,"utf8") : value.byteLength;
          if (!Number.isSafeInteger(length) || length < 0) fail();
          bytesSeen += length * 2 + 64;
          if (bytesSeen > MAX_BYTES) fail();
          const bytes = kind === "text" ? Buffer.from(value,"utf8") : Buffer.from(value);
          return {kind,value:bytes.toString("base64")};
        });
        bytesSeen += row.length * 64;
        if (bytesSeen > MAX_BYTES) fail();
        rows.push(row);
      }
      rows.sort((a,b) => { const x=JSON.stringify(a),y=JSON.stringify(b); return x<y?-1:x>y?1:0; });
      return {name:table.name,rows};
    });
    return this.normalizeSnapshot({schema_hash:this.schemaHash,tables});
  }
  initialize(request) {
    fields(request,["snapshot","host_id","key_bundle"]);
    const snapshot = this.normalizeSnapshot(request.snapshot);
    // Keys are encrypted files plus manifest, encoded as a bounded opaque bundle.
    if (base64(request.key_bundle).length > 512*1024 || base64(request.host_id).length !== 33) fail();
    const initialization = hash(request);
    return this.storage.transactionSync(() => {
      const old = this.sql("SELECT init_hash FROM proof_meta WHERE singleton=1");
      if (old.length) { if (old[0].init_hash !== initialization) fail(); return this.meta(); }
      this.sql("PRAGMA defer_foreign_keys=ON");
      for (const object of this.schema.objects.filter(o=>o.kind==="table")) this.sql(object.sql);
      for (const table of snapshot.tables) for (const row of table.rows) {
        this.sql(`INSERT INTO ${identifier(table.name)} VALUES (${row.map(placeholder).join(",")})`,...row.map(bind));
      }
      for (const object of this.schema.objects.filter(o=>o.kind!=="table")) this.sql(object.sql);
      if (this.sql("PRAGMA foreign_key_check").length) fail();
      if (hash(this.snapshot()) !== hash(snapshot)) fail();
      const host = this.sql("SELECT host_id FROM host_metadata WHERE singleton=1");
      if (host.length !== 1 || Buffer.from(host[0].host_id).toString("base64") !== request.host_id) fail();
      this.sql("INSERT INTO proof_meta VALUES(1,?,?,?,0,'',0,?)",this.schemaHash,initialization,request.host_id,request.key_bundle);
      return this.meta();
    });
  }
  claim(request) {
    fields(request,["expected_epoch","owner"]);
    const expected = BigInt(integer(request.expected_epoch)); token(request.owner);
    if (expected < 0n || expected === (1n<<63n)-1n) fail();
    return this.storage.transactionSync(() => {
      const meta = this.meta();
      const next = (expected+1n).toString();
      if (meta.epoch === next && meta.owner === request.owner) return meta;
      if (meta.epoch !== request.expected_epoch) fail();
      this.sql("UPDATE proof_meta SET epoch=CAST(? AS INTEGER),owner=? WHERE singleton=1",next,request.owner);
      return this.meta();
    });
  }
  read(request) {
    fields(request,["epoch","owner"]);
    return this.storage.transactionSync(() => {
      const meta = this.owner(request);
      return {epoch:meta.epoch,revision:meta.revision,snapshot:this.snapshot(),key_bundle:this.sql("SELECT key_bundle FROM proof_meta WHERE singleton=1")[0].key_bundle};
    });
  }
  receipt(attempt) {
    token(attempt);
    return this.sql("SELECT CAST(epoch AS TEXT) AS epoch,owner,request_hash,CAST(revision AS TEXT) AS revision,state_hash,output_hash FROM proof_receipts WHERE attempt=?",attempt)[0];
  }
  apply(request) {
    fields(request,["epoch","owner","revision","attempt","output_hash","delta"]);
    integer(request.revision); token(request.attempt); fingerprint(request.output_hash);
    fields(request.delta,["before_hash","after_hash","changes"]);
    fingerprint(request.delta.before_hash); fingerprint(request.delta.after_hash);
    if (!Array.isArray(request.delta.changes) || request.delta.changes.length > MAX_ROWS) fail();
    const requestHash = hash(request);
    return this.storage.transactionSync(() => {
      const meta = this.owner(request);
      const receipt = this.receipt(request.attempt);
      if (receipt) { if (receipt.request_hash !== requestHash) fail(); return receipt; }
      if (meta.revision !== request.revision || this.stateHash(this.snapshot()) !== request.delta.before_hash) fail();
      const countersBefore = this.counters();
      if (this.sql("SELECT count(*) AS n FROM proof_receipts")[0].n >= MAX_RECEIPTS) fail();
      let previous = -1;
      const keys = new Set();
      const prepared = [];
      for (const change of request.delta.changes) {
        fields(change,["table","old","new"]);
        const order = BASE.indexOf(change.table);
        const table = this.tables.get(change.table);
        if (order < previous || order < 0 || !table || (!change.old && !change.new) || (!change.new && change.table !== "kv_locks")) fail();
        previous = order;
        if (change.table === "kv_nodes" && change.new) {
          const exactIndex = table.columns.findIndex(c=>c.name==="exact_node");
          const exact = change.new[exactIndex];
          // Check encoded size before typed()/base64() makes a decoded copy.
          if (exactIndex < 0 || exact?.kind !== "blob" || typeof exact.value !== "string" ||
              exact.value.length > Math.ceil(MAX_EXACT_NODE_BYTES/3)*4) fail();
          const decodedLength=exact.value.length/4*3-(exact.value.endsWith("==")?2:exact.value.endsWith("=")?1:0);
          if(exact.value.length%4!==0 || decodedLength<1 || decodedLength>MAX_EXACT_NODE_BYTES) fail();
        }
        const normalize = row => {
          if (row === null) return null;
          if (!Array.isArray(row) || row.length !== table.columns.length) fail();
          return row.map(typed);
        };
        const old = normalize(change.old), next = normalize(change.new);
        const key = row => table.columns.flatMap((c,i)=>c.primary_key ? [row[i]] : []);
        const values = key(next || old);
        if (!values.length || values.some(v=>v.kind==="null") || (old && next && JSON.stringify(key(old))!==JSON.stringify(key(next)))) fail();
        const encodedKey = `${change.table}:${JSON.stringify(values)}`;
        if (keys.has(encodedKey)) fail(); keys.add(encodedKey);
        let nodeLength = null;
        if (change.table === "kv_nodes" && next) {
          const exactIndex = table.columns.findIndex(c=>c.name==="exact_node");
          const lengthIndex = table.columns.findIndex(c=>c.name==="ciphertext_bytes");
          if (exactIndex < 0 || lengthIndex < 0 || next[exactIndex].kind !== "blob" || next[lengthIndex].kind !== "integer") fail();
          const exact = base64(next[exactIndex].value);
          if (!exact.length || exact.length > MAX_EXACT_NODE_BYTES) fail();
          const derived = integer(this.#deriveNode(exact));
          if (BigInt(derived) < 0n || next[lengthIndex].value !== derived) fail();
          nodeLength = {value:derived,index:lengthIndex,exactIndex};
        }
        prepared.push({change,table,old,next,nodeLength});
      }
      // Derive every candidate before the first SQL effect. The later stamp and
      // every trigger/counter/receipt remain in this same transaction.
      for (const {change,table,old,next,nodeLength} of prepared) {
        const name = identifier(change.table);
        const written = nodeLength ? next.map((value,i)=>i===nodeLength.index ? {kind:"null"}:value) : next;
        if (!old) {
          this.sql(`INSERT INTO ${name} VALUES (${written.map(placeholder).join(",")})`,...written.map(bind));
          if (this.sql("SELECT changes() AS n")[0].n !== 1) fail();
        } else {
          const where = table.columns.map((c,i)=>`${identifier(c.name)} IS ${placeholder(old[i])}`).join(" AND ");
          if (next) {
            const set = table.columns.map((c,i)=>`${identifier(c.name)}=${placeholder(written[i])}`).join(",");
            this.sql(`UPDATE ${name} SET ${set} WHERE ${where}`,...written.map(bind),...old.map(bind));
          } else this.sql(`DELETE FROM ${name} WHERE ${where}`,...old.map(bind));
          if (this.sql("SELECT changes() AS n")[0].n !== 1) fail();
        }
        if (nodeLength) {
          const uidIndex=table.columns.findIndex(c=>c.name==="uid"), idIndex=table.columns.findIndex(c=>c.name==="node_id");
          if (uidIndex<0 || idIndex<0) fail();
          this.sql("UPDATE kv_nodes SET ciphertext_bytes=CAST(? AS INTEGER) WHERE uid=? AND node_id=? AND exact_node=? AND ciphertext_bytes IS NULL",
            nodeLength.value,bind(next[uidIndex]),bind(next[idIndex]),bind(next[nodeLength.exactIndex]));
          if (this.sql("SELECT changes() AS n")[0].n !== 1) fail();
        }
      }
      if (this.sql("PRAGMA foreign_key_check").length || this.stateHash(this.snapshot()) !== request.delta.after_hash) fail();
      // A counter never moves back, and a write to a base table other than
      // kv_locks (which has no read-dependency trigger) advances it.
      const advances = request.delta.changes.some(change => change.table !== "kv_locks");
      const countersAfter = this.counters();
      if (countersAfter.some((rows,t) => rows.length !== countersBefore[t].length ||
          rows.some((value,i) => advances ? value <= countersBefore[t][i] : value < countersBefore[t][i]))) fail();
      const revision = (BigInt(meta.revision)+1n).toString(); integer(revision);
      this.sql("UPDATE proof_meta SET revision=CAST(? AS INTEGER) WHERE singleton=1",revision);
      this.sql("INSERT INTO proof_receipts VALUES(?,CAST(? AS INTEGER),?,?,CAST(? AS INTEGER),?,?)",request.attempt,meta.epoch,meta.owner,requestHash,revision,request.delta.after_hash,request.output_hash);
      return this.receipt(request.attempt);
    });
  }
  prove(request) {
    fields(request,["epoch","owner","revision","state_hash"]);
    integer(request.revision); fingerprint(request.state_hash);
    return this.storage.transactionSync(() => {
      const meta = this.owner(request);
      if (meta.revision !== request.revision || this.stateHash(this.snapshot()) !== request.state_hash) fail();
      return {epoch:meta.epoch,revision:meta.revision,state_hash:request.state_hash};
    });
  }
}
