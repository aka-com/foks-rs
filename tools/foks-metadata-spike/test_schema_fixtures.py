import copy,json,os,pathlib,tempfile,unittest
from schema_fixtures import prepare,validate,verify
@unittest.skipUnless(os.environ.get('FOKS_SCHEMA_ENUMERATION_VECTORS'),'set FOKS_SCHEMA_ENUMERATION_VECTORS to the reviewed native schema vectors')
class SchemaFixtures(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source=pathlib.Path(os.environ['FOKS_SCHEMA_ENUMERATION_VECTORS'])
        cls.platform=pathlib.Path(__file__).with_name('celld-directory-schema.json')
        cls.fixture=json.loads((cls.source/'vectors.json').read_text())
    def test_native_scripts_prepare_exact_copy_and_detect_postcopy_change(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture,inputs=prepare(self.source,pathlib.Path(tmp)/'copy',self.source/'vectors.json',self.source/'catalog.json',self.source/'page-catalog.json',self.platform)
            self.assertEqual(fixture,self.fixture);verify(inputs)
            pathlib.Path(next(iter(inputs.values()))['copy']).write_bytes(b'changed')
            with self.assertRaises(ValueError):verify(inputs)
    def test_reject_schema_objects_cases_pages_types_and_unknown_fields(self):
        for kind in ['extra','cases','platform','coordinate','duplicate','page','rows','boolean','oracle']:
            with self.subTest(kind=kind):
                f=copy.deepcopy(self.fixture)
                if kind=='extra':f['unexpected']=True
                elif kind=='cases':f['vectors'].pop()
                elif kind=='platform':f['platform'].pop()
                elif kind=='coordinate':f['coordinate']['revision']='01'
                elif kind=='duplicate':f['seed'].insert(0,f['seed'][0])
                elif kind=='page':f['vectors'][0]['pages'][0]['command']='00'
                elif kind=='rows':f['vectors'][0]['pages'][0]['rows']+=1
                elif kind=='boolean':f['vectors'][0]['complete']=1
                else:f['vectors'][0]['union_matches']=False
                with self.assertRaises(ValueError):validate(f)
    def test_changed_external_artifacts_and_symlink_are_rejected(self):
        for kind in ['vectors','catalog','page','platform','symlink']:
            with self.subTest(kind=kind),tempfile.TemporaryDirectory() as tmp:
                root=pathlib.Path(tmp);other=root/'other';other.write_text('{}')
                source=self.source
                if kind=='symlink':source=root/'link';source.symlink_to(self.source,target_is_directory=True)
                with self.assertRaises(ValueError):prepare(source,root/'copy',other if kind=='vectors' else self.source/'vectors.json',other if kind=='catalog' else self.source/'catalog.json',other if kind=='page' else self.source/'page-catalog.json',other if kind=='platform' else self.platform)
if __name__=='__main__':unittest.main()
