import test from 'node:test';
import assert from 'node:assert/strict';
import worker, {DeadlineProbe, selected, expected, SIZES} from './deadline-worker.mjs';

const TOKEN = 'diagnostic-test-capability-of-fixed-length';
function request(path, options={}) {
  return new Request(`http://probe${path}`, {method:'POST', headers:{authorization:`Bearer ${TOKEN}`}, ...options});
}
function storage({badSum=false, syncError=false}={}) {
  let record = null;
  const events = [];
  return {events, sql:{exec(sql,...args) {
    events.push(sql.split(' ')[0]);
    let rows = [];
    if (sql.startsWith('INSERT')) {
      if (record) throw new Error('duplicate attempt');
      record = {stage:0,total:null};
    } else if (sql.startsWith('WITH')) {
      rows = [[badSum ? '0' : (BigInt(args[0])*(BigInt(args[0])+1n)/2n).toString()]];
    } else if (sql.startsWith('UPDATE')) record = {stage:1,total:args[0]};
    else if (sql.startsWith('SELECT')) rows = record ? [[record.stage,record.total]] : [];
    return {raw:()=>rows[Symbol.iterator]()};
  }}, transactionSync(fn) {
    events.push('begin'); const old = record;
    try { const value=fn(); events.push('commit'); return value; }
    catch(error) {record=old; events.push('rollback'); throw error;}
  }, async sync() {events.push('sync');if(syncError) throw new Error('sync unavailable');}};
}

test('capability, fixed paths and empty bodies are checked before actor routing', async()=>{
  let acquired=0;
  const env={PROBE_TOKEN:TOKEN,PROBES:{idFromName(){acquired++;return 'x';},get(){return {fetch:async()=>new Response('ok')};}}};
  for (const [req,status] of [
    [request('/main/init/0/1',{headers:{}}),403],
    [request('/main/run/4/1'),404],
    [request('/main/run/0/9'),404],
    [request('/other/run/0/1'),404],
    [request('/main/run/0/1?steps=100000000'),404],
    [request('/main/run/0/1',{method:'GET'}),405],
    [request('/main/run/0/1',{body:'x'}),400],
  ]) assert.equal((await worker.fetch(req,env)).status,status);
  assert.equal(acquired,0);
  assert.equal((await worker.fetch(request('/main/init/0/1'),env)).status,200);
  assert.equal(acquired,1);
  assert.equal(selected('http://probe/main/run/0/01'),null);
});

test('fixed aggregates publish markers only after transaction and sync', async()=>{
  assert.deepEqual(SIZES,[1000,100000,1000000,8000000]);
  assert.deepEqual(SIZES.map((_,i)=>expected(i)),['500500','5000050000','500000500000','32000004000000']);
  for(let i=0;i<4;i++) {
    const s=storage(), probe=new DeadlineProbe({storage:s});
    const response=await probe.fetch(request(`/main/run/${i}/1`));
    assert.equal(response.status,200);
    const result=await response.json();
    assert.equal(result.total,expected(i));assert.equal(result.synced,true);
    assert.equal(result.case_id,i);assert.equal(result.attempt,1);
    assert.equal(result.timer_before_sql_return,false);
    assert.deepEqual(s.events,['begin','INSERT','WITH','UPDATE','commit','sync']);
    const read=await (await probe.fetch(request(`/main/read/${i}/1`))).json();
    assert.deepEqual(read.marker,{stage:1,total:expected(i)});
    assert.ok(JSON.stringify(result).length<512);
    assert.equal((await probe.fetch(request(`/main/run/${i}/1`))).status,500);
  }
});

test('aggregate failure rolls back while sync failure remains ambiguous', async()=>{
  for(const mode of ['badSum','syncError']) {
    const s=storage({[mode]:true}), probe=new DeadlineProbe({storage:s});
    const response=await probe.fetch(request('/main/run/0/1'));
    assert.equal(response.status,500);
    const result=await response.json();
    assert.equal(result.outcome,'unknown_until_marker_observation');
    const read=await (await probe.fetch(request('/main/read/0/1'))).json();
    if(mode==='badSum') {assert.equal(read.marker,null);assert.ok(s.events.includes('rollback'));assert.ok(!s.events.includes('sync'));}
    else {assert.deepEqual(read.marker,{stage:1,total:expected(0)});assert.ok(s.events.includes('commit'));}
  }
});

test('probe timers are cleared on success, SQL rejection and sync rejection', async()=>{
  const originalSet=globalThis.setTimeout, originalClear=globalThis.clearTimeout;
  let created=0, cleared=0;
  try {
    globalThis.setTimeout=(_callback, delay)=>{assert.equal(delay,10);created++;return 77;};
    globalThis.clearTimeout=handle=>{assert.equal(handle,77);cleared++;};
    for(const mode of [{},{badSum:true},{syncError:true}]) {
      const probe=new DeadlineProbe({storage:storage(mode)});
      await probe.fetch(request('/main/run/0/1'));
    }
    assert.equal(created,3);assert.equal(cleared,3);
  } finally {globalThis.setTimeout=originalSet;globalThis.clearTimeout=originalClear;}
});

test('empty HTTP body streams work at both routing boundaries', async()=>{
  const empty=()=>new ReadableStream({start(controller){controller.close();}});
  const s=storage(), probe=new DeadlineProbe({storage:s});
  assert.equal((await probe.fetch(request('/main/init/0/1',{body:empty(),duplex:'half'}))).status,200);
  const before=s.events.length;
  assert.equal((await probe.fetch(request('/main/run/0/1',{body:'x'}))).status,400);
  assert.equal(s.events.length,before);
  let acquired=0;
  const env={PROBE_TOKEN:TOKEN,PROBES:{idFromName(){acquired++;return 'x';},get(){return {fetch:async()=>new Response('ok')};}}};
  assert.equal((await worker.fetch(request('/main/init/0/1',{body:empty(),duplex:'half'}),env)).status,200);
  assert.equal(acquired,1);
});
