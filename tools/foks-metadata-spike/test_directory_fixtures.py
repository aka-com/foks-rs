import json,pathlib,tempfile,unittest
from directory_fixtures import NAMES,prepare,verify
class DirectoryFixtures(unittest.TestCase):
    def setup_case(self):
        tmp=tempfile.TemporaryDirectory();self.addCleanup(tmp.cleanup);base=pathlib.Path(tmp.name);source=base/'source';source.mkdir()
        vectors={'vectors':[{'name':n,'file':n+'.bin','command':'00','calls':1,'outcome':'ok','reply':'00'} for n in NAMES]}
        (source/'directory-vectors.json').write_text(json.dumps(vectors));(source/'page-catalog.json').write_text('{}')
        for n in NAMES:(source/(n+'.bin')).write_bytes(b'abc')
        return base,source
    def run_prepare(self,base,source):return prepare(source,base/'copy',source/'directory-vectors.json',source/'page-catalog.json')
    def test_copy_and_post_run_mutation(self):
        base,source=self.setup_case();fixture,payloads,inputs=self.run_prepare(base,source);self.assertEqual(len(payloads),8);verify(inputs)
        self.assertEqual(fixture['vectors'][0]['length'],3)
        payloads[0].write_bytes(b'bad')
        with self.assertRaises(ValueError):verify(inputs)
    def test_missing_unexpected_symlink_oversize_and_case_set(self):
        for kind in ['missing','unexpected','symlink','oversize','case']:
            with self.subTest(kind=kind):
                base,source=self.setup_case();first=source/'small.bin'
                if kind=='missing':first.unlink()
                elif kind=='unexpected':(source/'extra.bin').write_bytes(b'1')
                elif kind=='symlink':first.unlink();first.symlink_to(source/'over64k.bin')
                elif kind=='oversize':
                    with first.open('wb') as f:f.truncate(16*1024*1024+1)
                else:
                    p=source/'directory-vectors.json';v=json.loads(p.read_text());v['vectors'].pop();p.write_text(json.dumps(v))
                with self.assertRaises((ValueError,FileNotFoundError)):self.run_prepare(base,source)
    def test_native_vector_and_catalog_must_match(self):
        for artifact in ['vectors','catalog']:
            base,source=self.setup_case();other=base/'other';other.write_text('{}x')
            with self.assertRaises(ValueError):prepare(source,base/'copy',other if artifact=='vectors' else source/'directory-vectors.json',other if artifact=='catalog' else source/'page-catalog.json')
if __name__=='__main__':unittest.main()
