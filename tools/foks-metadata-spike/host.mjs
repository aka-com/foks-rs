// Private local qualification host. Catalog and module are deployment artifacts,
// never supplied by a request. This does not implement production auth/fencing.
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
  constructor(module,catalog,storage) {
    this.storage=storage;
    this.catalog=new Map(catalog.map(spec=>[spec.id,spec]));
    this.instance=new WebAssembly.Instance(module,{foks:{sql:(...args)=>this.sql(...args)}});
  }
  range(pointer,length,maximum) {
    const memory=this.instance.exports.memory.buffer;
    if(!Number.isInteger(pointer)||!Number.isInteger(length)||pointer<0||length<0||length>maximum||pointer+length>memory.byteLength)throw new Error('memory range');
    return new Uint8Array(memory,pointer,length);
  }
  sql(id,pointer,length,write) {
    if(++this.calls>2048 || Date.now()>this.deadline)throw new Error('host deadline/call bound');
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
    if(this.failAfter===this.calls)throw new Error('injected host failure');
    if(this.trapAfter===this.calls)this.instance.exports.trap();
    const bytes=encoder.encode(JSON.stringify(result));
    if(bytes.length>65536)throw new Error('SQL reply bound');
    const destination=this.instance.exports.response_alloc(bytes.length);
    if(!destination)throw new Error('SQL reply allocation');
    this.range(destination,bytes.length,65536).set(bytes);
    return 0;
  }
  // The caller must execute this synchronous method inside transactionSync and
  // let all throws escape it. It may release result only after COMMIT + sync.
  run(command,{failAfter=0,trapAfter=0,deadline=Date.now()+1000}={}) {
    if(typeof command!=='string'||command.length>MAX_INPUT)throw new Error('input bound');
    const bytes=encoder.encode(command);
    if(bytes.length>MAX_INPUT)throw new Error('input byte bound');
    this.calls=0;this.failAfter=failAfter;this.trapAfter=trapAfter;this.deadline=deadline;
    const pointer=this.instance.exports.input_alloc(bytes.length);
    if(!pointer)throw new Error('input allocation');
    this.range(pointer,bytes.length,MAX_INPUT).set(bytes);
    const status=this.instance.exports.run();
    const result=JSON.parse(decoder.decode(this.range(this.instance.exports.output_pointer(),this.instance.exports.output_length(),65536)));
    if(status!==0)throw new KernelError(result);
    return result;
  }
}
export class KernelError extends Error {constructor(code){super(code);this.code=code;}}
