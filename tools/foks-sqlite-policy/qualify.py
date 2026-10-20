#!/usr/bin/env python3
"""Pinned-engine diagnostic. Its output cannot admit a production database."""
import argparse
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
from sqlite_api import Engine, SqlError

PINS = {
    'foks-59.json': '3cc399cd4f787af3324362f83378f6e148e469527fc30624a25f1c2ff571ceb5',
    'authority-v1.json': '14c93336778f8fef6323289edf8e063f4c324ade2051ddc2dc55e93abfeab998',
    'celld-0.6.2.json': 'c6848f846b47a76fa89abdb078bd9bc534e6ba2321391245b5de45c04d8c692f',
    'scalar-catalog.json': 'f148c7aad3859b8bc633c1cfd19adc3700f54f3779f2bddca049d88d46dd3b53',
    'page-catalog.json': 'a2b83eb458c38a39b3245fbcaa669bd9bd85255db0be08c4124813248b4f955b',
}
SOURCE_PIN = '7956a38f236a6be6c0bb30c96ba4f85f19e5a69f6beb6d2c62c9d246972a6775'
BUILD_PIN = 'a3a0c7f409e20573d21448fb53c5a1f168fd04c5983af68671da126cc3bf9ff4'
SOURCE_ID = '2024-01-15 17:01:13 1066602b2b1976fe58b5150777cced894af17c803e068f5918390d6915b46e1d'
DEFINES = ['SQLITE_CORE', 'SQLITE_DEFAULT_FOREIGN_KEYS=1', 'SQLITE_ENABLE_API_ARMOR',
    'SQLITE_ENABLE_COLUMN_METADATA', 'SQLITE_ENABLE_DBSTAT_VTAB', 'SQLITE_ENABLE_FTS3',
    'SQLITE_ENABLE_FTS3_PARENTHESIS', 'SQLITE_ENABLE_FTS5', 'SQLITE_ENABLE_JSON1',
    'SQLITE_ENABLE_LOAD_EXTENSION=1', 'SQLITE_ENABLE_MEMORY_MANAGEMENT', 'SQLITE_ENABLE_RTREE',
    'SQLITE_ENABLE_STAT2', 'SQLITE_ENABLE_STAT4', 'SQLITE_SOUNDEX', 'SQLITE_THREADSAFE=1',
    'SQLITE_USE_URI', 'HAVE_USLEEP=1', '_POSIX_THREAD_SAFE_FUNCTIONS', 'HAVE_ISNAN', 'HAVE_LOCALTIME_R']
LIMITS = [(0,2200000),(1,100000),(2,100),(3,100),(4,5),(5,25000),(6,127),(7,0),(8,50),(9,100),(10,10),(11,0)]
PRAGMA_QUERY = {'id':'scalar/44','sql':'SELECT CAST(schema_version AS TEXT) FROM pragma_schema_version'}
FUNCTION_OPS = {'Function','PureFunc','AggStep','AggStep1','AggInverse','AggValue','AggFinal'}
HOST_PIN = 'da8d96348f693bbcedfc0107b19335e6d7278e846fbbb6cef16c97657f5cd085'
CONTROLS = {'scalar/29':'SAVEPOINT metadata_domain','scalar/30':'ROLLBACK TO metadata_domain','scalar/31':'RELEASE metadata_domain'}
CONTROL_BINDING = 'host-domain-transactionSync-v1'
CONTROL_OUTCOMES = [[['0']],[[1]],[[1]],[[1],[3]],[['0']]]
ROOTS = "SELECT type,name,rootpage FROM sqlite_schema WHERE rootpage>0 ORDER BY rootpage"
# A closed reviewed builtin set; future schema/catalog functions require review.
FUNCTIONS = {'length','typeof','coalesce','zeroblob','max','substr','min','abs','hex','count','lower','upper','randomblob'}


def encoded(value):
    return json.dumps(value, separators=(',', ':'), ensure_ascii=False).encode()


def digest(value):
    return hashlib.sha256(encoded(value)).hexdigest()


