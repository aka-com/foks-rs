// The cases need lab artifacts that a checkout does not have. Without them
// the file reports one skipped test instead of failing at import.
import test from 'node:test';
const missing=['FOKS_SCALAR_WASM'].filter(name=>!process.env[name]);
if(missing.length)test('scalar-codec cases',{skip:`set ${missing.join(', ')} to the reviewed scalar Wasm artifact`},()=>{});
else await import('./scalar-codec.cases.mjs');
