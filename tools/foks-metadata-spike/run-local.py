#!/usr/bin/env python3
"""Bounded real-celld M0 equivalence gate; no production/AZ/fencing claim."""
import argparse,hashlib,json,os,pathlib,secrets,shutil,socket,subprocess,time,urllib.request,urllib.error
import boto3
from botocore.config import Config
from wasm_limits import memory_limits

p=argparse.ArgumentParser()
for name in ['celld','moto','esbuild','wasm','vectors','catalog','output']:p.add_argument('--'+name,type=pathlib.Path,required=True)
p.add_argument('--page-catalog',type=pathlib.Path)
a=p.parse_args()
for item in [a.celld,a.moto,a.esbuild,a.wasm,a.vectors,a.catalog]:
    if not item.is_file():p.error('missing input '+str(item))
if a.wasm.stat().st_size>32*1024*1024:p.error('wasm artifact bound')
if a.page_catalog is not None and (not a.page_catalog.is_file() or a.page_catalog.stat().st_size>65536):p.error('page catalog artifact bound')
if any(v.get('page') is True for v in json.loads(a.vectors.read_text())['vectors']) and a.page_catalog is None:p.error('page vectors require exact page catalog')
declared_memory=memory_limits(a.wasm.read_bytes())
a.output.mkdir(mode=0o700,exist_ok=False);root=a.output.resolve();source=pathlib.Path(__file__).resolve().parent
children=[];logs=[];results=[];passed=False
env={'PATH':os.environ.get('PATH','/usr/bin:/bin'),'TMPDIR':str(root),'AWS_ACCESS_KEY_ID':'local-m0','AWS_SECRET_ACCESS_KEY':'local-m0-dummy-secret','AWS_DEFAULT_REGION':'us-east-1','AWS_REGION':'us-east-1','AWS_CONFIG_FILE':'/dev/null','AWS_SHARED_CREDENTIALS_FILE':'/dev/null','AWS_EC2_METADATA_DISABLED':'true','CELLD_OTEL':'0','NO_COLOR':'1'}
def port():
    with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]
def start(command,name,extra=None):
    log=(root/(name+'.log')).open('x');logs.append(log)
    child=subprocess.Popen(command,cwd=root,env=dict(env,**(extra or {})),stdout=log,stderr=subprocess.STDOUT);children.append(child);return child