def pinned(path, expected, cap):
    if path.is_symlink() or not path.is_file() or path.stat().st_size > cap:
        raise ValueError('artifact regular-file/size gate')
    data = path.read_bytes()
    if len(data) > cap or hashlib.sha256(data).hexdigest() != expected:
        raise ValueError('artifact checksum gate: ' + path.name)
    return data


def artifacts(source):
    return {name: json.loads(pinned(source / name, sha, 2 * 1024 * 1024)) for name, sha in PINS.items()}


def quote(name):
    if not isinstance(name, str) or not 1 <= len(name.encode()) <= 256 or '\0' in name:
        raise ValueError('trusted identifier bound')
    return '"' + name.replace('"', '""') + '"'


def configure(db, trusted):
    for category, limit in LIMITS:
        db.engine.limit(db.handle, category, limit)
        if db.engine.limit(db.handle, category, -1) != limit:
            raise ValueError('native SQLite limit mismatch')
    for name, value in [('trusted_schema',trusted),('foreign_keys',1),('ignore_check_constraints',0),('recursive_triggers',1)]:
        db.query(f'PRAGMA {name}={value}')
        if db.query('PRAGMA ' + name) != [[value]]:
            raise ValueError('native connection policy mismatch')


def install(db, source):
    platform = source['celld-0.6.2.json']
    app = source['foks-59.json'] + source['authority-v1.json']
    for part in (platform, app):
        for kind in ('table','index','trigger'):
            for row in part:
                if row[0] == kind:
                    db.query(row[3])
    if any(row[0] not in ('table','index','trigger') for row in platform + app):
        raise ValueError('unreviewed trusted object kind')
    if db.query("SELECT count(*) FROM pragma_table_list WHERE type='virtual'") != [[0]]:
        raise ValueError('unexpected virtual table')


def corpus(db, source):
    result = []
    for kind, name, _table, _ddl in source['foks-59.json'] + source['authority-v1.json']:
        if kind != 'table':
            continue
        columns = db.query('PRAGMA table_xinfo(' + quote(name) + ')', maximum_rows=100)
        if not columns or any(row[6] != 0 for row in columns):
            raise ValueError('unsupported hidden/generated column')
        names = [quote(row[1]) for row in columns]
        table = quote(name)
        queries = [('insert', f'INSERT INTO {table}({",".join(names)}) VALUES({",".join("NULL" for _ in names)})'),
                   ('defaults', f'INSERT INTO {table} DEFAULT VALUES'),
                   ('update', f'UPDATE {table} SET ' + ','.join(n+'=NULL' for n in names)),
                   ('delete', f'DELETE FROM {table}')]
        result.extend({'id':'table/'+name+'/'+operation,'sql':sql} for operation, sql in queries)
    for kind, values in [('scalar',source['scalar-catalog.json']),('page',source['page-catalog.json']['descriptor']['statements'])]:
        result.extend({'id':kind+'/'+str(value['id']),'sql':value['sql']} for value in values)
    if len({row['id'] for row in result}) != len(result) or len(result) > 1024:
        raise ValueError('corpus identity/count')
    return result


def root_map(rows):
    result = {1:'table/sqlite_schema'}
    for kind, name, root in rows:
        if kind not in ('table','index') or type(root) is not int or root < 2 or root in result:
            raise ValueError('root-page inventory')
        result[root] = kind + '/' + name
    return result


def normalize(rows, roots, query=None):
    out = []
    virtual = query is not None and all(query.get(k)==v for k,v in PRAGMA_QUERY.items())
    vopen = 0
    for original in rows:
        if len(original) != 8 or not isinstance(original[1], str):
            raise ValueError('EXPLAIN row shape')
        row = list(original)
        if row[1] in ('OpenRead','OpenWrite','ReopenIdx'):
            if row[4] != 0 or row[6] & 0x10 or row[3] not in roots:
                raise ValueError('unqualified root operand')
            row[3] = roots[row[3]]
        if row[1] == 'VOpen':
            if not virtual or not isinstance(row[5],str) or not re.fullmatch(r'vtab:[0-9A-Fa-f]+',row[5]):
                raise ValueError('unqualified virtual-table operand')
            vopen += 1
            row[5] = 'vtab/builtin/pragma_schema_version'
        out.append(row)
    if vopen != (1 if virtual else 0):
        raise ValueError('virtual-table operand count')
    return out


