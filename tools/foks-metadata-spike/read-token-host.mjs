// Private authority-host lifecycle. No frontend route accepts setup entropy.
import {Kernel} from './host.mjs';
const text=new TextEncoder();
const MAX_LINEAR_MEMORY=64*1024*1024;
export class ReadTokenHost extends Kernel {
  constructor(module,catalog,storage,pageArtifact=null){
    super(module,catalog,storage,pageArtifact);
    this.tokenReady=false;this.exportDepth=0;this.lastWipe=null;
    // The immutable qualification artifact is separately checked for a 64 MiB
    // declared maximum. This guard additionally refuses an oversized initial heap.
    if(this.instance.exports.memory.buffer.byteLength>MAX_LINEAR_MEMORY){this.poisonAndDiscard();throw new Error('token memory bound');}
    const exports={};
    for(const [name,value] of Object.entries(this.instance.exports))exports[name]=typeof value==='function'?(...args)=>this.ownedExport(value,args):value;
    this.instance={exports};
  }
  ownedExport(fn,args){
    if(this.poisoned)throw new Error('poisoned kernel instance');
    this.exportDepth++;
    try{return fn(...args);}
    catch(error){this.poisoned=true;this.tokenReady=false;throw error;}
    finally{
      this.exportDepth--;
      // An imported exception can cross nested JS/Wasm callbacks. Wait until
      // the outer export has unwound; never call Rust cleanup in trapped state.
      if(this.poisoned&&this.exportDepth===0)this.poisonAndDiscard();
    }
  }
  poisonAndDiscard(){
    this.poisoned=true;this.tokenReady=false;
    const started=performance.now();let bytes=0,complete=false;
    try{
      const memory=this.instance?.exports.memory.buffer;
      if(memory){bytes=Math.min(memory.byteLength,MAX_LINEAR_MEMORY);new Uint8Array(memory,0,bytes).fill(0);complete=bytes===memory.byteLength;}
    }finally{
      this.instance=null;
      this.lastWipe={bytes,complete,elapsed_ms:performance.now()-started};
    }
  }
  tokenExport(name,...args){
    if(this.poisoned)throw new Error('poisoned kernel instance');
    return this.instance.exports[name](...args);
  }
  clearToken(){
    if(this.running)throw new Error('token setup during invocation');
    this.tokenReady=false;this.tokenExport('read_token_clear');
  }
  rotateRandom(){
    this.clearToken(); // Failure to obtain entropy cannot retain the former key.
    const entropy=new Uint8Array(48);
    try{globalThis.crypto.getRandomValues(entropy);this.installHostEntropy(entropy);}
    finally{entropy.fill(0);}
  }
  // Consumes/wipes a trusted host-owned buffer even on rejected setup. This is
  // not reachable through fixture commands or any public/frontend request.
  installHostEntropy(entropy){
    try{
      this.clearToken();
      if(!(entropy instanceof Uint8Array)||entropy.length!==48)throw new Error('token entropy shape');
      const pointer=this.tokenExport('read_token_setup_alloc',48);
      if(!pointer)throw new Error('token setup allocation');
      this.range(pointer,48,48).set(entropy);
      if(this.tokenExport('read_token_setup')!==0)throw new Error('token setup rejected');
      this.tokenReady=true;
    }finally{if(entropy instanceof Uint8Array)entropy.fill(0);}
  }
  // Present only for explicit read-token-qualification artifacts/tests. There is
  // no fallback to this issuer in a real metadata service or ordinary Kernel.run.
  runFixture(command){
    if(this.poisoned)throw new Error('poisoned kernel instance');
    if(this.running||!this.tokenReady)throw new Error('token lifecycle guard');
    if(typeof command!=='string'||command.length>32768)throw new Error('token fixture bound');
    const bytes=text.encode(command);if(!bytes.length||bytes.length>32768)throw new Error('token fixture bound');
    this.running=true;
    try{
      const pointer=this.tokenExport('read_token_fixture_alloc',bytes.length);if(!pointer)throw new Error('token fixture allocation');
      this.range(pointer,bytes.length,32768).set(bytes);
      const status=this.tokenExport('run_read_token_fixture');
      const output=new Uint8Array(this.range(this.instance.exports.output_pointer(),this.instance.exports.output_length(),180));
      if(status!==0)throw new Error(new TextDecoder('utf-8',{fatal:true,ignoreBOM:true}).decode(output));return output;
    }finally{bytes.fill(0);this.running=false;}
  }
}
