#!/usr/bin/env python3
"""Local-only actual-FOKS celld qualification controller. No AWS access.

Requires the explicitly supplied celld 0.6.2 binary, moto executable, generated
native schema and prebuilt Rust integration-test binary. Uses three independent
celld processes and an in-memory S3 emulator on one physical host. A pass does
not qualify physical-host/AZ loss, production S3, platform security or capacity.
"""
import argparse, concurrent.futures, hashlib, http.server, json, os, pathlib
import secrets, shutil, signal, socket, subprocess, sys, threading, time, urllib.request, urllib.error, urllib.parse
import boto3
from botocore.config import Config
from botocore.exceptions import ClientError
from clock_guard import ClockGuard
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parent.parent/'foks-metadata-spike'))
from wasm_limits import memory_limits

parser=argparse.ArgumentParser()
parser.add_argument('--celld',required=True,type=pathlib.Path)
parser.add_argument('--moto',required=True,type=pathlib.Path)
parser.add_argument('--schema',required=True,type=pathlib.Path)
parser.add_argument('--node-decoder-wasm',required=True,type=pathlib.Path)
parser.add_argument('--test-binary',required=True,type=pathlib.Path)
parser.add_argument('--esbuild',required=True,type=pathlib.Path)
parser.add_argument('--output',required=True,type=pathlib.Path)
parser.add_argument('--fault-mode',choices=['kill','pause-return'],default='kill')
parser.add_argument('--durability',choices=['bucket','fleet'],default='bucket')
parser.add_argument('--lease-ttl-ms',type=int,default=10000,help='celld node lease lifetime; default baseline 10000, alternate qualification 3000')
parser.add_argument('--test-name',default='real_foks_projection_recovers_keys_and_fences_outstanding_former_owner_mutation',choices=['real_foks_projection_recovers_keys_and_fences_outstanding_former_owner_mutation','real_foks_cell_owner_process_loss_preserves_acknowledged_sql_writes','real_foks_multiple_cells_capacity_baseline','official_go_client_uses_celld_authoritative_sql'])
args=parser.parse_args()
if args.fault_mode=='pause-return' and args.test_name!='real_foks_cell_owner_process_loss_preserves_acknowledged_sql_writes':parser.error('pause-return requires the single-owner loss test')
if not 1000<=args.lease_ttl_ms<=60000:parser.error('--lease-ttl-ms must be 1000..60000')
for path in [args.celld,args.moto,args.schema,args.node_decoder_wasm,args.test_binary,args.esbuild]:
    if not path.is_file(): parser.error(f'missing input file: {path}')
if args.node_decoder_wasm.stat().st_size>32*1024*1024:parser.error('decoder artifact exceeds32MiB')
decoder_bytes=args.node_decoder_wasm.read_bytes()
decoder_limits=memory_limits(decoder_bytes)
decoder_sha256=hashlib.sha256(decoder_bytes).hexdigest()
args.output.mkdir(mode=0o700,parents=False,exist_ok=False)
root=args.output.resolve(); source=pathlib.Path(__file__).resolve().parent
shutil.copyfile(__file__,root/"controller-script.py")
shutil.copyfile(source/"clock_guard.py",root/"clock_guard.py")
children=[]; logs=[]; event_lock=threading.Lock(); stopped=threading.Event()
ledger=(root/'controller.jsonl').open('x')
clock_guard=ClockGuard(time.monotonic(),time.time_ns())
def record(event,**fields):
    with event_lock:
        monotonic=time.monotonic();wall_ns=time.time_ns()
        drift=clock_guard.observe(monotonic,wall_ns)
        if event in ['result','qualification_complete']:fields['passed']=fields.get('passed',False) and clock_guard.valid
        row={'event':event,'monotonic':monotonic,'wall_ns':wall_ns,'clock_drift_seconds':drift,'maximum_clock_drift_seconds':clock_guard.maximum_drift_seconds,'timing_valid':clock_guard.valid,**fields}
        ledger.write(json.dumps(row)+'\n');ledger.flush();os.fsync(ledger.fileno())
        print(json.dumps(row),flush=True)
        return row