def function_dependencies(rows, authorized):
    names=set(authorized)
    for row in rows:
        if row[1] in FUNCTION_OPS:
            if not isinstance(row[5],str):
                raise ValueError('unqualified function operand')
            match=re.fullmatch(r'([A-Za-z_][A-Za-z_0-9]{0,127})\((-1|[0-9]{1,3})\)', row[5] or '')
            if match is None or int(match[2])>127:
                raise ValueError('unqualified function operand')
            names.add(match[1].lower())
    return sorted(names)


def validate_collations(rows):
    # Preserve all P4 data. NOCASE is an explicit SchemaObjects query dependency;
    # it does not widen the trusted table/index BINARY key layout.
    for row in rows:
        if row[1] in ('CollSeq','Eq','Ne','Lt','Le','Gt','Ge') and row[5] not in (None,'BINARY-8','NOCASE-8'):
            raise ValueError('unreviewed collation')
        if isinstance(row[5],str) and row[5].startswith('k('):
            if not re.fullmatch(r'k\([0-9]+(?:,-?(?:N\.)?B?)*\)', row[5]):
                raise ValueError('unreviewed KeyInfo collation: ' + row[5])


def compile_programs(db, queries, result):
    roots = root_map(db.query(ROOTS, maximum_rows=1024))
    total_bytes = 0
    for query in queries:
        db.functions.clear()
        try:
            rows = db.query('EXPLAIN ' + query['sql'])
        except (SqlError,ValueError) as error:
            raise ValueError('prepare '+query['id']+': '+str(error)) from error
        functions = function_dependencies(rows,db.functions)
        entry={**query,'parameter_count':db.last_parameter_count,'rows':rows,'functions':functions}
        total_bytes += len(encoded(entry))
        if total_bytes>32*1024*1024: raise ValueError('diagnostic full-program byte bound')
        result.append(entry)
        if set(functions) - FUNCTIONS:
            raise ValueError('unreviewed function: ' + str(set(functions) - FUNCTIONS))
        validate_collations(rows)
        entry['normalized_sha256']=digest(normalize(rows,roots,query))
    return result


