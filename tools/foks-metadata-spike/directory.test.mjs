// The cases need lab artifacts that a checkout does not have. Without them
// the file reports one skipped test instead of failing at import.
import test from 'node:test';
const missing=['FOKS_DIRECTORY_WASM','FOKS_DIRECTORY_VECTORS'].filter(name=>!process.env[name]);
if(missing.length)test('directory cases',{skip:`set ${missing.join(', ')} to the reviewed directory artifacts`},()=>{});
else await import('./directory.cases.mjs');