def port():
    with socket.socket() as sock: sock.bind(('127.0.0.1',0));return sock.getsockname()[1]
def start(command,name,env):
    log=(root/(name+'.log')).open('x');logs.append(log)
    child=subprocess.Popen(command,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT);children.append(child);return child
def get_json(url,timeout=2):
    with urllib.request.urlopen(url,timeout=timeout) as response:return json.load(response)
def ready(url,process):
    deadline=time.monotonic()+30
    while time.monotonic()<deadline:
        if process.poll() is not None:raise RuntimeError('process exited during readiness')
        try:return get_json(url,1)
        except (OSError,ValueError):time.sleep(.1)
    raise TimeoutError('local process readiness')
def marker(name,test,seconds=60):
    deadline=time.monotonic()+seconds
    while not (root/'fault'/name).exists():
        if test.poll() is not None:raise RuntimeError('test exited before '+name)
        if time.monotonic()>deadline:raise TimeoutError('test marker '+name)
        time.sleep(.05)

# Route only private proof HTTP. No emulated public FOKS transport is involved;
# actual protocol requests run through the Rust TCP/mTLS listener in the test.
route_lock=threading.Lock(); route=[0]
class Proxy(http.server.BaseHTTPRequestHandler):
    protocol_version='HTTP/1.1'
    def log_message(self,*_):pass
    def do_POST(self):
        self.connection.settimeout(5)
        try:
            length=int(self.headers.get('content-length','-1'))
            if not 0<=length<=8*1024*1024:raise ValueError('body limit')
            body=self.rfile.read(length)
            if len(body)!=length:raise ValueError('short body')
            with route_lock:target=route[0]
            request=urllib.request.Request(f'http://127.0.0.1:{target}'+self.path,data=body,headers={'content-type':'application/json','authorization':self.headers.get('authorization','')})
            try:
                response=urllib.request.urlopen(request,timeout=4)
            except urllib.error.HTTPError as error:response=error
            with response:
                status=response.code;result=response.read(8*1024*1024+1)
            if len(result)>8*1024*1024:raise ValueError('response limit')
        except (OSError,ValueError):status=502;result=b'local proof upstream unavailable'
        if status>=400:
            operation=urllib.parse.urlsplit(self.path).path
            if operation not in ['/initialize','/claim','/snapshot','/apply','/prove']:operation='unknown'
            record('proof_http_failure',status=status,operation=operation)
        try:
            self.send_response(status);self.send_header('content-type','application/json');self.send_header('content-length',str(len(result)));self.end_headers();self.wfile.write(result)
        except OSError:pass