def behavior(db):
    before = db.query('SELECT CAST(count(*) AS TEXT) FROM metadata_read_dependency')
    if before != [['0']]:
        raise ValueError('behavior fixture requires fresh database')
    statements = [
        ('ok', "INSERT INTO metadata_read_dependency VALUES(1,x'0101010101010101010101010101010101010101010101010101010101010101',1)"),
        ('ok', "INSERT INTO metadata_principals VALUES(zeroblob(32),zeroblob(33),x'01',1)"),
        ('reject', "INSERT INTO metadata_principals VALUES(x'02',zeroblob(33),x'01',1)"),
        ('ok', "INSERT INTO metadata_authority VALUES(1,'{}','{}',zeroblob(16),1,1,0,0,0,4,18432,0)"),
        ('ok', "INSERT INTO metadata_intents VALUES(1,1,zeroblob(32),zeroblob(32),zeroblob(33),1,1,0,NULL)"),
        ('read', 'SELECT retained_rows,reserved_bytes FROM metadata_authority'),
        ('ok', 'DELETE FROM metadata_intents'),
        ('read', 'SELECT retained_rows,reserved_bytes FROM metadata_authority'),
        ('reject', "INSERT INTO kv_directory_heads VALUES(zeroblob(33),zeroblob(16),1)"),
        ('read','SELECT revision FROM metadata_read_dependency'),
        ('ok', "INSERT INTO kv_namespaces(namespace_id,party_kind,host_id) VALUES(zeroblob(33),1,zeroblob(33))"),
        ('read','SELECT revision FROM metadata_read_dependency'),
        ('ok', 'DELETE FROM kv_namespaces'),
        ('read','SELECT revision FROM metadata_read_dependency'),
        ('ok','CREATE TABLE policy_recursive(id INTEGER PRIMARY KEY,n INTEGER)'),
        ('ok','CREATE TRIGGER policy_step AFTER UPDATE ON policy_recursive WHEN NEW.n<3 BEGIN UPDATE policy_recursive SET n=NEW.n+1 WHERE id=NEW.id; END'),
        ('ok','INSERT INTO policy_recursive VALUES(1,0)'),
        ('ok','UPDATE policy_recursive SET n=1 WHERE id=1'),
        ('read','SELECT n FROM policy_recursive'),
    ]
    result = []
    db.query('BEGIN')
    try:
        for expected, sql in statements:
            try:
                rows = db.query(sql)
            except SqlError as error:
                if expected != 'reject' or error.code & 0xff != 19:
                    raise
                category = 'check' if 'CHECK constraint failed' in str(error) else 'foreign_key' if 'FOREIGN KEY constraint failed' in str(error) else None
                if category is None: raise
                result.append(['reject',category])
            else:
                if expected == 'reject':
                    raise ValueError('behavior expected rejection')
                result.append([expected,rows])
    finally:
        db.query('ROLLBACK')
    if db.query('SELECT CAST(count(*) AS TEXT) FROM metadata_read_dependency') != before:
        raise ValueError('behavior rollback state')
    expected = [['ok',[]],['ok',[]],['reject','check'],['ok',[]],['ok',[]],['read',[[1,4608]]],['ok',[]],['read',[[0,0]]],['reject','foreign_key'],['read',[[1]]],['ok',[]],['read',[[2]]],['ok',[]],['read',[[3]]],['ok',[]],['ok',[]],['ok',[]],['ok',[]],['read',[[3]]]]
    if result != expected: raise ValueError('behavior differs from independent fixed outcomes')
    return statements,result


def compile_library(source, output):
    pinned(source/'build.rs', BUILD_PIN, 128*1024)
    source_bytes = pinned(source/'sqlite3/sqlite3.c', SOURCE_PIN, 16*1024*1024)
    owned_source = output/'sqlite3.c'
    with owned_source.open('xb') as handle:
        handle.write(source_bytes)
    if hashlib.sha256(owned_source.read_bytes()).hexdigest() != SOURCE_PIN:
        raise ValueError('private SQLite source copy mismatch')
    library = output / ('sqlite.dylib' if sys.platform == 'darwin' else 'sqlite.so')
    command = ['/usr/bin/cc','-O2', '-dynamiclib' if sys.platform == 'darwin' else '-shared', '-fPIC',
               *('-D'+value for value in DEFINES), str(owned_source), '-o',str(library),'-lpthread','-lm']
    done = subprocess.run(command, capture_output=True, timeout=120, env={'PATH':'/usr/bin:/bin','TMPDIR':str(output)})
    (output/'compiler.log').write_bytes(done.stdout+done.stderr)
    if hashlib.sha256(owned_source.read_bytes()).hexdigest() != SOURCE_PIN:
        raise ValueError('private SQLite source changed during compilation')
    if done.returncode:
        raise RuntimeError('pinned SQLite compilation failed')
    return library, command


def execution(query):
    if query['id'] not in CONTROLS:
        return 'sql'
    if query['sql'] != CONTROLS[query['id']] or query['parameter_count'] != 0:
        raise ValueError('mapped control identity')
    return CONTROL_BINDING


def program_order(programs):
    if (any(p.get('execution')!=execution(p) for p in programs) or
        sum(p['execution']==CONTROL_BINDING for p in programs)!=3 or
        {p['id'] for p in programs if p['execution']==CONTROL_BINDING}!=set(CONTROLS)):
        raise ValueError('mapped control inventory')
    # EXPLAIN sees the initial trusted schema before rollback diagnostics change
    # SQLite's connection-local schema generation. Retain original route indices.
    return sorted(enumerate(programs),key=lambda pair:pair[1]['execution']==CONTROL_BINDING)


