import os
import pathlib
import tempfile
import unittest
from qualify import Engine, SqlError, configure, normalize, root_map, pinned, validate_collations, function_dependencies, PRAGMA_QUERY, execution, CONTROLS, CONTROL_BINDING, program_order


class StructuralTests(unittest.TestCase):
    def test_root_normalization_is_narrow_and_retains_p4(self):
        roots=root_map([['table','a',2],['index','i',3]])
        raw=[[0,'OpenRead',0,2,0,'k(1,B)',2,None],[1,'CollSeq',0,0,0,'BINARY-8',0,None]]
        self.assertEqual(normalize(raw,roots),[[0,'OpenRead',0,'table/a',0,'k(1,B)',2,None],raw[1]])
        self.assertEqual(raw[0][3],2)
        for index,value in [(3,44),(4,1),(6,16)]:
            wrong=[list(raw[0])];wrong[0][index]=value
            with self.assertRaises(ValueError):normalize(wrong,roots)
        with self.assertRaises(ValueError):root_map([['table','a',2],['table','b',2]])

    def test_only_reviewed_query_collations_are_accepted(self):
        for collation in (None,'BINARY-8','NOCASE-8'):
            validate_collations([[0,'Ne',0,0,0,collation,0,None]])
        for opcode,collation in [('Eq','custom-8'),('CollSeq','RTRIM-8'),('OpenRead','k(1,NOCASE)')]:
            with self.assertRaises(ValueError):
                validate_collations([[0,opcode,0,0,0,collation,0,None]])

    def test_function_operands_and_builtin_virtual_identity_are_closed(self):
        self.assertEqual(function_dependencies([[0,'PureFunc',0,0,0,'policy_test(0)',0,None]],{'coalesce'}),['coalesce','policy_test'])
        for p4 in (None,'bad','x(128)','x(-2)','other/name(0)'):
            with self.assertRaises(ValueError):
                function_dependencies([[0,'Function',0,0,0,p4,0,None]],set())
        row=[0,'VOpen',0,0,0,'vtab:ABC123',0,None]
        self.assertEqual(normalize([row],{},PRAGMA_QUERY)[0][5],'vtab/builtin/pragma_schema_version')
        self.assertEqual(row[5],'vtab:ABC123')
        for rows,query in [([row],None),([row],{**PRAGMA_QUERY,'sql':'SELECT 1'}),([row],{**PRAGMA_QUERY,'id':'scalar/45'}),([row,row],PRAGMA_QUERY),([],PRAGMA_QUERY)]:
            with self.assertRaises(ValueError):normalize(rows,{},query)
        with self.assertRaises(ValueError):normalize([[0,'VOpen',0,0,0,'vtab:unknown',0,None]],{},PRAGMA_QUERY)

    def test_only_three_exact_zero_parameter_controls_are_mapped(self):
        for identity,sql in CONTROLS.items():
            query={'id':identity,'sql':sql,'parameter_count':0}
            self.assertEqual(execution(query),CONTROL_BINDING)
            for changed in ({**query,'sql':'SELECT 1'},{**query,'parameter_count':1}):
                with self.assertRaises(ValueError):execution(changed)
        self.assertEqual(execution({'id':'scalar/28','sql':'SELECT 1','parameter_count':0}),'sql')

    def test_program_order_preserves_indices_and_both_partitions(self):
        programs=[]
        for i,(identity,sql) in enumerate(CONTROLS.items()):
            programs.extend([{'id':'query/'+str(i),'sql':'SELECT 1','parameter_count':0,'execution':'sql'},
                             {'id':identity,'sql':sql,'parameter_count':0,'execution':CONTROL_BINDING}])
        self.assertEqual([i for i,_ in program_order(programs)],[0,2,4,1,3,5])
        self.assertEqual([p['id'] for p in programs][1::2],list(CONTROLS))
        for invalid in (programs[:-1],[{**p,'execution':'unknown'} for p in programs],programs+[programs[-1]]):
            with self.assertRaises(ValueError):program_order(invalid)

    def test_inputs_require_exact_bytes_regular_file_and_size(self):
        import hashlib
        with tempfile.TemporaryDirectory() as temporary:
            root=pathlib.Path(temporary);path=root/'input';path.write_bytes(b'trusted')
            expected=hashlib.sha256(b'trusted').hexdigest()
            self.assertEqual(pinned(path,expected,7),b'trusted')
            with self.assertRaises(ValueError):pinned(path,expected,6)
            path.write_bytes(b'changed')
            with self.assertRaises(ValueError):pinned(path,expected,7)
            link=root/'link';link.symlink_to(path)
            with self.assertRaises(ValueError):pinned(link,expected,7)


