// The cases need lab artifacts that a checkout does not have. Without them
// the file reports one skipped test instead of failing at import.
import test from 'node:test';
const missing=['FOKS_SCHEMA_ENUMERATION_VECTORS','FOKS_SCHEMA_ENUMERATION_WASM'].filter(name=>!process.env[name]);
if(missing.length)test('schema-enumeration cases',{skip:`set ${missing.join(', ')} to the reviewed native schema scripts and Wasm`},()=>{});
else await import('./schema-enumeration.cases.mjs');
