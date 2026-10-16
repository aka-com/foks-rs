// Qualification only: cumulative Rust charges and actual Wasm memory observations.
// This is not a serving adapter, reusable memory allowance, or publication gate.
import {Kernel,KernelError} from './host.mjs';
import {activatePages,PageWorkingBudget,MAX_PAGE} from './page-codec.mjs';

export const MAX_CHARGE_INPUT=20_480,MAX_CHARGE_OUTPUT=8192,MAX_CHARGE_MEMORY=64*1024*1024;
const encoder=new TextEncoder(),decoder=new TextDecoder('utf-8',{fatal:true,ignoreBOM:true});
export const CHARGE_EXPORTS=['list_charge_input_alloc','run_list_charge','list_charge_layout'];
const BASE_FUNCTIONS=['input_alloc','output_length','output_pointer','page_abi_version','page_catalog','page_input_alloc','page_response_alloc','read_personal','read_token_clear','read_token_setup','read_token_setup_alloc','response_alloc','run','run_authority','run_directory_key','run_page','run_personal','small_box_ciphertext_length','small_box_input_alloc','trap'];

export function validateChargeModule(module,feature){
  const expected=[['memory','memory'],['__data_end','global'],['__heap_base','global'],...BASE_FUNCTIONS.map(n=>[n,'function']),...(feature?CHARGE_EXPORTS.map(n=>[n,'function']):[])];
  const actual=WebAssembly.Module.exports(module).map(e=>[e.name,e.kind]);
  const canonical=a=>JSON.stringify(a.sort((a,b)=>a[0].localeCompare(b[0])));
  if(canonical(actual)!==canonical(expected))throw new Error('trace artifact exports');
  const imports=WebAssembly.Module.imports(module).map(i=>[`${i.module}.${i.name}`,i.kind]);
  if(canonical(imports)!==canonical(['sql','sql_page','domain'].map(n=>[`foks.${n}`,'function'])))throw new Error('trace artifact imports');
}

export class ListChargeHost extends Kernel {
  #pageRefusal=null;
  constructor(module,catalog,storage,pageArtifact){
    validateChargeModule(module,true);
    // Wrap exports before page activation: a setup trap must also discard the
    // instance. The artifact's declared maximum is checked separately, statically.
    super(module,catalog,storage,null);
    this.exportDepth=0;this.lastWipe=null;
    const exports={};
    for(const [name,value] of Object.entries(this.instance.exports))exports[name]=typeof value==='function'?(...args)=>{
      const result=this.ownedExport(value,args);
      // Only this normal return can create the scoped refusal. A thrown export
      // never reaches here and is already poisoned by ownedExport.
      if(name==='page_response_alloc'&&result===0&&this.#pageRefusal!==null)throw this.#pageRefusal;
      return result;
    }:value;
    this.instance={exports};
    try{
      if(exports.memory.buffer.byteLength>MAX_CHARGE_MEMORY)throw new Error('trace memory bound');
      this.pageSpec=activatePages(this.instance,pageArtifact);
    }catch(error){this.poisonAndDiscard();throw error;}
  }
  ownedExport(fn,args){
    if(this.poisoned)throw new Error('poisoned trace instance');
    this.exportDepth++;
    try{return fn(...args);}
    catch(error){this.poisoned=true;throw error;}
    finally{if(--this.exportDepth===0&&this.poisoned)this.poisonAndDiscard();}
  }
  poisonAndDiscard(){
    this.poisoned=true;
    const start=performance.now();let bytes=0,complete=false;
    try{
      const buffer=this.instance?.exports.memory.buffer;
      if(buffer){bytes=Math.min(buffer.byteLength,MAX_CHARGE_MEMORY);new Uint8Array(buffer,0,bytes).fill(0);complete=bytes===buffer.byteLength;}
    }finally{this.instance=null;this.lastWipe={bytes,complete,elapsed_ms:performance.now()-start};}
  }
  domain(){throw new Error('trace domain import forbidden');}
  sql(id,pointer,length,write){
    if(!this.running||write!==0)throw new Error('trace SQL phase/write');
    return super.sql(id,pointer,length,write);
  }
  sqlPage(...args){
    if(this.#pageRefusal!==null)throw new Error('trace nested page import');
    const refusal={};this.#pageRefusal=refusal;
    try{return super.sqlPage(...args);}
    catch(error){
      // This throw is caught on the JS side of the import. Rust sees a nonzero
      // status, unwinds normally, and the recorder reports its prior refusal.
      if(error===refusal)return 1;
      throw error;
    }finally{this.#pageRefusal=null;}
  }
  // Raw Kernel entries are deliberately unavailable through this fixture owner.
  run(){throw new Error('trace entry only');}
  runPage(){throw new Error('trace entry only');}
  runDirectory(){throw new Error('trace entry only');}
  begin(controls){
    if(this.poisoned||!this.instance)throw new Error('poisoned trace instance');
    if(this.running)throw new Error('trace reentry');
    const {deadline=Date.now()+15000,failAfter=0,trapAfter=0,failStatement=0,trapStatement=0}=controls;
    if(!Number.isFinite(deadline)||Date.now()>=deadline)throw new KernelError('expired');
    this.running=true;this.pageMode=true;this.authority=false;this.calls=0;
    this.deadline=deadline;this.failAfter=failAfter;this.trapAfter=trapAfter;this.failStatement=failStatement;this.trapStatement=trapStatement;
    this.pageWorking=new PageWorkingBudget(8*MAX_PAGE);this.lastPageWorkingBytes=0;
  }
  output(status){
    // Getter calls can trap too. Copy only after all getters; never retain a view
    // across export reentry or memory growth.
    const pointer=this.instance.exports.output_pointer(),length=this.instance.exports.output_length();
    const bytes=new Uint8Array(this.range(pointer,length,MAX_CHARGE_OUTPUT));
    const text=decoder.decode(bytes);
    if(status!==0)throw new KernelError(text);
    return JSON.parse(text);
  }
  layout(){
    this.begin({});
    try{const report=this.output(this.instance.exports.list_charge_layout());this.chargeLayout=report;return report;}
    finally{this.pageMode=false;this.running=false;}
  }
  runCharge(command,controls={}){
    if(typeof command!=='string'||!command.length||command.length>MAX_CHARGE_INPUT)throw new Error('trace input bound');
    const bytes=encoder.encode(command);
    try{
      if(bytes.length>MAX_CHARGE_INPUT)throw new Error('trace input byte bound');
      this.begin(controls);
      try{
        const pointer=this.instance.exports.list_charge_input_alloc(bytes.length);
        if(!pointer)throw new KernelError('bounds');
        this.range(pointer,bytes.length,MAX_CHARGE_INPUT).set(bytes);
        return this.output(this.instance.exports.run_list_charge());
      }finally{this.pageMode=false;this.running=false;}
    }finally{bytes.fill(0);}
  }
}