passed=False;proxy=None;sampler=None
# Deliberately exclude the operator's AWS profiles/credentials/proxy settings.
env={'PATH':os.environ.get('PATH','/usr/bin:/bin'),'TMPDIR':str(root),'AWS_ACCESS_KEY_ID':'foks-local-dummy','AWS_SECRET_ACCESS_KEY':'foks-local-dummy-secret','AWS_REGION':'us-east-1','AWS_DEFAULT_REGION':'us-east-1','AWS_EC2_METADATA_DISABLED':'true','AWS_CONFIG_FILE':'/dev/null','AWS_SHARED_CREDENTIALS_FILE':'/dev/null','CELLD_OTEL':'0','NO_COLOR':'1'}
try:
    mp=port();s3endpoint=f'http://127.0.0.1:{mp}'
    moto=start([str(args.moto.resolve()),'-H','127.0.0.1','-p',str(mp)],'moto',env)
    s3=boto3.client('s3',endpoint_url=s3endpoint,region_name='us-east-1',aws_access_key_id='foks-local-dummy',aws_secret_access_key='foks-local-dummy-secret',config=Config(retries={'max_attempts':0},connect_timeout=1,read_timeout=3))
    deadline=time.monotonic()+20
    while True:
        try:s3.list_buckets();break
        except Exception:
            if time.monotonic()>deadline or moto.poll() is not None:raise
            time.sleep(.1)
    bucket='foks-local-proof';s3.create_bucket(Bucket=bucket)
    first=s3.put_object(Bucket=bucket,Key='cas-check',Body=b'abcdef',IfNoneMatch='*')
    def expect412(call):
        try:call()
        except ClientError as error:
            assert error.response['ResponseMetadata']['HTTPStatusCode']==412;return
        raise AssertionError('emulator accepted stale conditional write')
    expect412(lambda:s3.put_object(Bucket=bucket,Key='cas-check',Body=b'x',IfNoneMatch='*'))
    second=s3.put_object(Bucket=bucket,Key='cas-check',Body=b'123456',IfMatch=first['ETag'])
    expect412(lambda:s3.put_object(Bucket=bucket,Key='cas-check',Body=b'x',IfMatch=first['ETag']))
    assert s3.get_object(Bucket=bucket,Key='cas-check',Range='bytes=1-3')['Body'].read()==b'234'
    def race(index):
        try:s3.put_object(Bucket=bucket,Key='cas-check',Body=str(index).encode(),IfMatch=second['ETag']);return 200
        except ClientError as error:return error.response['ResponseMetadata']['HTTPStatusCode']
    with concurrent.futures.ThreadPoolExecutor(2) as executor:assert sorted(executor.map(race,range(2)))==[200,412]
    token=secrets.token_urlsafe(32);control=secrets.token_urlsafe(32)
    project=root/'worker';project.mkdir(mode=0o700)
    for name in ['worker.mjs','authority.mjs','node-decoder.mjs']:shutil.copyfile(source/name,project/name)
    (project/'node-decoder.wasm').write_bytes(decoder_bytes)
    shutil.copyfile(args.schema,project/'generated-schema.json')
    config={'name':'foks-local-proof','main':'worker.mjs','compatibility_date':'2026-01-01','compatibility_flags':['nodejs_compat'],'vars':{'PROOF_TOKEN':token,'PROOF_CONTROL_TOKEN':control},'durable_objects':{'bindings':[{'name':'FOKS_PROOF','class_name':'FoksProof'}]},'migrations':[{'tag':'v1','new_sqlite_classes':['FoksProof']}]}
    config_path=project/'wrangler.jsonc';config_path.write_text(json.dumps(config));config_path.chmod(0o600)
    common=['--bucket','s3://'+bucket,'--endpoint',s3endpoint,'--region','us-east-1']
    deployment=subprocess.run([str(args.celld.resolve()),'deploy',str(project)]+common,cwd=root,env=dict(env,CELLD_ESBUILD=str(args.esbuild.resolve())),capture_output=True,text=True,timeout=45)
    (root/'deploy.log').write_text((deployment.stdout+deployment.stderr).replace(token,'[redacted]').replace(control,'[redacted]'))
    if deployment.returncode:raise RuntimeError('proof deployment failed; see deploy.log')
    tuning={'CELLD_TTL_MS':str(args.lease_ttl_ms),'CELLD_READY_FLEET_GATE_MS':'0','CELLD_REBALANCE_INTERVAL_MS':'0','CELLD_TOKIO_THREADS':'2','CELLD_MAX_RESIDENT_CELLS':'32','CELLD_MAX_RSS_MB':'512','CELLD_V8_HEAP_LIMIT_MB':'64','CELLD_LOCAL_CACHE_MAX_BYTES':'67108864','CELLD_DURABILITY':args.durability}
    nodes=[]
    def node(index):
        public,internal=port(),port();watch=root/f'node-{index}';watch.mkdir()
        process=start([str(args.celld.resolve())]+common+['--listen',f'127.0.0.1:{public}','--internal-listen',f'127.0.0.1:{internal}','--advertise',f'127.0.0.1:{internal}'],f'node-{index}',dict(env,**tuning,CELLD_NODE=f'foks-proof-{index}',CELLD_WATCH=str(watch)))
        ready(f'http://127.0.0.1:{public}/.well-known/celld/health',process)
        item=(process,public,internal);nodes.append(item);return item
    owner=node(0);route[0]=owner[1]
    proxy=http.server.ThreadingHTTPServer(('127.0.0.1',0),Proxy);proxy.daemon_threads=True
    threading.Thread(target=proxy.serve_forever,daemon=True).start()
    (root/'fault').mkdir();(root/'evidence').mkdir()
    testenv=dict(env,FOKS_CELLD_ENDPOINT=f'http://127.0.0.1:{proxy.server_port}/',FOKS_CELLD_TOKEN=token,FOKS_CELLD_CONTROL_TOKEN=control,FOKS_CELLD_FAULT_DIR=str(root/'fault'),FOKS_CELLD_EVIDENCE_DIR=str(root/'evidence'))
    for key in ['FOKS_GO_ORACLE_DIR','FOKS_CELLD_TENANTS','FOKS_CELLD_HISTORY_WRITES','FOKS_CELLD_IDLE_TENANTS','HOME','GOCACHE','GOPATH']:
        if key in os.environ:testenv[key]=os.environ[key]
    if args.fault_mode=='pause-return':testenv['FOKS_CELLD_PAUSE_RETURN']='1'
    test=start([str(args.test_binary.resolve()),args.test_name,'--exact','--ignored','--nocapture','--test-threads=1'],'test',testenv)
    def sample():
        while not stopped.wait(.5):
            for name,process in [('native-test',test)]+[(f'celld-{i}',entry[0]) for i,entry in enumerate(list(nodes))]:
                if process.poll() is None:
                    output=subprocess.run(['/bin/ps','-o','rss=','-p',str(process.pid)],capture_output=True,text=True,timeout=2)
                    if output.stdout.strip().isdigit():record('rss',process=name,kib=int(output.stdout.strip()))
    sampler=threading.Thread(target=sample,daemon=True);sampler.start()
    replay_inputs=[source/'worker.mjs',source/'authority.mjs',source/'node-decoder.mjs',pathlib.Path(__file__),source.parent/'foks-metadata-spike/wasm_limits.py',args.schema]
    record('configuration',celld_sha256=hashlib.sha256(args.celld.read_bytes()).hexdigest(),test_sha256=hashlib.sha256(args.test_binary.read_bytes()).hexdigest(),node_decoder_sha256=decoder_sha256,node_decoder_memory_pages=decoder_limits,replay_source_sha256={str(p.resolve()):hashlib.sha256(p.read_bytes()).hexdigest() for p in replay_inputs},test=args.test_name,fault_mode=args.fault_mode,tuning=tuning,host_load_average=os.getloadavg(),idle_sleep_inhibitor=os.environ.get('FOKS_PROOF_IDLE_SLEEP_INHIBITOR','none'),test_client_timeout_seconds=5,bridge_http_timeout_seconds=4,native_default_request_timeout_seconds=30,native_default_io_timeout_seconds=15,scope='same-host actual FOKS SQL authority experiment; unqualified in-memory S3 emulator')
    if args.test_name in ['real_foks_cell_owner_process_loss_preserves_acknowledged_sql_writes','real_foks_multiple_cells_capacity_baseline']:
        tenants=int(os.environ.get('FOKS_CELLD_TENANTS','4')) if args.test_name=='real_foks_multiple_cells_capacity_baseline' else 1
        assert 1<=tenants<=16
        idle=int(os.environ.get('FOKS_CELLD_IDLE_TENANTS','0')) if args.test_name=='real_foks_multiple_cells_capacity_baseline' else 0
        assert 0<=idle<=128
        if idle:
            marker('idle-populated',test,300)
            time.sleep(1) # sample steady idle celld RSS after native adapters close
            state=get_json(f'http://127.0.0.1:{owner[2]}/state')
            (root/'idle-state.json').write_text(json.dumps(state,indent=2))
            record('idle_population_state',population=idle,resident=state.get('phases',{}).get('resident'),owned=state.get('owned_cells'),stops=state.get('stops'),published=state.get('publishes'))
            (root/'fault'/'idle-state-captured').write_text('actual state recorded')
        for index in range(tenants):marker(f'prepared-{index}' if args.test_name=='real_foks_multiple_cells_capacity_baseline' else 'prepared',test,90)
        initial=get_json(f'http://127.0.0.1:{owner[2]}/state')
        if not idle:assert len(initial['deployment']['cells'])==tenants
        else:assert tenants<=len(initial['deployment']['cells'])<=32
        node(1);node(2)
        (root/'fault'/'fleet-ready').write_text('two followers started before acknowledged writes')
        marker('ready',test,180)
        for index,entry in enumerate(nodes):(root/f'pre-fault-{index}.json').write_text(json.dumps(get_json(f'http://127.0.0.1:{entry[2]}/state'),indent=2))
        if args.fault_mode=='kill':owner[0].kill();owner[0].wait(timeout=5)
        else:os.kill(owner[0].pid,signal.SIGSTOP)
        # Remove only the failed node's fixture-owned artifacts; surviving peer
        # logs and emulator remain. This is still one physical host, not an AZ.
        if args.fault_mode=='kill':shutil.rmtree(root/'node-0')
        with route_lock:route[0]=nodes[1][1]
        record('celld_owner_killed_and_local_volume_removed' if args.fault_mode=='kill' else 'celld_owner_paused',pid=owner[0].pid)
        (root/'fault'/'injected').write_text('owner unavailable, routed to survivor')
        if args.fault_mode=='pause-return':
            marker('successor-verified',test,20)
            os.kill(owner[0].pid,signal.SIGCONT)
            with route_lock:route[0]=owner[1]
            record('former_owner_resumed_and_routed',pid=owner[0].pid)
            (root/'fault'/'former-returned').write_text('former owner resumed')
            marker('former-probed',test,8)
            old_status=owner[0].wait(timeout=5)
            old_log=(root/'node-0.log').read_text()
            assert old_status==3 and 'SELF-FENCE:' in old_log and 'node_lease_' in old_log,'former owner did not prove terminal lease self-fencing'
            record('former_owner_self_fenced',exit_code=old_status)
            with route_lock:route[0]=nodes[1][1]
            (root/'fault'/'survivor-rerouted').write_text('former owner fenced; survivor restored')
    result=test.wait(timeout=180)
    output=(root/'test.log').read_text()
    passed=result==0 and 'test result: ok. 1 passed;' in output and args.test_name in output
    record('result',passed=passed,test_passed=passed,test_exit=result,scope='actual FOKS on local native projection and celld SQL; no production host/AZ qualification')
finally:
    stopped.set()
    if sampler:sampler.join(timeout=3)
    if proxy:proxy.shutdown();proxy.server_close()
    for process in reversed(children):
        if process.poll() is None:
            if args.fault_mode=='pause-return':
                try:os.kill(process.pid,signal.SIGCONT)
                except ProcessLookupError:pass
            process.terminate()
            try:process.wait(timeout=3)
            except subprocess.TimeoutExpired:process.kill();process.wait(timeout=3)
    record('cleanup',all_children_exited=all(p.poll() is not None for p in children),artifacts=str(root))
    for log in logs:log.close()
    passed=record('qualification_complete',passed=passed)['passed']
    ledger.close()
raise SystemExit(0 if passed else 1)
