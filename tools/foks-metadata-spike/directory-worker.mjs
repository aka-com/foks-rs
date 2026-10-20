import module from './kernel.wasm';
import catalog from './catalog.json' with {type:'json'};
import fixture from './vectors.json' with {type:'json'};
import pageArtifact from './page-catalog.json' with {type:'json'};
import platformSchema from './celld-directory-schema.json' with {type:'json'};
import {Kernel} from './host.mjs';
import {DirectoryCase,authorize} from './directory-fixture.mjs';
export class FoksMetadata {
  constructor(state){this.case=new DirectoryCase(state.storage,fixture,()=>new Kernel(module,catalog,state.storage,pageArtifact),platformSchema);}
  async fetch(request){try{return Response.json(await this.case.run(Number(new URL(request.url).pathname.slice(1)),request.body));}catch(error){return Response.json({passed:false,error:String(error.message).slice(0,300),schema_diff:error.schemaDiff??error.cause?.schemaDiff??null},{status:500});}}
}
export default {async fetch(request,env){
  const check=authorize(request,env.M0_TOKEN,fixture);if(check.status)return new Response('fixture denied',{status:check.status});
  const path=new URL(request.url).pathname;return env.FOKS_M0.get(env.FOKS_M0.idFromName(path)).fetch(request);
}};
