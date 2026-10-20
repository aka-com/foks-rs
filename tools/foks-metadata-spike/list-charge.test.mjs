// The cases need lab artifacts that a checkout does not have. Without them
// the file reports one skipped test instead of failing at import.
import test from 'node:test';
const missing=['FOKS_LIST_CHARGE_VECTORS','FOKS_LIST_CHARGE_WASM','FOKS_LIST_CHARGE_DEFAULT_WASM'].filter(name=>!process.env[name]);
if(missing.length)test('list-charge cases',{skip:`set ${missing.join(', ')} to the explicit list charge artifacts`},()=>{});
else await import('./list-charge.cases.mjs');