@unittest.skipUnless(os.environ.get('FOKS_SQLITE_POLICY_LIBRARY'), 'explicit separately built pinned library required')
class EngineTests(unittest.TestCase):
    def setUp(self):
        self.engine=Engine(pathlib.Path(os.environ['FOKS_SQLITE_POLICY_LIBRARY']))
        self.assertEqual(self.engine.libversion(),b'3.45.0')

    def test_statement_finalization_on_bounds_and_prepare_errors(self):
        db=self.engine.connect()
        with self.assertRaises(ValueError):db.query('SELECT 1 UNION ALL SELECT 2',maximum_rows=1)
        with self.assertRaises(ValueError):db.query("SELECT 'abcdef'",maximum_bytes=3)
        with self.assertRaises(ValueError):db.query('SELECT 1; SELECT 2')
        with self.assertRaises(SqlError):db.query('SELECT unknown_column')
        self.assertEqual(db.query('SELECT 7'),[[7]])
        db.close()  # SQLITE_BUSY here would expose an unfinalized statement.
        self.assertFalse(db.handle)

    def test_schema_dependency_shapes_reject_noninnocuous_functions(self):
        cases=[
            (['CREATE TABLE t(v INTEGER CHECK(policy_test()>0))'],'INSERT INTO t VALUES(1)'),
            (['CREATE TABLE t(v INTEGER DEFAULT(policy_test()))'],'INSERT INTO t DEFAULT VALUES'),
            (['CREATE TABLE t(v INTEGER)','CREATE INDEX i ON t(policy_test())'],'INSERT INTO t VALUES(1)'),
            (['CREATE TABLE t(v INTEGER)','CREATE INDEX i ON t(v) WHERE policy_test()>0'],'INSERT INTO t VALUES(1)'),
            (['CREATE TABLE t(v INTEGER)','CREATE TRIGGER tr AFTER UPDATE OF v ON t BEGIN SELECT policy_test(); END'],'UPDATE t SET v=1'),
        ]
        for ddl,query in cases:
            for innocuous in (False,True):
                with self.subTest(ddl=ddl,innocuous=innocuous),self.engine.connect() as db:
                    configure(db,1);db.register_test_function(innocuous)
                    for sql in ddl:db.query(sql)
                    db.functions.clear();program=db.query('EXPLAIN '+query)
                    self.assertIn('policy_test',function_dependencies(program,db.functions))
                    db.query('PRAGMA trusted_schema=OFF')
                    if innocuous:
                        db.query('EXPLAIN '+query)
                    else:
                        with self.assertRaises(SqlError) as error:db.query('EXPLAIN '+query)
                        self.assertIn('unsafe use of policy_test()',str(error.exception))

    def test_foreign_keys_check_and_recursive_policy_have_behavioral_effect(self):
        for trusted in (0,1):
            with self.engine.connect() as db:
                configure(db,trusted)
                for sql in ['CREATE TABLE p(id INTEGER PRIMARY KEY)','CREATE TABLE c(id INTEGER REFERENCES p(id), n INTEGER CHECK(n>0))',
                            'CREATE TABLE r(id INTEGER PRIMARY KEY,n INTEGER)',
                            'CREATE TRIGGER step AFTER UPDATE ON r WHEN NEW.n<3 BEGIN UPDATE r SET n=NEW.n+1 WHERE id=NEW.id; END']:
                    db.query(sql)
                for sql in ['INSERT INTO c VALUES(99,1)','INSERT INTO c VALUES(NULL,0)']:
                    with self.assertRaises(SqlError):db.query(sql)
                self.assertEqual(db.query('SELECT count(*) FROM c'),[[0]])
                db.query('INSERT INTO r VALUES(1,0)');db.query('UPDATE r SET n=1 WHERE id=1')
                self.assertEqual(db.query('SELECT n FROM r'),[[3]])


if __name__=='__main__':unittest.main()
