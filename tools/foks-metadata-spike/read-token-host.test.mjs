// The cases need lab artifacts that a checkout does not have. Without them
// the file reports one skipped test instead of failing at import.
import test from 'node:test';
const missing=['FOKS_READ_TOKEN_WASM','FOKS_READ_TOKEN_DEFAULT_WASM'].filter(name=>!process.env[name]);
if(missing.length)test('read-token-host cases',{skip:`set ${missing.join(', ')} to the reviewed read-token Wasm artifacts`},()=>{});
else await import('./read-token-host.cases.mjs');
