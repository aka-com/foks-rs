"""Immutable binary fixture preparation; no runtime/production upload API."""
import hashlib,json,pathlib,re,stat
MAXIMUM=16*1024*1024
NAMES=('small','over64k','slice_minus','slice_exact','slice_plus','maximum','malformed_maximum','configured_limit')
def regular(path,maximum):
    mode=path.lstat().st_mode
    if not stat.S_ISREG(mode) or path.is_symlink() or path.stat().st_size>maximum:
        raise ValueError('fixture regular file/size: '+str(path))
    return path.read_bytes()
def prepare(source,destination,vectors,page_catalog):
    if source.is_symlink():raise ValueError('fixture directory symlink')
    source=source.resolve(strict=True)
    if not source.is_dir():raise ValueError('fixture directory')
    raw=regular(source/'directory-vectors.json',1024*1024)
    if raw!=regular(vectors,1024*1024):raise ValueError('fixture vector artifact mismatch')
    page=regular(source/'page-catalog.json',65536)
    if page!=regular(page_catalog,65536):raise ValueError('fixture page artifact mismatch')
    fixture=json.loads(raw)
    cases=fixture.get('vectors',[])
    if [v.get('name') for v in cases]!=list(NAMES):raise ValueError('fixture eight cases')
    allowed={'directory-vectors.json','page-catalog.json',*(n+'.bin' for n in NAMES)}
    if {p.name for p in source.iterdir()}!=allowed:raise ValueError('fixture unexpected/missing file')
    destination.mkdir(mode=0o700,exist_ok=False)
    inputs={};payloads=[]
    def copy(path,name,maximum):
        data=regular(path,maximum);target=destination/name;target.write_bytes(data)
        if target.read_bytes()!=data:raise ValueError('fixture copy mismatch')
        inputs[str(path)]={'sha256':hashlib.sha256(data).hexdigest(),'copy':str(target),'copy_sha256':hashlib.sha256(target.read_bytes()).hexdigest()}
        return data,target
    copy(source/'directory-vectors.json','directory-vectors.json',1024*1024)
    copy(source/'page-catalog.json','page-catalog.json',65536)
    for v in cases:
        if v.get('file')!=v['name']+'.bin' or not re.fullmatch(r'[0-9a-f]{2,131072}',v.get('command','')) or len(v['command'])%2:
            raise ValueError('fixture filename/command')
        if type(v.get('calls')) is not int or not 0<v['calls']<=2048:raise ValueError('fixture calls')
        if v.get('outcome') not in ('ok','corrupt','unsupported_profile'):raise ValueError('fixture outcome')
        reply=v.get('reply')
        if v['outcome']=='ok':
            if not isinstance(reply,str) or not re.fullmatch(r'(?:[0-9a-f]{2}){1,128}',reply):raise ValueError('fixture reply')
        elif reply is not None:raise ValueError('fixture failure reply')
        data,target=copy(source/v['file'],v['file'],MAXIMUM)
        if not data:raise ValueError('fixture empty body')
        v.update(length=len(data),sha256=hashlib.sha256(data).hexdigest())
        payloads.append(target)
    (destination/'directory-manifest.json').write_text(json.dumps(fixture,separators=(',',':')))
    return fixture,payloads,inputs

def verify(inputs):
    for name,item in inputs.items():
        for path,expected in [(pathlib.Path(name),item['sha256']),(pathlib.Path(item['copy']),item['copy_sha256'])]:
            if hashlib.sha256(regular(path,MAXIMUM)).hexdigest()!=expected:raise ValueError('fixture input changed: '+str(path))