try:
    endpoint='http://127.0.0.1:'+str(port())
    moto=start([str(a.moto.resolve()),'-H','127.0.0.1','-p',endpoint.rsplit(':',1)[1]],'moto')
    s3=boto3.client('s3',endpoint_url=endpoint,region_name='us-east-1',aws_access_key_id=env['AWS_ACCESS_KEY_ID'],aws_secret_access_key=env['AWS_SECRET_ACCESS_KEY'],config=Config(retries={'max_attempts':0},connect_timeout=1,read_timeout=2))
    until=time.monotonic()+20
    while True:
        try:s3.list_buckets();break
        except Exception:
            if moto.poll() is not None or time.monotonic()>until:raise
            time.sleep(.1)
    bucket='foks-metadata-spike';s3.create_bucket(Bucket=bucket)
    project=root/'worker';project.mkdir(mode=0o700)
    for name in ['host.mjs','worker.mjs','durability.mjs','page-codec.mjs']:shutil.copyfile(source/name,project/name)
    for original,name in [(a.wasm,'kernel.wasm'),(a.vectors,'vectors.json'),(a.catalog,'catalog.json')]:shutil.copyfile(original,project/name)
    if a.page_catalog is None:(project/'page-catalog.json').write_text('null')
    else:shutil.copyfile(a.page_catalog,project/'page-catalog.json')
    token=secrets.token_urlsafe(32)
    control_token=secrets.token_urlsafe(32);frontend_token=secrets.token_urlsafe(32)
    config={'name':'foks-metadata-spike','main':'worker.mjs','compatibility_date':'2026-01-01','vars':{'M0_TOKEN':token,'CONTROL_TOKEN':control_token,'FRONTEND_TOKEN':frontend_token},'durable_objects':{'bindings':[{'name':'FOKS_M0','class_name':'FoksMetadata'}]},'migrations':[{'tag':'v1','new_sqlite_classes':['FoksMetadata']}]}
    (project/'wrangler.jsonc').write_text(json.dumps(config));(project/'wrangler.jsonc').chmod(0o600)
    common=['--bucket','s3://'+bucket,'--endpoint',endpoint,'--region','us-east-1']
    deployment=subprocess.run([str(a.celld.resolve()),'deploy',str(project)]+common,cwd=root,env=dict(env,CELLD_ESBUILD=str(a.esbuild.resolve())),capture_output=True,text=True,timeout=45)
    (root/'deploy.log').write_text((deployment.stdout+deployment.stderr).replace(token,'[redacted]').replace(control_token,'[redacted]').replace(frontend_token,'[redacted]'))
    if deployment.returncode:raise RuntimeError('deployment failed')
    public,internal=port(),port();watch=root/'node';watch.mkdir()
    node=start([str(a.celld.resolve())]+common+['--listen',f'127.0.0.1:{public}','--internal-listen',f'127.0.0.1:{internal}','--advertise',f'127.0.0.1:{internal}'],'celld',{'CELLD_NODE':'metadata-m0','CELLD_WATCH':str(watch),'CELLD_READY_FLEET_GATE_MS':'0','CELLD_DURABILITY':'bucket','CELLD_MAX_RESIDENT_CELLS':'32','CELLD_V8_HEAP_LIMIT_MB':'64','CELLD_TOKIO_THREADS':'2'})
    until=time.monotonic()+30
    while True:
        try:
            with urllib.request.urlopen(f'http://127.0.0.1:{public}/.well-known/celld/health',timeout=1) as response:json.load(response)
            break
        except (OSError,ValueError):
            if node.poll() is not None or time.monotonic()>until:raise
            time.sleep(.1)
    vectors=json.loads(a.vectors.read_text())['vectors']
    for index,vector in enumerate(vectors):
        selected=token
        wrong_rejected=None
        if vector.get('authority') is True or vector.get('personal'):
            selected=control_token if not vector.get('personal') and json.loads(vector['command'])['caller']['kind']=='controller' else frontend_token
            wrong=frontend_token if selected==control_token else control_token
            denied=urllib.request.Request(f'http://127.0.0.1:{public}/{index}',data=b'',headers={'authorization':'Bearer '+wrong})
            try:
                with urllib.request.urlopen(denied,timeout=15) as response:wrong_rejected=response.status==403
            except urllib.error.HTTPError as error:
                wrong_rejected=error.code==403;error.close()
            if not wrong_rejected:raise RuntimeError('private capability separation failed')
        request=urllib.request.Request(f'http://127.0.0.1:{public}/{index}',data=b'',headers={'authorization':'Bearer '+selected})
        began=time.monotonic()
        try:response=urllib.request.urlopen(request,timeout=15)
        except urllib.error.HTTPError as error:response=error
        with response:
            body=response.read(65537)
            if len(body)>65536:raise RuntimeError('result size')
            result=json.loads(body)
        result['elapsed_seconds']=time.monotonic()-began;result['wrong_capability_rejected']=wrong_rejected;results.append(result)
        (root/'results.json').write_text(json.dumps(results,indent=2))
        print(json.dumps(result),flush=True)
    passed=all(r.get('passed') is True for r in results) and len(results)==len(vectors)
finally:
    for child in reversed(children):
        if child.poll() is None:
            child.terminate()
            try:child.wait(timeout=5)
            except subprocess.TimeoutExpired:child.kill();child.wait()
    for log in logs:log.close()
    evidence={'passed':passed,'results':results,'wasm_memory_pages':declared_memory,'scope':'Private native/Wasm SQL qualification on actual initialized schema; per-vector seeded snapshots are fixture setup/assertions, not a persistent HTTP/native adapter; local emulator, not production FOKS authentication, hard deadlines or failover','sha256':{str(item):hashlib.sha256(item.read_bytes()).hexdigest() for item in [a.celld,a.wasm,a.vectors,a.catalog,source/'host.mjs',source/'worker.mjs',source/'durability.mjs',source/'page-codec.mjs',source/'wasm_limits.py',pathlib.Path(__file__),*([a.page_catalog] if a.page_catalog else [])]}}
    (root/'qualification.json').write_text(json.dumps(evidence,indent=2))
raise SystemExit(0 if passed else 1)
