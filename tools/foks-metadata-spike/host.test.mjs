import test from 'node:test';
import assert from 'node:assert/strict';
import {bind,sameData} from './host.mjs';

test('all signed integers use exact decimal text at the JS SQL boundary',()=>{
  for(const value of ['-9223372036854775808','9223372036854775807','9007199254740993','0'])assert.equal(bind({integer:value},'integer'),value);
  for(const value of ['9223372036854775808','-9223372036854775809','01','-0','1.0','1e3',9007199254740992])assert.throws(()=>bind({integer:value},'integer'));
});
test('blob types and lengths are validated before allocating typed copies',()=>{
  assert.deepEqual(bind({blob:[0,255,1]},'blob'),Uint8Array.of(0,255,1));
  assert.throws(()=>bind({blob:Array(4097).fill(0)},'blob'));
  assert.throws(()=>bind({blob:[-1]},'blob'));
  assert.throws(()=>bind({blob:[256]},'blob'));
  assert.throws(()=>bind({blob:[1.5]},'blob'));
  assert.equal(bind('null','nullable_blob'),null);
  assert.throws(()=>bind('null','blob'));
  assert.throws(()=>bind({integer:'1',blob:[]},'integer'));
});
test('fixture comparison ignores object key order but preserves all fields and exact values',()=>{
  const a=[{name:'table',columns:['a','b'],rows:[[{integer:'9007199254740993'},{blob:[1,2]}]]}];
  const b=[{columns:['a','b'],name:'table',rows:[[{integer:'9007199254740993'},{blob:[1,2]}]]}];
  assert.equal(sameData(a,b),true);
  for(const changed of [
    [{...b[0],rows:[[{integer:'9007199254740992'},{blob:[1,2]}]]}],
    [{...b[0],rows:[[{integer:'9007199254740993'},{blob:[2,1]}]]}],
    [{...b[0],columns:['b','a']}],
    [{...b[0],extra:1}],
    [],
  ])assert.equal(sameData(a,changed),false);
});
