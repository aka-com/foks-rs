// Immutable qualification transport/assertions, never a production request API.
import {sameData,KernelError} from './host.mjs';
export const MAX_DIRECTORY=16*1024*1024,SLICE=512*1024;
const quote=s=>'"'+s.replaceAll('"','""')+'"';
const bytes=v=>v instanceof ArrayBuffer?new Uint8Array(v):v;
const hex=v=>Array.from(v,b=>b.toString(16).padStart(2,'0')).join('');
const equal=(a,b)=>a.length===b.length&&a.every((v,i)=>v===b[i]);
export function authorize(request,token,fixture){
  if(request.method!=='POST')return {status:405};
  if(request.headers.get('authorization')!==`Bearer ${token}`)return {status:403};
  const path=new URL(request.url).pathname;
  if(!/^\/[0-7]$/.test(path)||!fixture.vectors[Number(path.slice(1))])return {status:404};
  return {index:Number(path.slice(1))};
}
export async function readBody(body,vector){
  const length=vector.length;
  if(!Number.isSafeInteger(length)||length<1||length>MAX_DIRECTORY||!/^[0-9a-f]{64}$/.test(vector.sha256)||!body)throw new Error('fixture manifest/body');
  const result=new Uint8Array(length),reader=body.getReader();let offset=0,complete=false;
  try{
    for(;;){const next=await reader.read();if(next.done)break;
      if(!(next.value instanceof Uint8Array)||next.value.length>length-offset)throw new Error('fixture extra body');
      result.set(next.value,offset);offset+=next.value.length;
    }
    if(offset!==length)throw new Error('fixture short body');
    if(hex(new Uint8Array(await crypto.subtle.digest('SHA-256',result)))!==vector.sha256)throw new Error('fixture body digest');
    complete=true;return result;
  }finally{if(!complete){result.fill(0);try{await reader.cancel();}catch{}}reader.releaseLock();}
}
function selected(fixture){
  const table=fixture.before.find(t=>t.name==='kv_directories');if(!table)throw new Error('fixture directory table');
  const uid=table.columns.indexOf('uid'),id=table.columns.indexOf('directory_id'),version=table.columns.indexOf('version'),payload=table.columns.indexOf('exact_directory');
  if([uid,id,version,payload].some(i=>i<0))throw new Error('fixture directory columns');
  const rows=table.rows.filter(r=>sameData(r[uid],{blob:fixture.uid})&&sameData(r[id],{blob:fixture.parent}));
  if(rows.length!==1||!rows[0][version]?.integer?.match(/^[1-9][0-9]*$/))throw new Error('fixture directory identity');
  return {table,uid,id,version,payload,number:rows[0][version].integer};
}
const bind=v=>v==='null'?null:'integer' in v?v.integer:'text' in v?v.text:'blob' in v&&v.blob.length<=65536?Uint8Array.from(v.blob):(()=>{throw new Error('fixture seed cell');})();
const identity=(fixture)=>{const s=selected(fixture);return [Uint8Array.from(fixture.uid),Uint8Array.from(fixture.parent),s.number];};
export function seed(storage,fixture,exact){
  storage.sql.exec('PRAGMA foreign_keys=ON');
  storage.transactionSync(()=>{
    storage.sql.exec('PRAGMA defer_foreign_keys=ON');
    for(const o of fixture.objects.filter(o=>o.kind==='table'))storage.sql.exec(o.sql);
    for(const t of fixture.before)for(const row of t.rows){
      const placeholders=row.map(v=>v!=='null'&&'integer' in v?'CAST(? AS INTEGER)':'?');
      storage.sql.exec(`INSERT INTO ${quote(t.name)} VALUES(${placeholders.join(',')})`,...row.map(bind));
    }
    for(const o of fixture.objects.filter(o=>o.kind!=='table'))storage.sql.exec(o.sql);
    try{storage.sql.exec('UPDATE kv_directories SET exact_directory=? WHERE uid=? AND directory_id=? AND version=CAST(? AS INTEGER)',exact,...identity(fixture));}
    catch(error){throw new Error('fixture directory payload bind: '+String(error.message),{cause:error});}
    if(!storage.sql.exec('PRAGMA foreign_key_check').raw().next().done)throw new Error('fixture foreign key');
    if(!Number.isSafeInteger(fixture.schema_version)||fixture.schema_version<1)throw new Error('fixture schema version');
    // The immutable native manifest supplies the schema label. celld's public
    // SQL authorizer forbids user_version writes; no kernel query uses it here.
  });
}
function rawOne(storage,sql,...params){const cursor=storage.sql.exec(sql,...params).raw(),first=cursor.next();if(first.done||!cursor.next().done)throw new Error('fixture point cardinality');return first.value;}
export function changes(storage){return rawOne(storage,'SELECT CAST(total_changes() AS TEXT)')[0];}
export function assertState(storage,fixture,exact,platformSchema=[]){
  const s=selected(fixture),params=identity(fixture);
  const [kind,length]=rawOne(storage,'SELECT typeof(exact_directory),CAST(length(CAST(exact_directory AS BLOB)) AS TEXT) FROM kv_directories WHERE uid=? AND directory_id=? AND version=CAST(? AS INTEGER)',...params);
  if(kind!=='blob'||length!==String(exact.length))throw new Error('fixture directory shape changed');
  let compared=0;
  for(let offset=0;offset<=exact.length;offset+=SLICE){
    const [part]=rawOne(storage,'SELECT substr(exact_directory,CAST(? AS INTEGER)+1,CAST(? AS INTEGER)) FROM kv_directories WHERE uid=? AND directory_id=? AND version=CAST(? AS INTEGER)',String(offset),String(SLICE),...params);
    const value=bytes(part),expected=exact.subarray(offset,Math.min(offset+SLICE,exact.length));
    if(!(value instanceof Uint8Array)||!equal(value,expected))throw new Error('fixture directory bytes changed');
    compared+=value.length;
  }
  // The final iteration above is empty only for an exact slice multiple.
  const [end]=rawOne(storage,'SELECT substr(exact_directory,CAST(? AS INTEGER)+1,1) FROM kv_directories WHERE uid=? AND directory_id=? AND version=CAST(? AS INTEGER)',String(exact.length),...params);
  if(!(bytes(end) instanceof Uint8Array)||bytes(end).length!==0||compared!==exact.length)throw new Error('fixture directory comparison incomplete');
  // Seeding changes only the selected exact bytes and their schema58 byte charge.
  // Keep the expected charge independent of observed SQL state; never bless a
  // post-seed snapshot as its own oracle. Other counters remain exact baseline.
  const old=s.table.rows.find(r=>sameData(r[s.uid],{blob:fixture.uid})&&sameData(r[s.id],{blob:fixture.parent}))[s.payload];
  if(!old?.blob)throw new Error('fixture original directory bytes');
  const expected=fixture.before.map(t=>({name:t.name,columns:t.columns,rows:t.rows.map(r=>{
    if(t.name===s.table.name&&sameData(r[s.uid],{blob:fixture.uid})&&sameData(r[s.id],{blob:fixture.parent})&&r[s.version]?.integer===s.number)return r.map((v,i)=>i===s.payload?{directory_payload:true}:v);
    if(t.name==='kv_namespaces'&&sameData(r[t.columns.indexOf('namespace_id')],{blob:fixture.uid}))return r.map((v,i)=>t.columns[i]==='accounted_bytes'?{integer:String(BigInt(v.integer)+BigInt(exact.length)-BigInt(old.blob.length))}:v);
    return r;
  })}));
  const observed=fixture.before.map(t=>{
    const expressions=t.columns.flatMap(name=>{const c=quote(name);return [`typeof(${c})`,`CASE WHEN typeof(${c})='integer' THEN CAST(${c} AS TEXT) WHEN length(CAST(${c} AS BLOB))<=65536 THEN ${c} ELSE NULL END`];});
    const rows=[];
    for(const row of storage.sql.exec(`SELECT ${expressions.join(',')} FROM ${quote(t.name)}`).raw()){
      if(rows.length>=10000)throw new Error('fixture snapshot row bound');
      const isTarget=t.name===s.table.name&&equal(bytes(row[s.uid*2+1])??[],fixture.uid)&&equal(bytes(row[s.id*2+1])??[],fixture.parent)&&row[s.version*2+1]===s.number;
      rows.push(t.columns.map((_,i)=>{
        const kind=row[i*2],value=row[i*2+1];
        if(isTarget&&i===s.payload){if(kind!=='blob')throw new Error('fixture directory type');return {directory_payload:true};}
        if(kind==='null')return 'null';if(kind==='integer'&&typeof value==='string')return {integer:value};if(kind==='text'&&typeof value==='string')return {text:value};if(kind==='blob'&&bytes(value) instanceof Uint8Array)return {blob:Array.from(bytes(value))};throw new Error('fixture snapshot cell bound/type');
      }));
    }
    return {name:t.name,columns:t.columns,rows};
  });
  for(const state of [expected,observed])for(const t of state)t.rows.sort((a,b)=>JSON.stringify(a).localeCompare(JSON.stringify(b)));
  if(!sameData(observed,expected))throw new Error('fixture nonpayload state changed');
  const objects=[];for(const [kind,sql] of storage.sql.exec("SELECT type,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'").raw())objects.push({kind,sql});
  const sort=a=>a.sort((x,y)=>JSON.stringify(x).localeCompare(JSON.stringify(y)));
  const expectedObjects=[...fixture.objects,...platformSchema];
  if(!sameData(sort(objects),sort(expectedObjects))){
    const describe=o=>({kind:o.kind,sql:o.sql.slice(0,256)});
    const unexpected=objects.filter(o=>!expectedObjects.some(e=>sameData(o,e))),missing=expectedObjects.filter(o=>!objects.some(e=>sameData(o,e)));
    const error=new Error('fixture schema changed');error.schemaDiff={unexpected_count:unexpected.length,missing_count:missing.length,unexpected:unexpected.slice(0,8).map(describe),missing:missing.slice(0,8).map(describe)};throw error;
  }
}
export class DirectoryCase {
  constructor(storage,fixture,createKernel,platformSchema=[]){this.storage=storage;this.fixture=fixture;this.createKernel=createKernel;this.platformSchema=platformSchema;this.used=false;}
  async run(index,body){
    if(this.used)throw new Error('fixture cell already used');this.used=true;
    const vector=this.fixture.vectors[index];if(!vector)throw new Error('fixture case');
    const exact=await readBody(body,vector),storage=this.storage;
    seed(storage,this.fixture,exact);
    try{assertState(storage,this.fixture,exact,this.platformSchema);}catch(error){throw new Error('fixture pre-execution state: '+String(error.message),{cause:error});}
    await storage.sync();
    const kernel=this.createKernel(),before=kernel.instance.exports.memory.buffer.byteLength;
    const originalChanges=changes(storage),started=performance.now(),deadline=Date.now()+15000;
    let reply=null,outcome='ok';
    try{reply=storage.transactionSync(()=>kernel.runDirectory(Uint8Array.from(vector.command.match(/../g),b=>parseInt(b,16)),{deadline}));}
    catch(error){if(!(error instanceof KernelError))throw error;outcome=error.code;}
    const elapsed=performance.now()-started,after=kernel.instance.exports.memory.buffer.byteLength;
    if(kernel.poisoned||after>64*1024*1024||kernel.pageWorking.current!==0)throw new Error('fixture kernel memory/lifecycle');
    const replyEqual=reply===null?vector.reply===null:hex(reply)===vector.reply;
    if(outcome!==vector.outcome||!replyEqual||kernel.calls!==vector.calls||changes(storage)!==originalChanges)throw new Error('fixture kernel result/state');
    assertState(storage,this.fixture,exact,this.platformSchema);await storage.sync();
    return {name:vector.name,passed:true,equal:true,replyEqual,outcome,calls:kernel.calls,memory_before_bytes:before,memory_bytes:after,page_declared_working_peak_bytes:kernel.lastPageWorkingBytes,execution_ms:elapsed,schema_version:this.fixture.schema_version,scope:'one-record decoder; excludes fixture transport/assertions, SQLite/JS RSS and production deadline guarantees'};
  }
}
