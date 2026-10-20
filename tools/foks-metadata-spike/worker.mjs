import module from './kernel.wasm';
import catalog from './catalog.json' with {type:'json'};
import fixture from './vectors.json' with {type:'json'};
import {Kernel,KernelError,sameData} from './host.mjs';

const quote=s=>'"'+s.replaceAll('"','""')+'"';
const bytes=value=>value instanceof ArrayBuffer ? new Uint8Array(value):value;
function seedBind(value) {
  if(value==='null')return null;
  if('integer' in value)return value.integer;
  if('text' in value)return value.text;
  if('blob' in value && value.blob.length<=65536)return Uint8Array.from(value.blob);
  throw new Error('fixture cell');
}
function seed(storage,vector) {
  storage.sql.exec('PRAGMA foreign_keys=ON');
  storage.transactionSync(()=>{
    storage.sql.exec('PRAGMA defer_foreign_keys=ON');
    for(const object of fixture.objects.filter(o=>o.kind==='table'))storage.sql.exec(object.sql);
    for(const table of vector.before)for(const row of table.rows) {
      const placeholders=row.map(v=>v!=='null'&&'integer' in v?'CAST(? AS INTEGER)':'?');
      storage.sql.exec(`INSERT INTO ${quote(table.name)} VALUES(${placeholders.join(',')})`,...row.map(seedBind));
    }
    for(const object of fixture.objects.filter(o=>o.kind!=='table'))storage.sql.exec(object.sql);
    if(!storage.sql.exec('PRAGMA foreign_key_check').raw().next().done)throw new Error('fixture foreign key');
    storage.sql.exec('CREATE TABLE m0_parent(id INTEGER PRIMARY KEY)');
    storage.sql.exec('CREATE TABLE m0_child(id INTEGER REFERENCES m0_parent(id) DEFERRABLE INITIALLY DEFERRED)');
  });
}
function snapshot(storage,vector) {
  // Full-state inspection is a test assertion only, never kernel execution.
  return vector.before.map(table=>{
    const expressions=table.columns.flatMap(name=>{
      const column=quote(name);
      return [`typeof(${column})`,`CASE WHEN typeof(${column})='integer' THEN CAST(${column} AS TEXT) WHEN length(${column})<=65536 THEN ${column} ELSE NULL END`];
    });
    const rows=[];
    for(const row of storage.sql.exec(`SELECT ${expressions.join(',')} FROM ${quote(table.name)}`).raw()) {
      if(rows.length>=10000)throw new Error('snapshot row bound');
      rows.push(table.columns.map((_,i)=>{
        const kind=row[2*i],value=row[2*i+1];
        if(kind==='null')return 'null';
        if(kind==='integer' && typeof value==='string')return {integer:value};
        if(kind==='text' && typeof value==='string')return {text:value};
        if(kind==='blob' && bytes(value) instanceof Uint8Array)return {blob:Array.from(bytes(value))};
        throw new Error('snapshot cell bound/type');
      }));
    }
    rows.sort((a,b)=>JSON.stringify(a)<JSON.stringify(b)?-1:JSON.stringify(a)>JSON.stringify(b)?1:0);
    return {name:table.name,columns:table.columns,rows};
  });
}
export class FoksMetadata {
  constructor(state){this.state=state;this.used=false;}
  async fetch(request) {
    if(this.used)return new Response('one vector per fresh qualification cell',{status:409});
    this.used=true;
    const index=Number(new URL(request.url).pathname.slice(1));
    if(!Number.isSafeInteger(index)||!fixture.vectors[index])return new Response('vector',{status:400});
    const vector=fixture.vectors[index],storage=this.state.storage;
    seed(storage,vector);await storage.sync();
    const kernel=new Kernel(module,catalog,storage);
    let reply=null,outcome='ok',message=null;
    try {
      reply=storage.transactionSync(()=>{
        const reply=kernel.run(vector.command,{failAfter:vector.fail_after,trapAfter:vector.trap_after,deadline:Date.now()+1000});
        if(vector.deferred_failure)storage.sql.exec('INSERT INTO m0_child VALUES(1)');
        return reply;
      });
    } catch(error) {outcome=error instanceof KernelError?error.code:'host_failure';message=String(error.message).slice(0,300);reply=null;}
    // The reply remains private until both COMMIT and durability proof succeed.
    await storage.sync();
    const observed=snapshot(storage,vector);
    const equal=sameData(observed,vector.after);
    const replyEqual=sameData(reply,vector.reply);
    const childEmpty=storage.sql.exec('SELECT count(*) AS n FROM m0_child').one().n===0;
    const passed=equal&&replyEqual&&outcome===vector.outcome&&childEmpty;
    return Response.json({name:vector.name,passed,equal,replyEqual,outcome,message,calls:kernel.calls,memory_bytes:kernel.instance.exports.memory.buffer.byteLength,schema_version:fixture.schema_version},{status:passed?200:500});
  }
}
export default {async fetch(request,env) {
  if(request.method!=='POST'||request.headers.get('authorization')!==`Bearer ${env.M0_TOKEN}`)return new Response('unauthorized',{status:403});
  const path=new URL(request.url).pathname;
  if(!/^\/[0-9]{1,3}$/.test(path))return new Response('route',{status:404});
  return env.FOKS_M0.get(env.FOKS_M0.idFromName(path)).fetch(request);
}};
