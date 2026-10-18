"""Bounded immutable schema scripts; expected definitions predate cell setup."""
import hashlib,json,pathlib,re,struct
from directory_fixtures import regular,verify
MAXIMUM=8*1024*1024
NAMES=('full_15','full_1','suffix_0','suffix_1','suffix_15','suffix_16','unicode','name_256','name_257','sql_65536','sql_65537','sqliteX','missing_foks','changed_foks','missing_authority','changed_authority','platform_missing','platform_changed','platform_extra')
def keys(value,expected):
    if not isinstance(value,dict) or set(value)!=set(expected):raise ValueError('schema fixture keys')
def bounded_integer(value,lo,hi):
    if type(value) is not int or not lo<=value<=hi:raise ValueError('schema fixture integer')
def objects(values):
    if not isinstance(values,list) or not 1<=len(values)<=1024:raise ValueError('schema fixture objects')
    previous=None;total=0
    for value in values:
        keys(value,('kind','name','table','sql'))
        for field,cap in zip(('kind','name','table','sql'),(8,257,257,65537)):
            if not isinstance(value[field],str) or not 0<len(value[field].encode())<=cap:raise ValueError('schema fixture object field')
        if value['kind'] not in ('table','index','trigger','view'):raise ValueError('schema fixture object kind')
        key=(value['kind'].encode(),value['name'].encode())
        if previous is not None and previous>=key:raise ValueError('schema fixture object order')
        previous=key;total+=len(value['sql'].encode())
    if total>2*1024*1024:raise ValueError('schema fixture SQL total')
def wire(text,kind,maximum):
    if not isinstance(text,str) or len(text)>2*maximum or not re.fullmatch(r'(?:[0-9a-f]{2}){24,}',text):raise ValueError('schema fixture hex')
    raw=bytes.fromhex(text)
    magic,abi,tag,statement,rows,columns,body,reserved=struct.unpack('<4sHHIHHII',raw[:24])
    if (magic,abi,tag,statement,reserved)!=(b'FKPG',1,kind,9,0) or body!=len(raw)-24 or columns!=(4 if kind==1 else 12):raise ValueError('schema fixture header')
    if (kind==1 and rows!=1) or not 0<=rows<=15:raise ValueError('schema fixture row count')
    return rows

