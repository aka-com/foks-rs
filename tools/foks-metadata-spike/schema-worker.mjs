import module from './kernel.wasm';
import catalog from './catalog.json' with {type:'json'};
import fixture from './vectors.json' with {type:'json'};
import pageArtifact from './page-catalog.json' with {type:'json'};
import {SchemaCase,authorize,emptyBody} from './schema-enumeration.mjs';
export class FoksMetadata {
  constructor(state){this.case=new SchemaCase(state.storage,fixture,module,catalog,pageArtifact);}
  async fetch(request){try{return Response.json(await this.case.run(Number(new URL(request.url).pathname.slice(1))));}catch(error){return Response.json({passed:false,error:String(error.message).slice(0,300)},{status:500});}}
}
export default {async fetch(request,env){const check=authorize(request,env.M0_TOKEN,fixture);if(check.status)return new Response('fixture denied',{status:check.status});try{await emptyBody(request);}catch{return new Response('fixture body denied',{status:400});}const path=new URL(request.url).pathname;return env.FOKS_M0.get(env.FOKS_M0.idFromName(path)).fetch(new Request(request.url,{method:'POST'}));}};
