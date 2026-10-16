// Versioned private row transport. SQL/domain decisions remain in shared Rust.
export const PAGE_ABI=1, HEADER=24, MAX_PAGE=1048576, MAX_PARAMETERS=65536;
const utf8=new TextEncoder(),text=new TextDecoder('utf-8',{fatal:true,ignoreBOM:true});
const fail=()=>{throw new Error('page contract');};
const exactKeys=(value,keys)=>value&&typeof value==='object'&&!Array.isArray(value)&&Object.keys(value).sort().join(',')===[...keys].sort().join(',');
const uint=(value,max)=>Number.isSafeInteger(value)&&value>=0&&value<=max;
const canonical=value=>Array.isArray(value)?value.map(canonical):value!==null&&typeof value==='object'?Object.fromEntries(Object.keys(value).sort().map(k=>[k,canonical(value[k])])):value;
export function sameArtifact(a,b){return JSON.stringify(canonical(a))===JSON.stringify(canonical(b));}
function columnMax(column){
  if(!exactKeys(column,['kind','maximum_bytes','nullable'])||!['integer','blob','text'].includes(column.kind)||!uint(column.maximum_bytes,MAX_PAGE)||typeof column.nullable!=='boolean')fail();
  if(column.kind==='integer'){if(column.maximum_bytes!==8)fail();return 9;}
  return 5+column.maximum_bytes;
}
export function validateSpec(spec){
  if(!exactKeys(spec,['id','sql','parameters','columns','maximum_rows','maximum_bytes','integer_ranges','limit_parameter'])||!uint(spec.id,0xffffffff)||!spec.id||typeof spec.sql!=='string'||!spec.sql||
    !Array.isArray(spec.parameters)||spec.parameters.length>16||!Array.isArray(spec.columns)||!spec.columns.length||spec.columns.length>16||
    !uint(spec.maximum_rows,64)||!spec.maximum_rows||!uint(spec.maximum_bytes,MAX_PAGE)||spec.maximum_bytes<=HEADER||!Array.isArray(spec.integer_ranges))fail();
  if(spec.parameters.reduce((n,c)=>n+columnMax(c),HEADER)>MAX_PARAMETERS)fail();
  const rowBytes=spec.columns.reduce((n,c)=>n+columnMax(c),0);
  const rows=Math.min(spec.maximum_rows,Math.floor((spec.maximum_bytes-HEADER)/rowBytes));
  if(!rows)fail();
  const integerIndex=index=>uint(index,15)&&spec.parameters[index]?.kind==='integer'&&!spec.parameters[index].nullable;
  // Range endpoints are signed i64 decimal strings in the immutable artifact.
  for(const range of spec.integer_ranges){
    if(!exactKeys(range,['parameter','minimum','maximum'])||!integerIndex(range.parameter))fail();
    if(integer(range.minimum)>integer(range.maximum))fail();
  }
  if(spec.limit_parameter!==null&&!integerIndex(spec.limit_parameter))fail();
  return {rows,rowBytes};
}
export function integer(value){
  if(typeof value!=='string'||value.length>20||!/^(?:0|-[1-9][0-9]*|[1-9][0-9]*)$/.test(value))fail();
  const n=BigInt(value);if(n<-(1n<<63n)||n>=(1n<<63n))fail();return n;
}
function limits(spec,kind){
  const checked=validateSpec(spec);
  return kind===1?{columns:spec.parameters,maximum:MAX_PARAMETERS,rows:1}:kind===2?{columns:spec.columns,maximum:spec.maximum_bytes,rows:checked.rows}:fail();
}
export function decodeParameters(bytes,spec){
  const {columns,maximum}=limits(spec,1);
  if(!(bytes instanceof Uint8Array)||bytes.length<HEADER||bytes.length>maximum)fail();
  const view=new DataView(bytes.buffer,bytes.byteOffset,bytes.byteLength);
  if(bytes[0]!==70||bytes[1]!==75||bytes[2]!==80||bytes[3]!==71||view.getUint16(4,true)!==PAGE_ABI||view.getUint16(6,true)!==1||view.getUint32(8,true)!==spec.id||view.getUint16(12,true)!==1||view.getUint16(14,true)!==columns.length||view.getUint32(16,true)!==bytes.length-HEADER||view.getUint32(20,true)!==0)fail();
  let offset=HEADER;
  const take=n=>{if(!uint(n,maximum)||n>bytes.length-offset)fail();const at=offset;offset+=n;return at;};
  const values=columns.map(column=>{
    const tag=bytes[take(1)];
    if(tag===0){if(!column.nullable)fail();return null;}
    if(tag===1){if(column.kind!=='integer')fail();return view.getBigInt64(take(8),true).toString();}
    if((tag===2&&column.kind==='blob')||(tag===3&&column.kind==='text')){
      const length=view.getUint32(take(4),true);if(length>column.maximum_bytes)fail();
      const at=take(length),value=bytes.subarray(at,at+length);
      return tag===2?new Uint8Array(value):text.decode(value);
    }
    fail();
  });
  if(offset!==bytes.length)fail();
  for(const range of spec.integer_ranges){const n=integer(values[range.parameter]);if(n<integer(range.minimum)||n>integer(range.maximum))fail();}
  const maximumRows=validateSpec(spec).rows;
  const requested=spec.limit_parameter===null?maximumRows:Number(integer(values[spec.limit_parameter]));
  if(!uint(requested,maximumRows)||!requested)fail();
  return {values,requested};
}

