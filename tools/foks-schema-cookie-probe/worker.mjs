import { runProbe } from './probe.mjs';

function authorized(request, env) {
  return typeof env.PROBE_TOKEN === 'string' && env.PROBE_TOKEN.length >= 32 &&
    request.headers.get('authorization') === `Bearer ${env.PROBE_TOKEN}`;
}
function admission(request, env) {
  if (!authorized(request, env)) return new Response('denied', { status: 403 });
  if (request.method !== 'POST' || new URL(request.url).pathname !== '/') return new Response('invalid request', { status: 400 });
  return null;
}
export class SchemaCookieProbe {
  constructor(state, env) { this.storage = state.storage; this.env = env; this.used = false; }
  async fetch(request) {
    const refused = admission(request, this.env);
    if (refused) return refused;
    if (this.used) return new Response('already attempted', { status: 409 });
    this.used = true;
    const result = runProbe(this.storage);
    try { await this.storage.sync(); }
    catch (error) { return Response.json({ passed: false, stage: 'sync', error: String(error?.message ?? error).slice(0, 300), candidate_passed: result.passed }, { status: 500 }); }
    return Response.json(result, { status: result.passed ? 200 : 500 });
  }
}
export default {
  async fetch(request, env) {
    const refused = admission(request, env);
    if (refused) return refused; // No cell lookup or SQL before capability check.
    return env.COOKIE_PROBE.get(env.COOKIE_PROBE.idFromName('fixed-single-attempt')).fetch(request);
  },
};
