// Private local qualification host. Catalog and module are deployment artifacts,
// never supplied by a request. This does not implement production auth/fencing.
import {activatePages,decodeParameters,PageWriter,PageWorkingBudget,validateSpec,HEADER,MAX_PAGE,MAX_PARAMETERS} from './page-codec.mjs';
export const MAX_INPUT = 2 * 1024 * 1024;
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', {fatal:true});
// Preserve every field and array position, but JSON object member insertion
// order is not part of the typed fixture value (serde_json sorts map keys).
export function sameData(a,b) {
  const canonical=value=>Array.isArray(value)?value.map(canonical):value!==null&&typeof value==='object'?Object.fromEntries(Object.keys(value).sort().map(key=>[key,canonical(value[key])])):value;
  return JSON.stringify(canonical(a))===JSON.stringify(canonical(b));
}
const integer = text => {
  if (typeof text !== 'string' || !/^(0|-[1-9][0-9]*|[1-9][0-9]*)$/.test(text) || text.length > 20)
    throw new Error('integer encoding');
  const n=BigInt(text);
  if(n<-(1n<<63n)||n>=(1n<<63n))throw new Error('integer range');
  return text;
};
export function bind(value, kind) {
  if(value==='null' && kind==='nullable_blob')return null;
  if(!value || typeof value!=='object' || Array.isArray(value)||Object.keys(value).length!==1)throw new Error('parameter shape');
  if(kind==='integer' && 'integer' in value)return integer(value.integer);
  if((kind==='blob'||kind==='nullable_blob') && Array.isArray(value.blob) && value.blob.length<=4096 && value.blob.every(b=>Number.isInteger(b)&&b>=0&&b<=255))return Uint8Array.from(value.blob);
  if(kind==='text' && typeof value.text==='string' && value.text.length<=4096 && encoder.encode(value.text).length<=4096)return value.text;
  throw new Error('parameter kind or size');
}
function cell(value,kind) {
  if(kind==='integer')return {integer:integer(value)};
  if(value===null && kind==='nullable_blob')return 'null';
  if(kind==='blob'||kind==='nullable_blob') {
    if(value instanceof ArrayBuffer)value=new Uint8Array(value);
    if(!(value instanceof Uint8Array)||value.length>4096)throw new Error('result blob bound');
    return {blob:Array.from(value)};
  }
  if(kind==='text' && typeof value==='string' && value.length<=4096 && encoder.encode(value).length<=4096)return {text:value};
  throw new Error('result kind');
}

