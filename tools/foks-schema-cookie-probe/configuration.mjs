// Fixed connection-capability qualification, not production schema admission.
const COOKIE = 'SELECT CAST(schema_version AS TEXT) AS value FROM pragma_schema_version';
const INVENTORY = "SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type COLLATE BINARY,name COLLATE BINARY,tbl_name COLLATE BINARY";
const PREFIX_COUNT = "SELECT CAST(count(*) AS TEXT) AS value FROM sqlite_schema WHERE substr(name,1,22)='foks_connection_probe_'";
const SETTINGS = ['foreign_keys', 'ignore_check_constraints', 'recursive_triggers'];
const EXPECTED = [1, 0, 1];
const CREATE = [
  'CREATE TABLE foks_connection_probe_parent(id INTEGER PRIMARY KEY)',
  'CREATE TABLE foks_connection_probe_child(parent INTEGER REFERENCES foks_connection_probe_parent(id))',
  'CREATE TABLE foks_connection_probe_check(value INTEGER NOT NULL CHECK(value>0))',
  'CREATE TABLE foks_connection_probe_recursive(id INTEGER PRIMARY KEY,value INTEGER NOT NULL)',
  'CREATE TRIGGER foks_connection_probe_step AFTER UPDATE ON foks_connection_probe_recursive WHEN NEW.value<3 BEGIN UPDATE foks_connection_probe_recursive SET value=NEW.value+1 WHERE id=NEW.id; END',
];
// Every statement succeeds in unrestricted SQLite with the fixture target alive.
export const DENIED = [
  ['temp_table', 'CREATE TEMP TABLE foks_connection_probe_shadow(value)'],
  ['qualified_temp_table', 'CREATE TABLE temp.foks_connection_probe_qualified(value)'],
  ['temp_view', 'CREATE TEMP VIEW foks_connection_probe_view AS SELECT 1 AS value'],
  ['temp_trigger', 'CREATE TEMP TRIGGER foks_connection_probe_temp_step AFTER INSERT ON main.foks_connection_probe_parent BEGIN SELECT 1; END'],
  ['attach', "ATTACH ':memory:' AS foks_connection_probe_attached"],
];
function require(condition, message) { if (!condition) throw new Error(message); }
function scalar(storage, sql) {
  const rows = storage.sql.exec(sql).toArray();
  require(rows.length === 1 && rows[0] && typeof rows[0] === 'object' && Object.keys(rows[0]).length === 1, 'scalar shape');
  return Object.values(rows[0])[0];
}
function settings(storage) {
  return SETTINGS.map(name => {
    const value = scalar(storage, 'PRAGMA ' + name);
    require(value === 0 || value === 1, 'boolean PRAGMA shape');
    return value;
  });
}
function cookie(storage) {
  const value = scalar(storage, COOKIE);
  require(typeof value === 'string' && /^(0|[1-9][0-9]{0,9})$/.test(value) && BigInt(value) <= 0xffff_ffffn, 'cookie shape');
  return value;
}
function inventory(storage) {
  const rows = storage.sql.exec(INVENTORY).toArray();
  require(rows.length <= 1024, 'inventory row bound');
  let size = 0;
  const result = rows.map(row => {
    require(row && Object.keys(row).length === 4, 'inventory shape');
    return ['type','name','tbl_name','sql'].map(key => {
      const value = row[key];
      require(typeof value === 'string' || (key === 'sql' && value === null), 'inventory type');
      size += value?.length ?? 0;
      require(size <= 2 * 1024 * 1024, 'inventory byte bound');
      return value;
    });
  });
  return JSON.stringify(result);
}
function rejected(storage, sql, message, onAccepted) {
  let refused = false;
  try { storage.sql.exec(sql).toArray(); } catch { refused = true; }
  if (!refused) onAccepted?.();
  require(refused, message);
  require(scalar(storage, "SELECT 'alive' AS value") === 'alive', 'connection unusable after rejection');
}
function acceptedObservation(storage, name) {
  const observe = sql => {
    try {const value = scalar(storage, sql);return typeof value === 'string' && value.length <= 20 ? value : 'unavailable';}
    catch {return 'unavailable';}
  };
  const count = sql => {const value = observe(sql);return /^(0|[1-9][0-9]{0,19})$/.test(value) ? value : 'unavailable';};
  const result = {
    name,
    main_cookie: count(COOKIE),
    main_count: count("SELECT CAST(count(*) AS TEXT) AS value FROM sqlite_schema WHERE name='foks_connection_probe_qualified'"),
    temp_count: count("SELECT CAST(count(*) AS TEXT) AS value FROM sqlite_temp_schema WHERE name='foks_connection_probe_qualified'"),
  };
  if (name === 'qualified_temp_table' && result.main_count === '0' && result.temp_count === '1') {
    result.shadow = 'unavailable';
    try {
      storage.sql.exec("INSERT INTO temp.foks_connection_probe_qualified VALUES('temp-marker')").toArray();
      storage.sql.exec('CREATE TABLE main.foks_connection_probe_qualified(value)').toArray();
      storage.sql.exec("INSERT INTO main.foks_connection_probe_qualified VALUES('main-marker')").toArray();
      result.temp_value = observe('SELECT value FROM temp.foks_connection_probe_qualified');
      result.main_value = observe('SELECT value FROM main.foks_connection_probe_qualified');
      result.unqualified_value = observe('SELECT value FROM foks_connection_probe_qualified');
      result.shadow = result.temp_value === 'temp-marker' && result.main_value === 'main-marker' && result.unqualified_value === 'temp-marker' ? 'confirmed' : 'not-confirmed';
    } catch { /* Preserve the failed gate; diagnostics grant no permission. */ }
  }
  return result;
}
export function runProbe(storage) {
  let stage = 'initial';
  const denied = [];
  let initial, accepted_observation;
  try {
    const before = cookie(storage), objects = inventory(storage);
    require(scalar(storage, PREFIX_COUNT) === '0', 'fixture objects already exist');
    initial = settings(storage);
    stage = 'configure';
    for (let i = 0; i < SETTINGS.length; i++) storage.sql.exec('PRAGMA ' + SETTINGS[i] + '=' + EXPECTED[i]).toArray();
    require(settings(storage).every((value,i) => value === EXPECTED[i]), 'settings not established');
    require(cookie(storage) === before, 'configuration changed main cookie');
    stage = 'behavior';
    const rollback = new Error('owned connection probe rollback');
    try {
      storage.transactionSync(() => {
        require(settings(storage).every((value,i) => value === EXPECTED[i]), 'transaction settings differ');
        for (const sql of CREATE) storage.sql.exec(sql).toArray();
        storage.sql.exec('INSERT INTO foks_connection_probe_parent VALUES(1)').toArray();
        storage.sql.exec('INSERT INTO foks_connection_probe_child VALUES(1)').toArray();
        rejected(storage, 'INSERT INTO foks_connection_probe_child VALUES(2)', 'foreign key not enforced');
        require(scalar(storage, 'SELECT CAST(count(*) AS TEXT) AS value FROM foks_connection_probe_child') === '1', 'failed foreign key insert survived');
        storage.sql.exec('INSERT INTO foks_connection_probe_check VALUES(1)').toArray();
        rejected(storage, 'INSERT INTO foks_connection_probe_check VALUES(0)', 'CHECK not enforced');
        require(scalar(storage, 'SELECT CAST(count(*) AS TEXT) AS value FROM foks_connection_probe_check') === '1', 'failed CHECK insert survived');
        storage.sql.exec('INSERT INTO foks_connection_probe_recursive VALUES(1,0)').toArray();
        storage.sql.exec('UPDATE foks_connection_probe_recursive SET value=1 WHERE id=1').toArray();
        require(scalar(storage, 'SELECT CAST(value AS TEXT) AS value FROM foks_connection_probe_recursive WHERE id=1') === '3', 'recursive trigger behavior differs');
        stage = 'name-resolution';
        const fixtureCookie = cookie(storage);
        for (const [name, sql] of DENIED) {
          rejected(storage, sql, 'forbidden operation accepted: ' + name, () => {accepted_observation = acceptedObservation(storage, name);});
          require(cookie(storage) === fixtureCookie, 'rejected operation changed main cookie');
          denied.push(name);
        }
        throw rollback;
      });
      throw new Error('rollback returned normally');
    } catch (error) { if (error !== rollback) throw error; }
    stage = 'restored';
    require(cookie(storage) === before && inventory(storage) === objects, 'rollback did not restore main schema');
    require(scalar(storage, PREFIX_COUNT) === '0', 'fixture objects survived');
    const final = settings(storage);
    require(final.every((value,i) => value === EXPECTED[i]), 'final settings differ');
    return {passed:true, mode:'configuration', initial_settings:initial, settings:final, denied, final_table_absent:true, rollback_restored:true, trusted_schema:'unqualified', production_admission:false};
  } catch (error) {
    return {passed:false, mode:'configuration', stage, initial_settings:initial, denied, accepted_observation, error:String(error?.message ?? error).slice(0,300), trusted_schema:'unqualified', production_admission:false};
  }
}
