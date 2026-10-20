// Fixed private qualification SQL, never a caller-selected query interface.
export const COOKIE_SQL = 'SELECT CAST(schema_version AS TEXT) FROM pragma_schema_version';
const TABLE = 'foks_cookie_probe_only';
const CREATE = `CREATE TABLE ${TABLE}(value INTEGER NOT NULL)`;
const DROP = `DROP TABLE ${TABLE}`;
const PRESENT = `SELECT CAST(count(*) AS TEXT) AS present FROM sqlite_schema WHERE name='${TABLE}'`;
const MAX_COOKIE = 0xffff_ffffn;

function rows(storage, sql) {
  const result = storage.sql.exec(sql).toArray();
  if (result.length !== 1 || !result[0] || typeof result[0] !== 'object' || Object.keys(result[0]).length !== 1) {
    throw new Error('expected exactly one row and column');
  }
  return Object.values(result[0])[0];
}
function decimal(value) {
  if (typeof value !== 'string' || !/^(0|[1-9][0-9]{0,9})$/.test(value)) throw new Error('noncanonical cookie TEXT');
  const number = BigInt(value);
  if (number > MAX_COOKIE) throw new Error('cookie exceeds uint32');
  return number;
}
function read(storage, phases, phase) {
  const value = rows(storage, COOKIE_SQL);
  const checked = decimal(value);
  phases.push({ phase, type: typeof value, value: checked.toString() });
  return checked;
}
function present(storage) {
  const count = rows(storage, PRESENT);
  if (count !== '0' && count !== '1') throw new Error('invalid fixture table count');
  return count === '1';
}
function require(condition, message) {
  if (!condition) throw new Error(message);
}

export function runProbe(storage) {
  const phases = [];
  let stage = 'initial';
  try {
    const initial = storage.transactionSync(() => {
      const first = read(storage, phases, 'initial-first');
      const second = read(storage, phases, 'initial-second');
      require(first === second, 'repeated cookie changed');
      require(!present(storage), 'fixture table already exists');
      return first;
    });
    stage = 'rollback';
    const rollback = new Error('private fixture rollback sentinel');
    try {
      storage.transactionSync(() => {
        require(read(storage, phases, 'rollback-before') === initial, 'unexpected initial cookie');
        storage.sql.exec(CREATE);
        require(present(storage), 'created table absent');
        require(read(storage, phases, 'rollback-during') !== initial, 'DDL did not change cookie');
        throw rollback;
      });
      throw new Error('rollback transaction returned normally');
    } catch (error) {
      if (error !== rollback) throw error;
    }
    storage.transactionSync(() => {
      require(read(storage, phases, 'rollback-after') === initial, 'rollback cookie not restored');
      require(!present(storage), 'rollback table survived');
    });
    stage = 'commit-create';
    const created = storage.transactionSync(() => {
      const before = read(storage, phases, 'create-before');
      storage.sql.exec(CREATE);
      const after = read(storage, phases, 'create-during');
      require(after !== before && present(storage), 'create did not update schema');
      return after;
    });
    storage.transactionSync(() => {
      require(read(storage, phases, 'create-after') === created && present(storage), 'create commit not retained');
      require(read(storage, phases, 'create-repeat') === created, 'committed cookie not stable');
    });
    stage = 'commit-drop';
    const dropped = storage.transactionSync(() => {
      storage.sql.exec(DROP);
      const after = read(storage, phases, 'drop-during');
      require(after !== created && !present(storage), 'drop did not update schema');
      return after;
    });
    storage.transactionSync(() => {
      require(read(storage, phases, 'drop-after') === dropped && !present(storage), 'drop commit not retained');
    });
    return { passed: true, candidate: COOKIE_SQL, phases, relation: 'stable/rollback-restored/committed-create-and-drop-change', final_table_absent: true };
  } catch (error) {
    // Diagnostic only: never promote a denied candidate into a successful probe.
    let diagnostic;
    try {
      const value = storage.transactionSync(() => rows(storage, 'PRAGMA schema_version'));
      if (typeof value !== 'number' || !Number.isInteger(value) || value < 0 || value > Number(MAX_COOKIE)) throw new Error('invalid PRAGMA uint32');
      diagnostic = { allowed: true, type: typeof value, value: BigInt(value).toString() };
    } catch (failure) {
      diagnostic = { allowed: false, error: String(failure?.message ?? failure).slice(0, 300) };
    }
    return { passed: false, candidate: COOKIE_SQL, stage, phases, error: String(error?.message ?? error).slice(0, 300), pragma_diagnostic: diagnostic };
  }
}
