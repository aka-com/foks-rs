import schema from "./generated-schema.json";
import { Buffer } from "node:buffer";
import { Authority, boundedJson, MAX_BYTES } from "./authority.mjs";

export class FoksProof {
  constructor(state) { this.state=state; this.authority=new Authority(state.storage,schema); this.busy=false; }
  async fetch(request) {
    // Body parsing awaits I/O and otherwise permits overlapping cell events.
    // Bound one body/candidate per cell; admission failure carries no SQL value.
    if (this.busy) return new Response("proof busy",{status:503});
    this.busy=true;
    try {
    const input = await boundedJson(request);
    let result;
    switch (new URL(request.url).pathname) {
      case "/initialize": result=this.authority.initialize(input); break;
      case "/claim": result=this.authority.claim(input); break;
      case "/snapshot": result=this.authority.read(input); break;
      case "/apply": result=this.authority.apply(input); break;
      case "/prove": result=this.authority.prove(input); break;
      default: return new Response("unsupported proof operation",{status:404});
    }
    // Outside SQL transaction; celld's response output gate additionally checks
    // durability and ownership. No candidate native response leaves before this.
    await this.state.storage.sync();
    const body=JSON.stringify(result);
    if (Buffer.byteLength(body)>MAX_BYTES) throw new Error("proof response byte limit");
    return new Response(body,{headers:{"content-type":"application/json"}});
    } finally { this.busy=false; }
  }
}

export default {
  async fetch(request,env) {
    const url=new URL(request.url);
    // Native adapters cannot promote themselves. Only the separately held
    // controller capability can initialize or advance the adapter epoch.
    const control=["/initialize","/claim"].includes(url.pathname);
    const token=control ? env.PROOF_CONTROL_TOKEN : env.PROOF_TOKEN;
    if (request.method !== "POST" || typeof token !== "string" || token.length < 32 ||
        env.PROOF_TOKEN === env.PROOF_CONTROL_TOKEN ||
        request.headers.get("authorization") !== `Bearer ${token}`) {
      return new Response("unauthorized",{status:401});
    }
    const tenant=url.searchParams.get("tenant");
    if (!tenant || !/^[a-z0-9_-]{1,64}$/.test(tenant)) return new Response("invalid tenant",{status:400});
    return env.FOKS_PROOF.get(env.FOKS_PROOF.idFromName(tenant)).fetch(request);
  }
};
