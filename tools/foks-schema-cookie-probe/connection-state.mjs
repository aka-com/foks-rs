// Fixed runtime prerequisites only. Exclusive connection ownership is not proven.
export const TEMP_PRESENT = "SELECT CASE WHEN EXISTS(SELECT 1 FROM temp.sqlite_schema LIMIT 1) THEN '1' ELSE '0' END AS value";
export const ATTACHED_PRESENT = "SELECT CASE WHEN EXISTS(SELECT 1 FROM pragma_table_list WHERE schema NOT IN ('main','temp')) THEN '1' ELSE '0' END AS value";
const COOKIE = 'SELECT CAST(schema_version AS TEXT) AS value FROM pragma_schema_version';
const MAIN_ABSENT = "SELECT CASE WHEN EXISTS(SELECT 1 FROM main.sqlite_schema WHERE name='foks_connection_state_target') THEN '0' ELSE '1' END AS value";
export const ATTACH = "ATTACH ':memory:' AS foks_connection_state_attached";
export const CANDIDATES = [
  ['temp_table', 'CREATE TEMP TABLE foks_connection_state_table(value)'],
  ['qualified_temp_table', 'CREATE TABLE temp.foks_connection_state_qualified(value)'],
  ['temp_view', 'CREATE TEMP VIEW foks_connection_state_view AS SELECT 1 AS value'],
  ['temp_trigger', 'CREATE TEMP TRIGGER foks_connection_state_step AFTER INSERT ON main.foks_connection_state_target BEGIN SELECT 1; END'],
];
function require(condition, message) { if (!condition) throw new Error(message); }
function scalar(storage, sql) {
  const rows = storage.sql.exec(sql).toArray();
  require(Array.isArray(rows) && rows.length === 1 && rows[0] && typeof rows[0] === 'object' && !Array.isArray(rows[0]) && Object.keys(rows[0]).length === 1 && Object.hasOwn(rows[0], 'value'), 'scalar shape');
  return Object.values(rows[0])[0];
}
function bit(storage, sql) {
  const value = scalar(storage, sql);
  require(value === '0' || value === '1', 'inspection bit shape');
  return value;
}
function temp(storage) {return bit(storage,TEMP_PRESENT);}
function cookie(storage) {
  const value = scalar(storage, COOKIE);
  require(typeof value === 'string' && /^(0|[1-9][0-9]{0,9})$/.test(value) && BigInt(value) <= 0xffff_ffffn, 'cookie shape');
  return value;
}
function alive(storage) {
  require(scalar(storage, "SELECT 'alive' AS value") === 'alive', 'connection unusable');
}
export function runProbe(storage) {
  let stage = 'initial';
  const outcomes = [];
  const context = {mode:'connection-state', trusted_schema:'unqualified', production_admission:false};
  try {
    stage = 'initial-temp';
    require(temp(storage) === '0', 'initial TEMP is not empty');
    stage = 'initial-attachments';
    require(bit(storage,ATTACHED_PRESENT) === '0', 'initial attached database');
    stage = 'initial-main';
    require(scalar(storage, MAIN_ABSENT) === '1', 'initial fixture exists');
    const before = cookie(storage);
    for (const [name, sql] of CANDIDATES) {
      stage = name;
      const rollback = new Error('owned connection-state rollback');
      let candidate;
      try {
        storage.transactionSync(() => {
          require(temp(storage) === '0', 'TEMP changed before candidate');
          storage.sql.exec('CREATE TABLE main.foks_connection_state_target(value)').toArray();
          const mainCookie = cookie(storage);
          let denied = false;
          // Assertion, setup and rollback failures are never classified as denial.
          try { storage.sql.exec(sql).toArray(); } catch { denied = true; }
          const present = temp(storage);
          require(present === (denied ? '0' : '1'), 'TEMP inspection disagrees with candidate');
          require(cookie(storage) === mainCookie, 'candidate changed main schema');
          alive(storage);
          candidate = {name, outcome:denied ? 'denied' : 'detected'};
          throw rollback;
        });
        throw new Error('rollback returned normally');
      } catch (error) { if (error !== rollback) throw error; }
      require(temp(storage) === '0' && cookie(storage) === before && scalar(storage, MAIN_ABSENT) === '1', 'rollback restoration failed');
      require(candidate !== undefined, 'candidate missing');
      outcomes.push(candidate);
    }
    require(outcomes[1].outcome === 'detected', 'qualified TEMP positive detection missing');
    stage = 'attach';
    let denied = false;
    try { storage.sql.exec(ATTACH).toArray(); } catch { denied = true; }
    // ATTACH does not roll back. Unexpected acceptance ends this disposable probe.
    require(denied, 'ATTACH accepted');
    // A result/cursor failure after the effect must not masquerade as denial.
    require(bit(storage,ATTACHED_PRESENT) === '0', 'attachment survived reported denial');
    alive(storage);
    require(temp(storage) === '0' && cookie(storage) === before && scalar(storage, MAIN_ABSENT) === '1', 'final state changed');
    return {...context, passed:true, outcomes, attach_denied:true, final_attached_absent:true, final_temp_empty:true, final_table_absent:true, rollback_restored:true};
  } catch (error) {
    return {...context, passed:false, stage, outcomes, error:String(error?.message ?? error).slice(0,300)};
  }
}