def qualify(engine, source, runs):
    if engine.libversion() != b'3.45.0' or engine.sourceid().decode() != SOURCE_ID:
        raise ValueError('engine source/version mismatch')
    for trusted in (0,1):
        with engine.connect() as db:
            configure(db,trusted)
            db.functions.clear()
            install(db,source)
            schema_functions=sorted(db.functions)
            if set(schema_functions)-FUNCTIONS:
                raise ValueError('unreviewed schema function')
            queries = corpus(db,source)
            programs=[]
            run={'trusted_schema':trusted,'programs':programs,'schema_functions':schema_functions}
            runs.append(run)
            compile_programs(db,queries,programs)
            used = set(schema_functions) | {name for row in programs for name in row['functions']}
            functions = [row for row in db.query('PRAGMA function_list',maximum_rows=512) if row[0] in used]
            if used - {row[0] for row in functions} or any(row[1] != 1 or not row[5] & 0x200000 for row in functions):
                raise ValueError('non-builtin/non-innocuous dependency')
            options = [row[0] for row in db.query('PRAGMA compile_options',maximum_rows=128)]
            statements, outcomes = behavior(db)
            run.update(functions=functions,compile_options=options,behavior=outcomes)
    def comparable(run):
        return {**{k:v for k,v in run.items() if k not in ('trusted_schema','programs')},
                'programs':[{k:v for k,v in row.items() if k!='rows'} for row in run['programs']]}
    if comparable(runs[0]) != comparable(runs[1]):
        raise ValueError('trusted_schema ON/OFF programs or behavior differ')
    base = runs[0]
    fixture = {'pins':PINS,'platform':source['celld-0.6.2.json'],'app':source['foks-59.json']+source['authority-v1.json'],
               'programs':[{**{k:v for k,v in row.items() if k!='rows'},'execution':execution(row)} for row in base['programs']],
               'control_host_sha256':HOST_PIN,
               'compile_options':base['compile_options'],'behavior':statements,'behavior_expected':base['behavior']}
    return fixture


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('sqlite-source','profile','output'):
        parser.add_argument('--'+name,type=pathlib.Path,required=True)
    args=parser.parse_args()
    source=artifacts(args.profile)
    pinned(pathlib.Path(__file__).resolve().parent.parent/'foks-metadata-spike/host.mjs',HOST_PIN,1024*1024)
    args.output.mkdir(mode=0o700,exist_ok=False)
    report={'passed':False,'production_admission':False,'pins':PINS,'sqlite_source':SOURCE_PIN,'sqlite_build':BUILD_PIN,'control_host_sha256':HOST_PIN,
            'source_sha256':{name:hashlib.sha256((pathlib.Path(__file__).parent/name).read_bytes()).hexdigest() for name in ('qualify.py','sqlite_api.py')}}
    runs=[]
    try:
        library,command=compile_library(args.sqlite_source.resolve(),args.output.resolve())
        report.update(command=command,library_sha256=hashlib.sha256(library.read_bytes()).hexdigest())
        fixture=qualify(Engine(library),source,runs)
        pinned(pathlib.Path(__file__).resolve().parent.parent/'foks-metadata-spike/host.mjs',HOST_PIN,1024*1024)
        raw=encoded(fixture)
        if len(raw)>4*1024*1024:raise ValueError('worker fixture bound')
        (args.output/'fixture.json').write_bytes(raw)
        report.update(passed=True,programs=len(fixture['programs']),fixture_sha256=hashlib.sha256(raw).hexdigest())
    except Exception as error:
        report['error']=str(error)[:400]
    (args.output/'native-programs.json').write_bytes(encoded(runs))
    (args.output/'qualification.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))
    return 0 if report['passed'] else 1


if __name__=='__main__':
    raise SystemExit(main())
