// The cases need lab artifacts that a checkout does not have. Without them
// the file reports one skipped test instead of failing at import.
import test from 'node:test';
const missing=['FOKS_PROOF_VECTORS','FOKS_PROOF_NODE_DECODER_WASM'].filter(name=>!process.env[name]);
if(missing.length)test('authority cases',{skip:`set ${missing.join(', ')} to the native-generated vectors and the reviewed decoder Wasm`},()=>{});
else await import('./authority.cases.mjs');