export class PageWriter{
  constructor(spec,limit){
    const checked=validateSpec(spec);if(!uint(limit,checked.rows)||!limit)fail();
    this.spec=spec;this.limit=limit;this.rows=0;this.offset=HEADER;
    this.bytes=new Uint8Array(spec.maximum_bytes);this.view=new DataView(this.bytes.buffer);
    this.bytes.set([70,75,80,71]);this.view.setUint16(4,PAGE_ABI,true);this.view.setUint16(6,2,true);
    this.view.setUint32(8,spec.id,true);this.view.setUint16(14,spec.columns.length,true);
  }
  push(row){
    if(!Array.isArray(row)||row.length!==this.spec.columns.length||this.rows>=this.limit)fail();
    // At most one bounded SQL cursor row is materialized. No page-wide row tree.
    const cells=row.map((value,index)=>{
      const column=this.spec.columns[index];
      if(value===null){if(!column.nullable)fail();return {tag:0,length:1};}
      if(column.kind==='integer')return {tag:1,value:integer(value),length:9};
      if(column.kind==='blob'){
        if(value instanceof ArrayBuffer)value=new Uint8Array(value);
        if(!(value instanceof Uint8Array)||value.length>column.maximum_bytes)fail();
        return {tag:2,value,length:5+value.length};
      }
      if(typeof value!=='string'||value.length>column.maximum_bytes)fail();
      const encoded=utf8.encode(value);if(encoded.length>column.maximum_bytes||text.decode(encoded)!==value)fail();
      return {tag:3,value:encoded,length:5+encoded.length};
    });
    const needed=cells.reduce((n,c)=>n+c.length,0);if(needed>this.bytes.length-this.offset)fail();
    for(const cell of cells){
      this.bytes[this.offset++]=cell.tag;
      if(cell.tag===1){this.view.setBigInt64(this.offset,cell.value,true);this.offset+=8;}
      if(cell.tag===2||cell.tag===3){this.view.setUint32(this.offset,cell.value.length,true);this.offset+=4;this.bytes.set(cell.value,this.offset);this.offset+=cell.value.length;}
    }
    this.rows++;
  }
  finish(){this.view.setUint16(12,this.rows,true);this.view.setUint32(16,this.offset-HEADER,true);return this.bytes.subarray(0,this.offset);}
}

export function activatePages(instance,artifact){
  if(instance.exports.page_abi_version?.()!==PAGE_ABI||instance.exports.page_catalog?.()!==0)fail();
  const pointer=instance.exports.output_pointer(),length=instance.exports.output_length(),memory=instance.exports.memory.buffer;
  if(!uint(pointer,memory.byteLength)||!uint(length,MAX_PARAMETERS)||!length||pointer>memory.byteLength-length)fail();
  const compiled=JSON.parse(text.decode(new Uint8Array(memory,pointer,length)));
  // Compare every descriptor field AND compiled fingerprint. A supplied hash
  // cannot authorize different SQL/contracts; no fallback to old scalar hosts.
  if(!sameArtifact(compiled,artifact)||!exactKeys(artifact,['descriptor','sha256'])||!exactKeys(artifact.descriptor,['abi_version','statements'])||artifact.descriptor.abi_version!==PAGE_ABI||!Array.isArray(artifact.descriptor.statements))fail();
  const specs=new Map();
  for(const original of artifact.descriptor.statements){
    const spec=JSON.parse(JSON.stringify(original));validateSpec(spec);
    if(specs.has(spec.id))fail();
    for(const list of [spec.parameters,spec.columns,spec.integer_ranges]){list.forEach(Object.freeze);Object.freeze(list);}Object.freeze(spec);
    specs.set(spec.id,spec);
  }
  return id=>{const spec=specs.get(id);if(!spec)fail();return spec;};
}

// Qualification-only admission of declared JS payload/container working bounds.
// This is neither allocator instrumentation nor a node-wide concurrency pool.
export class PageWorkingBudget {
  constructor(limit=8*MAX_PAGE){if(!uint(limit,8*MAX_PAGE))fail();this.limit=limit;this.current=0;this.peak=0;}
  reserve(bytes){if(!uint(bytes,this.limit)||bytes>this.limit-this.current)fail();this.current+=bytes;this.peak=Math.max(this.peak,this.current);return ()=>{this.current-=bytes;};}
}