def validate(fixture):
    keys(fixture,('schema_version','platform_sha256','catalog_sha256','page_catalog_sha256','seed','platform','baseline','coordinate','vectors'))
    if type(fixture['schema_version']) is not int or fixture['schema_version']!=60:raise ValueError('schema fixture version')
    for name in ('platform_sha256','catalog_sha256','page_catalog_sha256'):
        if not isinstance(fixture[name],str) or not re.fullmatch('[0-9a-f]{64}',fixture[name]):raise ValueError('schema fixture digest')
    objects(fixture['seed']);objects(fixture['platform']);objects(fixture['baseline'])
    if sorted(fixture['baseline'],key=lambda o:(o['kind'],o['name']))!=sorted(fixture['seed']+fixture['platform'],key=lambda o:(o['kind'],o['name'])):raise ValueError('schema fixture baseline union')
    if len(fixture['platform'])!=10:raise ValueError('schema fixture platform count')
    coordinate=fixture['coordinate'];keys(coordinate,('incarnation','revision'))
    if not isinstance(coordinate['incarnation'],list) or len(coordinate['incarnation'])!=32:raise ValueError('schema fixture coordinate')
    for b in coordinate['incarnation']:bounded_integer(b,0,255)
    if not any(coordinate['incarnation']) or not isinstance(coordinate['revision'],str) or not re.fullmatch('[1-9][0-9]{0,18}',coordinate['revision']) or int(coordinate['revision'])>2**63-1:raise ValueError('schema fixture coordinate')
    if not isinstance(fixture['vectors'],list) or any(not isinstance(v,dict) for v in fixture['vectors']) or [v.get('name') for v in fixture['vectors']]!=list(NAMES):raise ValueError('schema fixture cases')
    for case in fixture['vectors']:
        keys(case,('name','limit','start','pages','observed','union_matches','complete'))
        if case['observed'] is not None:objects(case['observed'])
        observed=case['observed'] if case['observed'] is not None else fixture['baseline']
        expected=[dict(o) for o in fixture['baseline']]
        if case['name']=='platform_missing':expected=[o for o in expected if o['name']!='_cf_WAKE_update']
        elif case['name']=='platform_changed':
            item=next(o for o in expected if o['name']=='_cf_WAKE_update');item['sql']=item['sql'].replace('sequence=sequence+1','sequence=sequence+2')
        elif case['name']=='platform_extra':expected.append({'kind':'table','name':'_cf_fixture_unknown','table':'_cf_fixture_unknown','sql':'CREATE TABLE _cf_fixture_unknown(v)'})
        expected.sort(key=lambda o:(o['kind'].encode(),o['name'].encode()))
        if case['limit'] not in (1,15) or type(case['limit']) is not int:raise ValueError('schema fixture page limit')
        for name in ('union_matches','complete'):
            if type(case[name]) is not bool:raise ValueError('schema fixture bool')
        if case['union_matches']!=(observed==expected) or case['union_matches']!=(case['name'].startswith('full_') or case['name'].startswith('suffix_')):raise ValueError('schema fixture union oracle')
        if case['complete']!=(case['name'] not in ('name_257','sql_65537')):raise ValueError('schema fixture completion oracle')
        if case['pages'] is None and not case['name'].startswith('platform_'):raise ValueError('schema fixture page reference')
        pages=case['pages'] if case['pages'] is not None else fixture['vectors'][0]['pages']
        if not isinstance(pages,list) or not 1<=len(pages)<=1025:raise ValueError('schema fixture pages')
        def cursor(value):
            if value is not None and (not isinstance(value,list) or len(value)!=2 or value[0] not in ('table','index','trigger','view') or not isinstance(value[1],str) or not 0<len(value[1].encode())<=256):raise ValueError('schema fixture cursor')
        cursor(case['start']);total=0
        for page in pages:
            keys(page,('command','reply','rows','last','valid','sql_bytes'))
            wire(page['command'],1,65536)
            if wire(page['reply'],2,1048576)!=page['rows']:raise ValueError('schema fixture row metadata')
            bounded_integer(page['rows'],0,case['limit']);bounded_integer(page['sql_bytes'],0,2*1024*1024)
            if type(page['valid']) is not bool:raise ValueError('schema fixture page bool')
            cursor(page['last']);total+=page['rows']
        if total>1024:raise ValueError('schema fixture row total')
    return fixture

def prepare(source,destination,vectors,catalog,page_catalog,platform):
    if source.is_symlink():raise ValueError('schema fixture directory symlink')
    source=source.resolve(strict=True)
    if not source.is_dir() or {p.name for p in source.iterdir()}!={'vectors.json','catalog.json','page-catalog.json'}:raise ValueError('schema fixture files')
    raw=regular(source/'vectors.json',MAXIMUM)
    if raw!=regular(vectors,MAXIMUM):raise ValueError('schema fixture vector mismatch')
    fixture=validate(json.loads(raw))
    for name,source_path,given,digest in [('catalog.json',source/'catalog.json',catalog,fixture['catalog_sha256']),('page-catalog.json',source/'page-catalog.json',page_catalog,fixture['page_catalog_sha256'])]:
        data=regular(source_path,65536)
        if data!=regular(given,65536) or hashlib.sha256(data).hexdigest()!=digest:raise ValueError('schema fixture catalog mismatch')
    platform_bytes=regular(platform,65536)
    if hashlib.sha256(platform_bytes).hexdigest()!=fixture['platform_sha256']:raise ValueError('schema fixture platform hash')
    expected_platform=json.loads(platform_bytes)
    if sorted((o['kind'],o['sql']) for o in fixture['platform'])!=sorted((o['kind'],o['sql']) for o in expected_platform):raise ValueError('schema fixture platform definitions')
    destination.mkdir(mode=0o700,exist_ok=False);inputs={}
    for path in (source/'vectors.json',source/'catalog.json',source/'page-catalog.json',platform):
        data=regular(path,MAXIMUM);copy=destination/path.name;copy.write_bytes(data)
        if copy.read_bytes()!=data:raise ValueError('schema fixture copy')
        inputs[str(path)]={'sha256':hashlib.sha256(data).hexdigest(),'copy':str(copy),'copy_sha256':hashlib.sha256(copy.read_bytes()).hexdigest()}
    return fixture,inputs
