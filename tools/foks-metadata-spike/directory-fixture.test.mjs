// The cases need lab artifacts that a checkout does not have. Without them
// the file reports one skipped test instead of failing at import.
import test from 'node:test';
const missing=['FOKS_DIRECTORY_VECTORS'].filter(name=>!process.env[name]);
if(missing.length)test('directory-fixture cases',{skip:`set ${missing.join(', ')} to the reviewed directory fixtures`},()=>{});
else await import('./directory-fixture.cases.mjs');
