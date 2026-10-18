// Disposable fixed-schema policy diagnostic; never a production SQL adapter.
import fixture from './fixture.json' with {type:'json'};
const ROOTS='SELECT type,name,rootpage FROM sqlite_schema WHERE rootpage>0 ORDER BY rootpage';
const SCHEMA="SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY type,name";
const TEMP_QUERY={id:'connection/empty_temp',sql:"SELECT CASE WHEN EXISTS(SELECT 1 FROM temp.sqlite_schema) THEN '1' ELSE '0' END"};
const ATTACHMENTS_QUERY={id:'connection/no_attachments',sql:"SELECT CASE WHEN EXISTS(SELECT 1 FROM pragma_table_list WHERE schema NOT IN ('main','temp')) THEN '1' ELSE '0' END AS value"};
const utf8=new TextEncoder();
export function normalize(rows,roots,query){
  const exact=expected=>query?.id===expected.id&&query.sql===expected.sql;
  const virtual=exact({id:'scalar/44',sql:'SELECT CAST(schema_version AS TEXT) FROM pragma_schema_version'})?'pragma_schema_version':
    exact(ATTACHMENTS_QUERY)?'pragma_table_list':null;
  const temp=exact(TEMP_QUERY);
  let vopen=0,tempRoots=0;
  const out=rows.map(original=>{
    if(original.length!==8||typeof original[1]!=='string')throw Error('EXPLAIN row shape');
    const row=[...original];
    if(['OpenRead','OpenWrite','ReopenIdx'].includes(row[1])){
      if(temp&&row[1]==='OpenRead'&&row[4]===1&&row[3]===1&&!(row[6]&16)){
        tempRoots++;row[3]='table/temp.sqlite_schema';
      }else{
        if(row[4]!==0||(row[6]&16)||!roots.has(row[3]))throw Error('unqualified root operand');
        row[3]=roots.get(row[3]);
      }
    }
    if(row[1]==='VOpen'){
      if(!virtual||typeof row[5]!=='string'||!/^vtab:[0-9A-Fa-f]+$/.test(row[5]))throw Error('unqualified virtual-table operand');
      vopen++;row[5]='vtab/builtin/'+virtual;
    }
    return row;
  });
  if(vopen!==(virtual?1:0))throw Error('virtual-table operand count');
  if(tempRoots!==(temp?1:0))throw Error('temp schema operand count');
  return out;
}
export function selected(url){
  const u=new URL(url);if(u.search)return null;
  if(u.pathname==='/init'||u.pathname==='/behavior')return u.pathname.slice(1);
  const m=/^\/program\/(0|[1-9][0-9]{0,3})$/.exec(u.pathname);
  return m&&Number(m[1])<fixture.programs.length?Number(m[1]):null;
}
async function emptyBody(request){
  if(request.body===null)return true;
  const reader=request.body.getReader();let timer;
  try{const next=await Promise.race([reader.read(),new Promise(resolve=>{timer=setTimeout(()=>resolve(null),1000);})]);return next!==null&&next.done;}
  finally{clearTimeout(timer);reader.cancel().catch(()=>{});reader.releaseLock();}
}
function rows(storage,sql,parameters=[],maxRows=25000){
  const out=[];let bytes=0;
  for(const row of storage.sql.exec(sql,...parameters).raw()){
    if(!Array.isArray(row)||row.length>128||out.length>=maxRows)throw Error('diagnostic result shape/count');
    for(const value of row){
      if(value===null)bytes++;
      else if(typeof value==='number'&&Number.isSafeInteger(value))bytes+=20;
      else if(typeof value==='string'){const n=utf8.encode(value).length;if(n>65536)throw Error('diagnostic text bound');bytes+=n;}
      else throw Error('diagnostic result type');
    }
    if(bytes>8*1024*1024)throw Error('diagnostic result bytes');out.push(row);
  }
  return out;
}
function same(a,b){return JSON.stringify(a)===JSON.stringify(b);}
function sorted(value){return [...value].sort((a,b)=>a[0]<b[0]?-1:a[0]>b[0]?1:a[1]<b[1]?-1:a[1]>b[1]?1:0);}
function rootMap(storage){
  const result=new Map([[1,'table/sqlite_schema']]);
  for(const [kind,name,page] of rows(storage,ROOTS,[],1024)){
    if(!['table','index'].includes(kind)||!Number.isSafeInteger(page)||page<2||result.has(page))throw Error('root inventory');
    result.set(page,`${kind}/${name}`);
  }
  return result;
}
async function hash(value){const bytes=utf8.encode(JSON.stringify(value));return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),v=>v.toString(16).padStart(2,'0')).join('');}
function authorized(request,env){return typeof env?.POLICY_TOKEN==='string'&&env.POLICY_TOKEN.length>=32&&request.headers.get('authorization')===`Bearer ${env.POLICY_TOKEN}`;}
export function compileOptions(expected,actual){
  const compiler=values=>values.filter(v=>typeof v==='string'&&v.startsWith('COMPILER='));
  const a=compiler(expected),b=compiler(actual);
  const reviewed=a.length===1&&b.length===1&&a[0]==='COMPILER=clang-21.0.0'&&b[0]==='COMPILER=clang-15.0.0';
  const features=same(expected.filter(v=>!String(v).startsWith('COMPILER=')),actual.filter(v=>!String(v).startsWith('COMPILER=')));
  return {passed:reviewed&&features,feature_options_equal:features,compiler_identity:a.length===1&&b.length===1&&a[0]===b[0],reviewed_compiler_pair:reviewed,native_compiler:a,deployed_compiler:b};
}
const CONTROL_BINDING='host-domain-transactionSync-v1';
const CONTROLS={'scalar/29':'SAVEPOINT metadata_domain','scalar/30':'ROLLBACK TO metadata_domain','scalar/31':'RELEASE metadata_domain'};
const CONTROL_OUTCOMES=[[['0']],[[1]],[[1]],[[1],[3]],[['0']]];
export function mappedControl(storage,program){
  if(program.execution!==CONTROL_BINDING||CONTROLS[program.id]!==program.sql||program.parameter_count!==0)throw Error('mapped control identity');
  let denied=false;
  try{rows(storage,'EXPLAIN '+program.sql);}catch(error){
    if(error?.message!=='SQL error: prepare SQL cursor: not authorized')throw error;
    denied=true;
  }
  if(!denied)throw Error('raw control unexpectedly accepted');
  const readCookie=()=>{const result=rows(storage,'PRAGMA schema_version',[],1);
    if(result.length!==1||result[0].length!==1||!Number.isSafeInteger(result[0][0])||result[0][0]<0||result[0][0]>0xffffffff)throw Error('schema cookie shape');
    return result;};
  const cookie=readCookie();
  const absent=()=>rows(storage,"SELECT CAST(count(*) AS TEXT) FROM sqlite_schema WHERE name='policy_domain'");
  const outcomes=[absent()],outer={},inner={},returned={};
  let innerCaught=false,outerCaught=false;
  if(!same(outcomes[0],[['0']]))throw Error('control fixture not fresh');
  try{storage.transactionSync(()=>{
    rows(storage,'CREATE TABLE policy_domain(v INTEGER)');
    const value=storage.transactionSync(()=>{rows(storage,'INSERT INTO policy_domain VALUES(1)');return returned;});
    if(value!==returned)throw Error('control return value mismatch');
    outcomes.push(rows(storage,'SELECT v FROM policy_domain ORDER BY v'));
    try{storage.transactionSync(()=>{rows(storage,'INSERT INTO policy_domain VALUES(2)');throw inner;});}
    catch(error){if(error!==inner)throw error;innerCaught=true;}
    outcomes.push(rows(storage,'SELECT v FROM policy_domain ORDER BY v'));
    rows(storage,'INSERT INTO policy_domain VALUES(3)');
    outcomes.push(rows(storage,'SELECT v FROM policy_domain ORDER BY v'));
    throw outer;
  });}catch(error){if(error!==outer)throw error;outerCaught=true;}
  outcomes.push(absent());
  const unchanged=same(cookie,readCookie());
  if(!same(outcomes,CONTROL_OUTCOMES)||!unchanged||!innerCaught||!outerCaught)throw Error('mapped control behavior mismatch');
  return {passed:true,id:program.id,binding:CONTROL_BINDING,sql_authorization_denied:true,outcomes,schema_cookie_unchanged:unchanged,callback_return_preserved:true,sentinels_propagated:true,production_admission:false};
}
export class SqlitePolicy {
  constructor(state,env){this.storage=state.storage;this.env=env;this.initialized=false;}
  async fetch(request){
    if(!authorized(request,this.env))return new Response('denied',{status:403});
    const operation=selected(request.url);
    if(operation===null||request.method!=='POST'||!await emptyBody(request))return new Response('invalid diagnostic',{status:400});
    try{
      const storage=this.storage;
      if(operation==='init'){
        if(this.initialized)throw Error('diagnostic actor already initialized');
        if(!same(rows(storage,SCHEMA,[],1024),sorted(fixture.platform)))throw Error('fresh platform schema mismatch');
        for(const [name,value] of [['foreign_keys',1],['ignore_check_constraints',0],['recursive_triggers',1]]){
          rows(storage,`PRAGMA ${name}=${value}`);if(!same(rows(storage,`PRAGMA ${name}`),[[value]]))throw Error('connection setting mismatch');
        }
        storage.transactionSync(()=>{
          for(const kind of ['table','index','trigger'])for(const object of fixture.app)if(object[0]===kind)rows(storage,object[3]);
        });
        await storage.sync();
        const actual=rows(storage,SCHEMA,[],1024).filter(row=>row[1]!=='sqlite_sequence');
        if(!same(actual,sorted([...fixture.platform,...fixture.app])))throw Error('installed trusted schema mismatch');
        const options=[];
        for(let i=0;i<=128;i++){
          const result=rows(storage,'SELECT sqlite_compileoption_get(?)',[i],1);
          if(result.length!==1||result[0].length!==1)throw Error('compile-option shape');
          if(result[0][0]===null)break;
          if(typeof result[0][0]!=='string'||i===128)throw Error('compile-option bound');options.push(result[0][0]);
        }
        this.initialized=true;
        const provenance=compileOptions(fixture.compile_options,options);
        return Response.json({passed:provenance.passed,compile_options:options,compiler_provenance:provenance,production_admission:false});
      }
      if(!this.initialized)throw Error('actor lost qualification initialization');
      if(operation==='behavior'){
        const outcomes=[],sentinel={};
        try{storage.transactionSync(()=>{
          for(const [expected,sql] of fixture.behavior){
            let result;
            try{result=rows(storage,sql);}catch(error){
              if(expected!=='reject')throw error;
              const text=String(error.message),category=text.includes('CHECK constraint failed')?'check':text.includes('FOREIGN KEY constraint failed')?'foreign_key':null;
              if(category===null)throw error;outcomes.push(['reject',category]);continue;
            }
            if(expected==='reject')throw Error('expected constraint rejection');outcomes.push([expected,result]);
          }
          throw sentinel;
        });}catch(error){if(error!==sentinel)throw error;}
        const empty=rows(storage,'SELECT CAST(count(*) AS TEXT) FROM metadata_read_dependency');
        const absent=rows(storage,"SELECT CAST(count(*) AS TEXT) FROM sqlite_schema WHERE name IN ('policy_recursive','policy_step')");
        return Response.json({passed:same(outcomes,fixture.behavior_expected)&&same(empty,[['0']])&&same(absent,[['0']]),outcomes,production_admission:false});
      }
      const program=fixture.programs[operation];
      if(program.execution===CONTROL_BINDING)return Response.json(mappedControl(storage,program));
      if(program.execution!=='sql')throw Error('program execution binding');
      const roots=rootMap(storage);
      // Catalog placeholders are numbered. Binding nulls preserves preparation;
      // EXPLAIN never executes these application mutations.
      const count=program.parameter_count;
      if(!Number.isSafeInteger(count)||count<0||count>100)throw Error('fixture parameter count');
      const programRows=rows(storage,'EXPLAIN '+program.sql,Array(count).fill(null));
      const observed=await hash(normalize(programRows,roots,program));
      return Response.json({passed:observed===program.normalized_sha256,id:program.id,normalized_sha256:observed,roots:Array.from(roots),rows:programRows,production_admission:false});
    }catch(error){return Response.json({passed:false,error:String(error?.message??error).slice(0,300),production_admission:false},{status:500});}
  }
}
export default {async fetch(request,env){
  if(!authorized(request,env))return new Response('denied',{status:403});
  if(request.method!=='POST'||selected(request.url)===null||!await emptyBody(request))return new Response('invalid diagnostic',{status:400});
  return env.POLICY.get(env.POLICY.idFromName('fixed-policy')).fetch(new Request(request.url,{method:'POST',headers:{authorization:request.headers.get('authorization')}}));
}};
