import test from 'node:test';
import {DatabaseSync} from 'node:sqlite';
import assert from 'node:assert/strict';
import {mkdtemp,copyFile,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';
const dir=await mkdtemp(join(tmpdir(),'foks-policy-unit-'));
let worker,SqlitePolicy,normalize,selected,compileOptions,mappedControl;
try{
  await copyFile(new URL('./worker.mjs',import.meta.url),join(dir,'worker.mjs'));
  await writeFile(join(dir,'fixture.json'),JSON.stringify({programs:[{id:'fixed',sql:'SELECT 1',parameter_count:0}],platform:[],app:[],compile_options:[],behavior:[],behavior_expected:[]}));
  ({default:worker,SqlitePolicy,normalize,selected,compileOptions,mappedControl}=await import(pathToFileURL(join(dir,'worker.mjs'))));
}finally{await rm(dir,{recursive:true,force:true});}
const token='fixed-test-capability-longer-than-thirty-two-bytes';
const request=(path,options={})=>new Request('http://policy'+path,{method:'POST',headers:{authorization:'Bearer '+token},...options});

test('capability, fixed program index and empty body precede actor access',async()=>{
  let acquired=0;
  const env={POLICY_TOKEN:token,POLICY:{idFromName(){acquired++;return 'one';},get(){return{fetch:async req=>{assert.equal(req.headers.get('authorization'),'Bearer '+token);return new Response('ok');}};}}};
  for(const [req,status] of [[request('/init',{headers:{}}),403],[request('/program/1'),400],[request('/program/00'),400],
    [request('/program/0?sql=DROP'),400],[request('/init',{method:'GET'}),400],[request('/init',{body:'sql'}),400]])
    assert.equal((await worker.fetch(req,env)).status,status);
  assert.equal(acquired,0);
  assert.equal((await worker.fetch(request('/init'),env)).status,200);assert.equal(acquired,1);
  assert.equal(selected('http://policy/program/0'),0);assert.equal(selected('http://policy/behavior'),'behavior');
});

test('root normalization preserves originals, P4, flags and non-root operands',()=>{
  const rows=[[0,'OpenRead',1,7,0,'k(2,B,)',2,null],[1,'Transaction',0,0,12,'0',1,null]];
  const normalized=normalize(rows,new Map([[7,'table/example']]));
  assert.deepEqual(normalized[0],[0,'OpenRead',1,'table/example',0,'k(2,B,)',2,null]);
  assert.equal(rows[0][3],7);assert.deepEqual(normalized[1],rows[1]);
  for(const [index,value] of [[3,8],[4,1],[6,16]]){
    const invalid=[...rows[0]];invalid[index]=value;
    assert.throws(()=>normalize([invalid],new Map([[7,'table/example']])),/unqualified root/);
  }
});

test('actor refuses malformed input and lost initialization without SQL',async()=>{
  let calls=0;const actor=new SqlitePolicy({storage:{sql:{exec(){calls++;throw Error('must not enter');}}}},{POLICY_TOKEN:token});
  assert.equal((await actor.fetch(request('/init',{headers:{}}))).status,403);
  assert.equal((await actor.fetch(request('/init',{body:'x'}))).status,400);
  const response=await actor.fetch(request('/program/0'));assert.equal(response.status,500);
  assert.match((await response.json()).error,/lost qualification/);assert.equal(calls,0);
});

test('empty streamed POST reaches initialization and wrong platform fails before mutation',async()=>{
  const calls=[];const actor=new SqlitePolicy({storage:{sql:{exec(sql){calls.push(sql);return{raw:()=>[['table','unexpected','unexpected','CREATE TABLE unexpected(v)']][Symbol.iterator]()};}},transactionSync(){throw Error('must not mutate');}}},{POLICY_TOKEN:token});
  const body=new ReadableStream({start(c){c.close();}});
  const response=await actor.fetch(request('/init',{body,duplex:'half'}));assert.equal(response.status,500);
  assert.match((await response.json()).error,/platform schema mismatch/);assert.equal(calls.length,1);
});


test('only the exact pinned pragma program can normalize one virtual-table pointer',()=>{
  const query={id:'scalar/44',sql:'SELECT CAST(schema_version AS TEXT) FROM pragma_schema_version'};
  const row=[0,'VOpen',0,0,0,'vtab:AB123',0,null];
  assert.equal(normalize([row],new Map(),query)[0][5],'vtab/builtin/pragma_schema_version');
  assert.equal(row[5],'vtab:AB123');
  for(const [rows,q] of [[[row],undefined],[[row],{...query,sql:'SELECT 1'}],[[row],{...query,id:'scalar/45'}],[[row,row],query],[[],query]])
    assert.throws(()=>normalize(rows,new Map(),q),/virtual-table/);
  assert.throws(()=>normalize([[0,'VOpen',0,0,0,'vtab:unknown',0,null]],new Map(),query),/virtual-table/);
});


test('compiler provenance remains explicit and only its reviewed pair is separated',()=>{
  const expected=['A','COMPILER=clang-21.0.0','Z'],actual=['A','COMPILER=clang-15.0.0','Z'];
  assert.deepEqual(compileOptions(expected,actual),{passed:true,feature_options_equal:true,compiler_identity:false,reviewed_compiler_pair:true,native_compiler:['COMPILER=clang-21.0.0'],deployed_compiler:['COMPILER=clang-15.0.0']});
  for(const changed of [['A','COMPILER=clang-16.0.0','Z'],['B','COMPILER=clang-15.0.0','Z'],['A','Z'],[...actual,'COMPILER=clang-15.0.0'],['Z','COMPILER=clang-15.0.0','A']])
    assert.equal(compileOptions(expected,changed).passed,false);
});


function controlStorage({denial='SQL error: prepare SQL cursor: not authorized',broken=false,swallow=false,loseReturn=false}={}){
  const db=new DatabaseSync(':memory:');let level=0;
  return {db,storage:{sql:{exec(sql){
    if(sql.startsWith('EXPLAIN ')&&denial!==null)throw Error(denial);
    return {raw:()=>db.prepare(sql).all().map(row=>Object.values(row))[Symbol.iterator]()};
  }},transactionSync(fn){
    const name='policy_'+(++level);db.exec('SAVEPOINT '+name);
    try{const value=fn();db.exec('RELEASE '+name);return loseReturn?undefined:value;}
    catch(error){if(!broken)db.exec('ROLLBACK TO '+name);db.exec('RELEASE '+name);if(!swallow)throw error;}
    finally{level--;}
  }}};
}
test('mapped control uses real nested rollback and preserves outer work and schema',()=>{
  for(const [id,sql] of [['scalar/29','SAVEPOINT metadata_domain'],['scalar/30','ROLLBACK TO metadata_domain'],['scalar/31','RELEASE metadata_domain']]){
    const {db,storage}=controlStorage();try{
      const result=mappedControl(storage,{id,sql,parameter_count:0,execution:'host-domain-transactionSync-v1'});
      assert.equal(result.passed,true);assert.equal(result.schema_cookie_unchanged,true);assert.equal(result.callback_return_preserved,true);assert.equal(result.sentinels_propagated,true);assert.ok(!('normalized_sha256' in result));
    }finally{db.close();}
  }
});
test('mapped control rejects accepted raw SQL, other errors, wrong identity and broken rollback',()=>{
  const program={id:'scalar/29',sql:'SAVEPOINT metadata_domain',parameter_count:0,execution:'host-domain-transactionSync-v1'};
  for(const options of [{denial:null},{denial:'other failure'},{broken:true},{swallow:true},{loseReturn:true}]){
    const {db,storage}=controlStorage(options);try{assert.throws(()=>mappedControl(storage,program));}finally{db.close();}
  }
  const {db,storage}=controlStorage();try{
    for(const changed of [{...program,id:'scalar/28'},{...program,sql:'SELECT 1'},{...program,parameter_count:1}])
      assert.throws(()=>mappedControl(storage,changed),/identity/);
  }finally{db.close();}
});
