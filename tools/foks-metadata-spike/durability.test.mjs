import test from 'node:test';
import assert from 'node:assert/strict';
import {executeDurably} from './durability.mjs';

test('response waits for durable sync and preserves exact committed outcome',async()=>{
  let release,committed=false,published=false;
  const gate=new Promise(resolve=>{release=resolve;});
  const exact={reply:{values:[{integer:'9007199254740993'}]}};
  const storage={transactionSync:fn=>{const value=fn();committed=true;return value;},sync:()=>gate};
  const operation=executeDurably(storage,()=>exact).then(value=>{published=true;return value;});
  await Promise.resolve();
  assert.equal(committed,true);assert.equal(published,false);
  release();assert.equal(await operation,exact);assert.equal(published,true);
});
test('commit failure cannot publish or call postcommit sync',async()=>{
  let synced=false;
  const storage={transactionSync:fn=>{fn();throw new Error('commit failed');},sync:async()=>{synced=true;}};
  await assert.rejects(executeDurably(storage,()=>({reply:'candidate'})),/commit failed/);
  assert.equal(synced,false);
});
test('sync failure stays unknown; retry returns the already committed exact outcome',async()=>{
  let attempts=0,effects=0,receipt;
  const storage={transactionSync:fn=>fn(),sync:async()=>{if(++attempts===1)throw new Error('sync unknown');}};
  const resolve=()=>receipt??(receipt={effect:++effects,bytes:[0,1,255]});
  await assert.rejects(executeDurably(storage,resolve),/sync unknown/);
  assert.deepEqual(await executeDurably(storage,resolve),{effect:1,bytes:[0,1,255]});
  assert.equal(effects,1);
});
test('held sync cannot publish after deadline; a new observation may resolve later',async()=>{
  let clock=1,release,receipt;
  const gate=new Promise(resolve=>{release=resolve;});
  const storage={transactionSync:fn=>fn(),sync:()=>gate};
  const operation=executeDurably(storage,()=>receipt={value:'committed'}, {deadline:10,now:()=>clock});
  clock=11;release();
  await assert.rejects(operation,/deadline expired/);
  assert.deepEqual(await executeDurably({...storage,sync:async()=>{}},()=>receipt,{deadline:20,now:()=>clock}),{value:'committed'});
});
test('expired admission and precommit check cannot publish or sync',async()=>{
  let clock=10,transactions=0,syncs=0;
  const storage={transactionSync:fn=>{transactions++;return fn();},sync:async()=>{syncs++;}};
  await assert.rejects(executeDurably(storage,()=>null,{deadline:10,now:()=>clock}),/deadline expired/);
  assert.equal(transactions,0);
  clock=1;
  await assert.rejects(executeDurably(storage,()=>{clock=10;return null;},{deadline:10,now:()=>clock}),/deadline expired/);
  assert.equal(transactions,1);assert.equal(syncs,0);
});
