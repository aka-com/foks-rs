// The cases need lab artifacts that a checkout does not have. Without them
// the file reports one skipped test instead of failing at import.
import test from 'node:test';
const missing=['FOKS_PAGE_WASM','FOKS_PAGE_VECTORS','FOKS_PAGE_CATALOG'].filter(name=>!process.env[name]);
if(missing.length)test('page-codec cases',{skip:`set ${missing.join(', ')} to the reviewed page artifacts`},()=>{});
else await import('./page-codec.cases.mjs');