export class Kernel {
  constructor(module,catalog,storage,pageArtifact=null) {
    this.storage=storage;
    this.catalog=new Map(catalog.map(spec=>[spec.id,spec]));
    this.instance=new WebAssembly.Instance(module,{foks:{sql:(...args)=>this.sql(...args),domain:(...args)=>this.domain(...args),sql_page:(...args)=>this.sqlPage(...args)}});
    this.pageMode=false;this.poisoned=false;
    this.running=false;this.domainActive=false;this.exportDepth=0;this.lastWipe=null;
    // Activation uses raw setup exports before subclass wrappers exist. Any
    // failure retires the new instance only after that synchronous call unwinds.
    try{this.pageSpec=pageArtifact===null?null:activatePages(this.instance,pageArtifact);}
    catch(error){this.poisonAndDiscard();throw error;}
  }
  range(pointer,length,maximum) {
    const memory=this.instance.exports.memory.buffer;
    if(!Number.isInteger(pointer)||!Number.isInteger(length)||pointer<0||length<0||length>maximum||pointer+length>memory.byteLength)throw new Error('memory range');
    return new Uint8Array(memory,pointer,length);
  }
  admit() {
    if(++this.calls>2048)throw new KernelError('bounds');
    if(Date.now()>=this.deadline)throw new KernelError('expired');
  }
  sqlPage(id,pointer,length){
    if(!this.running||!this.pageMode||this.domainActive||!this.pageSpec)throw new Error('page entry guard');
    this.admit();
    const spec=this.pageSpec(id);
    // Reserve before copying bindings, allocating an encoder or advancing the
    // cursor. Account UTF-16 strings, temporary UTF-8 encodings and containers.
    // This declared bound is separate from Wasm memory and SQLite internals.
    let release;
    try {release=this.pageWorking.reserve(length*4+spec.maximum_bytes+validateSpec(spec).rowBytes*3+16*512);}
    catch {throw new KernelError('bounds');}
    try {
      const encoded=new Uint8Array(this.range(pointer,length,MAX_PARAMETERS));
      let decoded,writer;
      try {decoded=decodeParameters(encoded,spec);writer=new PageWriter(spec,decoded.requested);}
      catch {throw new KernelError('bounds');}
      const cursor=this.storage.sql.exec(spec.sql,...decoded.values).raw();
      for(let index=0;index<=decoded.requested;index++){
        const row=cursor.next();if(row.done)break;
        if(index===decoded.requested)throw new KernelError('bounds');
        try{writer.push(row.value);}catch{throw new KernelError('bounds');}
      }
      if(this.failAfter===this.calls||this.failStatement===id)throw new Error('injected page host failure');
      if(this.trapAfter===this.calls||this.trapStatement===id)this.invokeExport('trap');
      const bytes=writer.finish();
      const destination=this.invokeExport('page_response_alloc',bytes.length);
      if(!destination)throw new KernelError('bounds');
      this.range(destination,bytes.length,MAX_PAGE).set(bytes);
    } finally {release();this.lastPageWorkingBytes=this.pageWorking.peak;}
    return 0;
  }
  invokeExport(name,...args){
    if(this.poisoned)throw new Error('poisoned kernel instance');
    // Nested imports can reenter allocation exports. Physical cleanup waits for
    // the outermost export to exit; a normal returned refusal never enters catch.
    this.exportDepth++;
    try {return this.instance.exports[name](...args);}
    catch(error){this.poisoned=true;throw error;}
    finally{if(--this.exportDepth===0&&this.poisoned)this.poisonAndDiscard();}
  }
  poisonAndDiscard(){
    this.poisoned=true;
    const start=performance.now();let bytes=0,complete=false;
    try{
      const buffer=this.instance?.exports.memory?.buffer;
      if(buffer){bytes=Math.min(buffer.byteLength,64*1024*1024);new Uint8Array(buffer,0,bytes).fill(0);complete=bytes===buffer.byteLength;}
    }finally{this.instance=null;this.lastWipe={bytes,complete,elapsed_ms:performance.now()-start};}
  }
  invokePageExport(name,...args){return this.invokeExport(name,...args);}
  runPage(bytes,controls={}){return this.runPageEntry('run_page',bytes,controls);}
  runDirectory(bytes,controls={}){return this.runPageEntry('run_directory_key',bytes,controls);}
  runPageEntry(entry,bytes,{deadline=Date.now()+1000,failAfter=0,trapAfter=0,failStatement=0,trapStatement=0,workingLimit=8*MAX_PAGE}={}){
    if(this.poisoned)throw new Error('poisoned kernel instance');
    if(this.running||!this.pageSpec)throw new Error('page activation/entry guard');
    if(!(bytes instanceof Uint8Array)||bytes.length<HEADER||bytes.length>MAX_PARAMETERS)throw new KernelError('bounds');
    this.pageWorking=new PageWorkingBudget(workingLimit);this.lastPageWorkingBytes=0;
    this.running=true;this.pageMode=true;this.authority=false;this.calls=0;
    this.deadline=deadline;this.failAfter=failAfter;this.trapAfter=trapAfter;this.failStatement=failStatement;this.trapStatement=trapStatement;
    try{
      if(Date.now()>=deadline)throw new KernelError('expired');
      const pointer=this.invokePageExport('page_input_alloc',bytes.length);
      if(!pointer)throw new KernelError('bounds');
      this.range(pointer,bytes.length,MAX_PARAMETERS).set(bytes);
      // An imported exception/trap skips Rust destructors. Never reuse this
      // instance after an export throws; a normal returned error remains safe.
      const status=this.invokePageExport(entry);
      const output=this.range(this.invokeExport('output_pointer'),this.invokeExport('output_length'),MAX_PAGE);
      if(status!==0)throw new KernelError(decoder.decode(output));
      if(Date.now()>=deadline)throw new KernelError('expired');
      let release;try{release=this.pageWorking.reserve(output.length);}catch{throw new KernelError('bounds');}
      try{return new Uint8Array(output);}finally{release();this.lastPageWorkingBytes=this.pageWorking.peak;}
    }finally{this.pageMode=false;this.running=false;}
  }
  sql(id,pointer,length,write) {
    this.admit();
    const spec=this.catalog.get(id);
    if(!spec || Number(spec.write)!==write)throw new Error('statement ID');
    const values=JSON.parse(decoder.decode(this.range(pointer,length,65536)));
    if(!Array.isArray(values)||values.length!==spec.parameters.length)throw new Error('parameter count');
    const cursor=this.storage.sql.exec(spec.sql,...values.map((v,i)=>bind(v,spec.parameters[i])));
    const rows=cursor.raw();
    const first=rows.next();
    let result;
    if(write) {
      if(!first.done)throw new Error('unexpected write rows');
      // rowsWritten includes trigger changes in celld. changes() is the
      // statement's direct affected rows, matching rusqlite::execute.
      const count=this.storage.sql.exec('SELECT CAST(changes() AS TEXT)').raw().next();
      if(count.done)throw new Error('missing change count');
      result={integer:integer(count.value[0])};
    } else {
      if(!first.done && first.value.length!==spec.columns.length)throw new Error('column count');
      result=first.done ? null : first.value.map((v,i)=>cell(v,spec.columns[i]));
      if(!first.done && !rows.next().done)throw new Error('row bound');
    }
    // Injected failures are qualification-only, after a real SQL statement.
    if(this.failAfter===this.calls||this.failStatement===id)throw new Error('injected host failure');
    if(this.trapAfter===this.calls||this.trapStatement===id)this.invokeExport('trap');
    return this.respond(result);
  }
  respond(result) {
    const bytes=encoder.encode(JSON.stringify(result));
    if(bytes.length>65536)throw new Error('SQL reply bound');
    const destination=this.invokeExport('response_alloc',bytes.length);
    if(!destination)throw new Error('SQL reply allocation');
    this.range(destination,bytes.length,65536).set(bytes);
    return 0;
  }
  domain(pointer,length) {
    if(!this.running||!this.authority||this.domainActive)throw new Error('domain entry guard');
    // Copy before input_alloc can grow memory. The outer Rust input is owned,
    // and no Rust RefCell buffer borrow survives this synchronous import.
    const command=decoder.decode(new Uint8Array(this.range(pointer,length,MAX_INPUT)));
    this.domainActive=true;
    let rejection;
    try {
      this.admit(); // logical SAVEPOINT charge, matching NativeSql
      let result;
      try {
        const reply=this.storage.transactionSync(()=>this.invoke(command,false,code=>{
          // Only a decoded nonzero domain export status creates this token.
          // Exceptions from imported SQL, even KernelError instances, bypass
          // the status factory and can never become durable rejections.
          if(['conflict','permission','quota'].includes(code)) {
            rejection={code};return rejection;
          }
          return new KernelError(code);
        }));
        this.admit(); // logical RELEASE charge before successful return
        result={success:reply};
      } catch(error) {
        if(!rejection||error!==rejection)throw error;
        this.admit(); // logical ROLLBACK charge
        this.admit(); // logical RELEASE charge
        result={rejected:rejection.code};
      }
      // invoke copied nested OUTPUT before this allocation touches RESPONSE.
      return this.respond(result);
    } finally {this.domainActive=false;}
  }
  invoke(command,authority,statusError=code=>new KernelError(code)) {
    if(typeof command!=='string'||command.length>MAX_INPUT)throw new Error('input bound');
    const bytes=encoder.encode(command);
    if(bytes.length>MAX_INPUT)throw new Error('input byte bound');
    const pointer=this.invokeExport('input_alloc',bytes.length);
    if(!pointer)throw new Error('input allocation');
    this.range(pointer,bytes.length,MAX_INPUT).set(bytes);
    const entry=authority==='personal'?'run_personal':authority==='personal_point'?'read_personal':authority?'run_authority':'run';
    const status=this.invokeExport(entry);
    const result=JSON.parse(decoder.decode(this.range(this.invokeExport('output_pointer'),this.invokeExport('output_length'),65536)));
    if(status!==0)throw statusError(result);
    return result;
  }
  // Only this outer entry resets controls. Domain reentry inherits all limits.
  // The caller must let all errors escape its transactionSync and await sync.
  run(command,{failAfter=0,trapAfter=0,failStatement=0,trapStatement=0,deadline=Date.now()+1000,authority=false,personal=false}={}) {
    if(this.poisoned)throw new Error('poisoned kernel instance');
    if(this.running)throw new Error('outer entry guard');
    if(personal!==false&&personal!=='authority'&&personal!=='point')throw new Error('personal entry selector');
    if(personal&&authority)throw new Error('mixed entry selectors');
    const entry=personal==='authority'?'personal':personal==='point'?'personal_point':authority;
    this.running=true;
    this.calls=0;this.failAfter=failAfter;this.trapAfter=trapAfter;
    this.failStatement=failStatement;this.trapStatement=trapStatement;
    this.deadline=deadline;this.authority=authority||personal==='authority';
    try {
      if(Date.now()>=this.deadline)throw new KernelError('expired');
      return this.invoke(command,entry,code=>personal?new PersonalError(code):new KernelError(code));
    }
    finally {this.running=false;}
  }
}
export class KernelError extends Error {constructor(code){super(code);this.code=code;}}

// Never a domain callback rejection: this is an outer typed resolver failure.
export class PersonalError extends Error {constructor(payload){super(JSON.stringify(payload));this.payload=payload;}}
