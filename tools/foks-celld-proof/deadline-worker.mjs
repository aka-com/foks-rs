// Disposable deployed-SQL diagnostic. No FOKS serving or cancellation capability.
export const SIZES = Object.freeze([1000, 100000, 1000000, 8000000]);
const CREATE = 'CREATE TABLE IF NOT EXISTS deadline_probe(case_id INTEGER NOT NULL, attempt INTEGER NOT NULL, stage INTEGER NOT NULL, total TEXT, PRIMARY KEY(case_id,attempt))';
const AGGREGATE = 'WITH RECURSIVE work(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM work WHERE n < ?) SELECT CAST(sum(n) AS TEXT) FROM work';
const READ = 'SELECT stage,total FROM deadline_probe WHERE case_id=? AND attempt=?';

export function selected(url) {
  const parsed = new URL(url);
  if (parsed.search) return null;
  const match = /^\/(main|other)\/(init|read|run)\/([0-3])\/([1-8])$/.exec(parsed.pathname);
  if (!match || (match[2] === 'run' && match[1] !== 'main')) return null;
  return {cell:match[1], action:match[2], caseId:Number(match[3]), attempt:Number(match[4])};
}
export function expected(caseId) {
  const n = BigInt(SIZES[caseId]);
  return (n * (n + 1n) / 2n).toString();
}
async function emptyBody(request) {
  if (request.body === null) return true;
  const reader = request.body.getReader();
  let timer;
  try {
    const result = await Promise.race([
      reader.read(),
      new Promise(resolve => { timer = setTimeout(() => resolve(null), 1000); }),
    ]);
    return result !== null && result.done;
  } finally {
    clearTimeout(timer);
    // Diagnostic-only stream cleanup: never await a hostile stream's cancellation.
    reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}
function one(cursor) {
  const rows = cursor.raw();
  const first = rows.next();
  if (!rows.next().done) throw new Error('diagnostic extra row');
  return first.done ? null : first.value;
}
function marker(storage, operation) {
  const value = one(storage.sql.exec(READ, operation.caseId, operation.attempt));
  if (value === null) return null;
  if (value.length !== 2 || ![0,1].includes(value[0]) ||
      !(value[1] === null || (typeof value[1] === 'string' && /^[0-9]{1,20}$/.test(value[1]))))
    throw new Error('diagnostic marker shape');
  return {stage:value[0], total:value[1]};
}

export class DeadlineProbe {
  constructor(state) { this.storage = state.storage; }
  async fetch(request) {
    const operation = selected(request.url);
    if (!operation || request.method !== 'POST' || !await emptyBody(request))
      return new Response('invalid diagnostic operation', {status:400});
    const {action, caseId, attempt} = operation;
    let timer;
    try {
      if (action === 'init') {
        this.storage.transactionSync(() => this.storage.sql.exec(CREATE));
        await this.storage.sync();
        return Response.json({initialized:true, case_id:caseId, attempt});
      }
      if (action === 'read')
        return Response.json({case_id:caseId, attempt, marker:marker(this.storage, operation)});
      let timerFired = false;
      timer = setTimeout(() => { timerFired = true; }, 10);
      let timerBeforeSqlReturn;
      this.storage.transactionSync(() => {
        // A duplicate attempt is rejected, never mistaken for a fresh success.
        this.storage.sql.exec('INSERT INTO deadline_probe(case_id,attempt,stage,total) VALUES(?,?,0,NULL)', caseId, attempt);
        console.log(`foks_deadline_sql_start ${caseId}/${attempt}`);
        const row = one(this.storage.sql.exec(AGGREGATE, SIZES[caseId]));
        timerBeforeSqlReturn = timerFired;
        console.log(`foks_deadline_sql_end ${caseId}/${attempt}`);
        if (!row || row.length !== 1 || row[0] !== expected(caseId)) throw new Error('diagnostic aggregate mismatch');
        this.storage.sql.exec('UPDATE deadline_probe SET stage=1,total=? WHERE case_id=? AND attempt=?', row[0], caseId, attempt);
      });
      await this.storage.sync();
      return Response.json({case_id:caseId, attempt, steps:SIZES[caseId], total:expected(caseId), timer_before_sql_return:timerBeforeSqlReturn, synced:true});
    } catch (error) {
      return Response.json({case_id:caseId, attempt, error:String(error?.message ?? error).slice(0,240), outcome:'unknown_until_marker_observation'}, {status:500});
    } finally { if (timer !== undefined) clearTimeout(timer); }
  }
}

export default {async fetch(request, env) {
  if (typeof env.PROBE_TOKEN !== 'string' || env.PROBE_TOKEN.length < 32 ||
      request.headers.get('authorization') !== `Bearer ${env.PROBE_TOKEN}`)
    return new Response('denied', {status:403});
  const operation = selected(request.url);
  if (!operation) return new Response('unknown diagnostic', {status:404});
  if (request.method !== 'POST') return new Response('method denied', {status:405});
  // Real HTTP empty POSTs can arrive as a stream. Read at most one chunk with
  // a bounded wait, before routing; no actor work consumes the incoming stream.
  if (!await emptyBody(request)) return new Response('body denied', {status:400});
  return env.PROBES.get(env.PROBES.idFromName(operation.cell)).fetch(new Request(request.url, {method:'POST'}));
}};
