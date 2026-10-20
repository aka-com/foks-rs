// Fixed native page scripts; no activation factory or public SQL interface.
import {Kernel,sameData} from './host.mjs';
import {decodeParameters,validateSpec} from './page-codec.mjs';
export const NAMES=['full_15','full_1','suffix_0','suffix_1','suffix_15','suffix_16','unicode','name_256','name_257','sql_65536','sql_65537','sqliteX','missing_foks','changed_foks','missing_authority','changed_authority','platform_missing','platform_changed','platform_extra'];
const utf8=new TextEncoder(),caps=[8,257,257,65537];
const array=value=>value instanceof ArrayBuffer?new Uint8Array(value):value;
const equal=(a,b)=>a.length===b.length&&a.every((v,i)=>v===b[i]);
const hex=value=>{if(typeof value!=='string'||value.length%2||!/^[0-9a-f]*$/.test(value))throw new Error('fixture hex');const out=new Uint8Array(value.length/2);for(let i=0;i<out.length;i++)out[i]=parseInt(value.slice(i*2,i*2+2),16);return out;};
const fail=message=>{throw new Error(message);};
export function authorize(request,token,fixture){
  if(request.method!=='POST')return {status:405};
  if(request.headers.get('authorization')!==`Bearer ${token}`)return {status:403};
  const path=new URL(request.url).pathname;
  if(!/^\/(?:0|[1-9][0-9]?)$/.test(path))return {status:404};
  const index=Number(path.slice(1));
  if(!fixture.vectors[index]||fixture.vectors[index].name!==NAMES[index])return {status:404};
  return {index};
}
export async function emptyBody(request){
  if(request.body===null)return;
  const reader=request.body.getReader();let done=false;
  try{for(let i=0;i<4;i++){const next=await reader.read();if(next.done){done=true;return;}if(!(next.value instanceof Uint8Array)||next.value.length)fail('fixture body must be empty');}fail('fixture empty body framing');}
  finally{if(!done)await reader.cancel();reader.releaseLock();}
}
function one(storage,sql,...params){const rows=storage.sql.exec(sql,...params).raw(),row=rows.next();if(row.done||!rows.next().done)fail('fixture scalar cardinality');return row.value;}
export function snapshot(storage){
  const rows=[];let bytes=0;
  for(const row of storage.sql.exec('SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type COLLATE BINARY,name COLLATE BINARY').raw()){
    if(row.length!==4)fail('fixture schema columns');
    const name=row[1];
    if(typeof name==='string'&&name.slice(0,7).toLowerCase()==='sqlite_')continue;
    for(let i=0;i<4;i++)if(typeof row[i]!=='string'||utf8.encode(row[i]).length>caps[i])fail('fixture schema type/length');
    if(rows.length>=1024||(bytes+=utf8.encode(row[3]).length)>2*1024*1024)fail('fixture schema aggregate');
    rows.push({kind:row[0],name,table:row[2],sql:row[3]});
  }
  return rows;
}
function coordinate(storage,spec,expected){
  const row=one(storage,spec.sql);
  if(row.length!==5||row[0]!=='blob'||row[1]!=='32'||!(array(row[2]) instanceof Uint8Array)||!equal(array(row[2]),expected.incarnation)||row[3]!=='integer'||row[4]!==expected.revision)fail('fixture dependency coordinate');
  return [Array.from(array(row[2])),row[4]];
}
function cookie(storage,spec){const row=one(storage,spec.sql);if(row.length!==1||typeof row[0]!=='string'||!/^(0|[1-9][0-9]*)$/.test(row[0])||BigInt(row[0])>0xffffffffn)fail('fixture schema cookie');return row[0];}
function longView(size){const first="CREATE VIEW schema_long AS SELECT '",last="' AS v";return first+'x'.repeat(size-first.length-last.length)+last;}
export function mutation(name){
  switch(name){
    case 'unicode':return ['CREATE TABLE "\uFEFF𝄞schema"(v TEXT DEFAULT \'A B\')'];
    case 'name_256':return [`CREATE TABLE "${'é'.repeat(128)}"(v)`];
    case 'name_257':return [`CREATE TABLE "${'é'.repeat(128)}x"(v)`];
    case 'sql_65536':return [longView(65536)];case 'sql_65537':return [longView(65537)];
    case 'sqliteX':return ['CREATE TABLE sqliteX_fixture(v)'];
    case 'missing_foks':return ['DROP INDEX kv_dirent_current_name'];
    case 'changed_foks':return ['DROP TRIGGER kv_current_head_insert','CREATE TRIGGER kv_current_head_insert AFTER INSERT ON kv_dirent_heads BEGIN SELECT 1; END'];
    case 'missing_authority':return ['DROP INDEX metadata_terminal'];
    case 'changed_authority':return ['DROP TRIGGER metadata_intent_reserve','CREATE TRIGGER metadata_intent_reserve AFTER INSERT ON metadata_intents BEGIN SELECT 1; END'];
    default:if(NAMES.includes(name))return [];throw new Error('fixture mutation name');
  }
}
export function expectedUnion(fixture,name){
  const expected=fixture.baseline.map(o=>({...o}));
  if(name==='platform_missing')return expected.filter(o=>o.name!=='_cf_WAKE_update');
  if(name==='platform_changed'){const o=expected.find(o=>o.name==='_cf_WAKE_update');o.sql=o.sql.replace('sequence=sequence+1','sequence=sequence+2');}
  if(name==='platform_extra')expected.push({kind:'table',name:'_cf_fixture_unknown',table:'_cf_fixture_unknown',sql:'CREATE TABLE _cf_fixture_unknown(v)'});
  // Compare unordered sets only for the fixed extra expected artifact; observed
  // pages and the independent snapshot still have strict SQLite binary order.
  return expected;
}
function sameObjects(a,b){return a.length===b.length&&a.every(o=>b.some(e=>sameData(o,e)));}
export class SchemaCase{
  constructor(storage,fixture,module,catalog,pageArtifact){
    this.storage=storage;this.fixture=fixture;this.module=module;this.catalog=catalog;this.pageArtifact=pageArtifact;this.used=false;
  }
  async run(index){
    if(this.used)fail('fixture cell already used');this.used=true;
    const fixture=this.fixture,vector=fixture.vectors[index],storage=this.storage;
    if(!vector||vector.name!==NAMES[index])fail('fixture index');
    if(!sameData(snapshot(storage),fixture.platform))fail('unpinned fresh platform schema');
    storage.transactionSync(()=>{
      for(const object of fixture.seed.filter(o=>o.kind==='table'))storage.sql.exec(object.sql);
      storage.sql.exec('INSERT INTO metadata_read_dependency(singleton,incarnation,revision) VALUES(1,?,CAST(? AS INTEGER))',Uint8Array.from(fixture.coordinate.incarnation),fixture.coordinate.revision);
      for(const object of fixture.seed.filter(o=>o.kind!=='table'))storage.sql.exec(object.sql);
    });
    await storage.sync();
    const baseline=snapshot(storage);
    const expectedBaseline=[...fixture.seed,...fixture.platform];
    // Fixture definitions are compared as a set here; the independently captured
    // live snapshot and native page scripts retain SQLite's exact binary order.
    if(!sameObjects(baseline,expectedBaseline)||!sameData(baseline,fixture.baseline))fail('fixture seeded union');
    const catalog=new Map(this.catalog.map(s=>[s.id,s]));
    const dependency=catalog.get(43),schemaCookie=catalog.get(44);
    if(!dependency||dependency.write||!schemaCookie||schemaCookie.write)fail('fixture scalar artifact');
    const originalCookie=cookie(storage,schemaCookie),originalCoordinate=coordinate(storage,dependency,fixture.coordinate);
    const kernel=new Kernel(this.module,this.catalog,storage,this.pageArtifact);
    const spec=this.pageArtifact.descriptor.statements.find(s=>s.id===9);
    if(!spec||validateSpec(spec).rows!==15)fail('fixture page9 artifact');
    const sentinel={},started=performance.now(),deadline=Date.now()+15000;
    const check=()=>{if(Date.now()>=deadline)fail('fixture enumeration deadline');};
    let report;
    try{
      storage.transactionSync(()=>{
        for(const sql of mutation(vector.name))storage.sql.exec(sql);
        const before=cookie(storage,schemaCookie);coordinate(storage,dependency,fixture.coordinate);
        const observed=snapshot(storage);if(!sameData(observed,vector.observed??fixture.baseline))fail('independent full schema mismatch');
        const unionMatches=sameObjects(observed,expectedUnion(fixture,vector.name));
        if(unionMatches!==vector.union_matches)fail('fixture union outcome');
        let after=vector.start,rows=0,sqlBytes=0,calls=0,maximumPage=0,workingPeak=0,complete=false,invalid=false;
        for(const expected of vector.pages??fixture.vectors[0].pages){
          check();if(complete||invalid||++calls>1025)fail('fixture page termination');
          const command=hex(expected.command),decoded=decodeParameters(command,spec);
          if(decoded.requested!==vector.limit||!sameData(decoded.values,[after===null?'1':'0',after?.[0]??'',after?.[1]??'',String(vector.limit)]))fail('fixture cursor chain');
          const actual=kernel.runPage(command,{deadline});
          const expectedBytes=hex(expected.reply);
          if(!equal(actual,expectedBytes))fail('native binary schema page mismatch');
          if(new DataView(actual.buffer,actual.byteOffset,actual.byteLength).getUint16(12,true)!==expected.rows)fail('fixture page row metadata');
          rows+=expected.rows;sqlBytes+=expected.sql_bytes;
          if(rows>1024||sqlBytes>2*1024*1024)fail('fixture page aggregate');
          maximumPage=Math.max(maximumPage,actual.length);workingPeak=Math.max(workingPeak,kernel.lastPageWorkingBytes);
          invalid=!expected.valid;complete=!invalid&&expected.rows<vector.limit;
          after=expected.last;
          if(!complete&&!invalid&&after===null)fail('fixture missing continuation');
        }
        if(complete!==vector.complete||(!complete&&!invalid))fail('fixture incomplete script');
        check();if(cookie(storage,schemaCookie)!==before||!sameData(coordinate(storage,dependency,fixture.coordinate),originalCoordinate))fail('fixture enumeration changed state');
        if(!sameData(snapshot(storage),observed))fail('fixture enumeration changed schema');check();
        if(kernel.poisoned||kernel.pageWorking.current!==0||kernel.instance.exports.memory.buffer.byteLength>64*1024*1024)fail('fixture kernel lifecycle');
        report={name:vector.name,passed:true,pages:calls,rows,sql_bytes:sqlBytes,complete,invalid_field:invalid,union_matches:unionMatches,cookie:before,maximum_page_bytes:maximumPage,page_declared_working_peak_bytes:workingPeak,memory_bytes:kernel.instance.exports.memory.buffer.byteLength,execution_ms:performance.now()-started,schema_version:fixture.schema_version,scope:'page9 binary transport and fixed exact-union fixture; scalar43/44 SQL assertions only, no activation factory or latency/HA qualification'};
        throw sentinel;
      });
      fail('fixture rollback did not execute');
    }catch(error){if(error!==sentinel)throw error;}
    if(!report||cookie(storage,schemaCookie)!==originalCookie||!sameData(coordinate(storage,dependency,fixture.coordinate),originalCoordinate)||!sameData(snapshot(storage),baseline))fail('fixture rollback state');
    await storage.sync();return report;
  }
}
