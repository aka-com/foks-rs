#!/usr/bin/env python3
"""Run fixed, locally generated policy fixtures on disposable pinned celld/Moto."""
import argparse
import hashlib
import http.client
import json
import os
import pathlib
import secrets
import shutil
import socket
import subprocess
import threading
import time
import boto3
from botocore.config import Config
from qualify import PINS, artifacts, encoded, pinned, HOST_PIN, CONTROL_BINDING, CONTROL_OUTCOMES, program_order

CELLD_PIN='92ededd2bcfebeb1e78cc2d97219f00214a79eb6606f0eef48208fca3f98c15a'


def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('celld','moto','esbuild','native-results','profile','output'):
        parser.add_argument('--'+name,type=pathlib.Path,required=True)
    args=parser.parse_args()
    if digest(args.celld)!=CELLD_PIN:parser.error('unreviewed celld binary')
    profile=artifacts(args.profile)
    fixture_path=args.native_results/'fixture.json'
    if fixture_path.is_symlink() or fixture_path.stat().st_size>4*1024*1024:parser.error('native fixture bound')
    fixture_bytes=fixture_path.read_bytes();fixture=json.loads(fixture_bytes)
    native_report=json.loads((args.native_results/'qualification.json').read_text())
    if (native_report.get('passed') is not True or native_report.get('pins')!=PINS or
        native_report.get('fixture_sha256')!=hashlib.sha256(fixture_bytes).hexdigest() or
        fixture.get('pins')!=PINS or fixture.get('app')!=profile['foks-59.json']+profile['authority-v1.json'] or
        fixture.get('platform')!=profile['celld-0.6.2.json'] or fixture.get('control_host_sha256')!=HOST_PIN):parser.error('native generated fixture provenance mismatch')
    programs=fixture.get('programs')
    if not isinstance(programs,list) or not 1<=len(programs)<=1024:parser.error('fixture program count')
    ordered=program_order(programs)
    args.output.mkdir(mode=0o700,exist_ok=False);root=args.output.resolve()
    source=pathlib.Path(__file__).resolve().parent;token=secrets.token_urlsafe(32)
    host_path=source.parent/'foks-metadata-spike/host.mjs'
    pinned(host_path,HOST_PIN,1024*1024)
    env={'PATH':os.environ.get('PATH','/usr/bin:/bin'),'TMPDIR':str(root),'AWS_ACCESS_KEY_ID':'policy-dummy',
         'AWS_SECRET_ACCESS_KEY':'policy-dummy-secret','AWS_REGION':'us-east-1','AWS_DEFAULT_REGION':'us-east-1',
         'AWS_CONFIG_FILE':'/dev/null','AWS_SHARED_CREDENTIALS_FILE':'/dev/null','AWS_EC2_METADATA_DISABLED':'true','CELLD_OTEL':'0','NO_COLOR':'1'}
    children=[];logs=[];deadline=time.monotonic()+600
    report={'passed':False,'production_admission':False,'scope':__doc__,'programs':[],'cleanup':[],
            'control_host_sha256':HOST_PIN,'direct_sql_programs':0,'mapped_controls':0,
            'celld_sha256':CELLD_PIN,'fixture_sha256':hashlib.sha256(fixture_bytes).hexdigest(),
            'source_sha256':{p.name:digest(p) for p in source.iterdir() if p.suffix in ('.py','.mjs')}}

    def left(maximum):
        remaining=deadline-time.monotonic()
        if remaining<=0:raise TimeoutError('controller deadline')
        return min(maximum,remaining)

    def port():
        with socket.socket() as sock:
            sock.bind(('127.0.0.1',0));return sock.getsockname()[1]

    def start(command,name,extra=None):
        log=(root/(name+'.log')).open('x');logs.append(log)
        p=subprocess.Popen(command,cwd=root,env=dict(env,**(extra or {})),stdout=log,stderr=subprocess.STDOUT)
        children.append((name,p));return p

    def call(public,path,capability=token,method='POST',maximum=20):
        begin=time.monotonic();end=begin+left(maximum)
        connection=http.client.HTTPConnection('127.0.0.1',public,timeout=max(.001,end-time.monotonic()))
        timer=None
        try:
            connection.connect();sock=connection.sock
            def expire():
                try:sock.shutdown(socket.SHUT_RDWR)
                except OSError:pass
            timer=threading.Timer(max(.001,end-time.monotonic()),expire);timer.daemon=True;timer.start()
            connection.request(method,path,body=None,headers={'authorization':'Bearer '+capability,'content-length':'0'})
            response=connection.getresponse();body=response.read(16*1024*1024+1)
            if len(body)>16*1024*1024:raise ValueError('response bound')
            try:value=json.loads(body)
            except ValueError:value={'text':body[:240].decode(errors='replace')}
            return response.status,value,time.monotonic()-begin
        finally:
            if timer is not None:timer.cancel();timer.join(timeout=1)
            connection.close()

    try:
        endpoint='http://127.0.0.1:'+str(port())
        moto=start([str(args.moto.resolve()),'-H','127.0.0.1','-p',endpoint.rsplit(':',1)[1]],'moto')
        s3=boto3.client('s3',endpoint_url=endpoint,region_name='us-east-1',aws_access_key_id=env['AWS_ACCESS_KEY_ID'],aws_secret_access_key=env['AWS_SECRET_ACCESS_KEY'],config=Config(retries={'max_attempts':0},connect_timeout=1,read_timeout=2))
        until=time.monotonic()+left(20)
        while True:
            try:s3.list_buckets();break
            except Exception:
                if moto.poll() is not None or time.monotonic()>=until:raise
                time.sleep(.1)
        bucket='foks-sqlite-policy';s3.create_bucket(Bucket=bucket)
        project=root/'worker';project.mkdir(mode=0o700)
        shutil.copyfile(source/'worker.mjs',project/'worker.mjs');(project/'fixture.json').write_bytes(fixture_bytes)
        config={'name':'foks-sqlite-policy','main':'worker.mjs','compatibility_date':'2026-01-01',
                'vars':{'POLICY_TOKEN':token},'durable_objects':{'bindings':[{'name':'POLICY','class_name':'SqlitePolicy'}]},
                'migrations':[{'tag':'v1','new_sqlite_classes':['SqlitePolicy']}]}
        config_path=project/'wrangler.jsonc';config_path.write_bytes(encoded(config));config_path.chmod(0o600)
        common=['--bucket','s3://'+bucket,'--endpoint',endpoint,'--region','us-east-1']
        deployed=subprocess.run([str(args.celld.resolve()),'deploy',str(project)]+common,cwd=root,env=dict(env,CELLD_ESBUILD=str(args.esbuild.resolve())),capture_output=True,text=True,timeout=left(45))
        (root/'deploy.log').write_text((deployed.stdout+deployed.stderr).replace(token,'[redacted]'))
        if deployed.returncode:raise RuntimeError('deployment failed')
        report['deployed_sha256']={p.name:digest(p) for p in project.iterdir() if p.is_file()}
        public,internal=port(),port();watch=root/'watch';watch.mkdir()
        node=start([str(args.celld.resolve())]+common+['--listen',f'127.0.0.1:{public}','--internal-listen',f'127.0.0.1:{internal}','--advertise',f'127.0.0.1:{internal}'],'celld',
                   {'CELLD_NODE':'sqlite-policy','CELLD_WATCH':str(watch),'CELLD_READY_FLEET_GATE_MS':'0','CELLD_DURABILITY':'bucket','CELLD_MAX_RESIDENT_CELLS':'4','CELLD_V8_HEAP_LIMIT_MB':'64','CELLD_TOKIO_THREADS':'2'})
        until=time.monotonic()+left(30)
        while True:
            try:
                status,_,_=call(public,'/.well-known/celld/health',method='GET',maximum=1)
                if status==200:break
            except (OSError,http.client.HTTPException):pass
            if node.poll() is not None or time.monotonic()>=until:raise RuntimeError('readiness failed')
            time.sleep(.1)
        status,_,_=call(public,'/init',capability='wrong')
        if status!=403:raise RuntimeError('private capability gate')
        status,value,elapsed=call(public,'/init');report['initialization']={'status':status,'result':value,'seconds':elapsed}
        if status!=200 or value.get('passed') is not True:raise RuntimeError('initialization/compile-option equivalence failed')
        for index,expected in ordered:
            status,value,elapsed=call(public,'/program/'+str(index))
            (root/f'program-{index:04d}.json').write_bytes(encoded(value))
            row={'index':index,'id':expected['id'],'status':status,'seconds':elapsed,'passed':value.get('passed') is True}
            report['programs'].append(row)
            if status!=200 or not row['passed'] or value.get('id')!=expected['id']:
                raise RuntimeError('program qualification failed: '+expected['id'])
            if expected['execution']==CONTROL_BINDING:
                if (value.get('binding')!=CONTROL_BINDING or value.get('sql_authorization_denied') is not True or
                    value.get('outcomes')!=CONTROL_OUTCOMES or value.get('schema_cookie_unchanged') is not True or
                    value.get('callback_return_preserved') is not True or value.get('sentinels_propagated') is not True or 'normalized_sha256' in value):
                    raise RuntimeError('mapped control qualification failed')
                report['mapped_controls']+=1
            else:
                if value.get('normalized_sha256')!=expected['normalized_sha256'] or 'binding' in value:
                    raise RuntimeError('program equivalence failed: '+expected['id'])
                report['direct_sql_programs']+=1
        status,value,elapsed=call(public,'/behavior');report['behavior']={'status':status,'result':value,'seconds':elapsed}
        if status!=200 or value.get('passed') is not True:raise RuntimeError('behavior equivalence failed')
        if report['mapped_controls']!=3 or report['direct_sql_programs']!=len(programs)-3:
            raise RuntimeError('qualification completeness failed')
        report['passed']=True
    except Exception as error:report['error']=str(error).replace(token,'[redacted]')[:400]
    finally:
        for name,process in reversed(children):
            forced=False
            if process.poll() is None:
                process.terminate()
                try:process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    forced=True;process.kill()
                    try:process.wait(timeout=3)
                    except subprocess.TimeoutExpired:report['passed']=False;report['error']='child not reaped'
            report['cleanup'].append({'process':name,'reaped':process.poll() is not None,'forced_kill':forced,'exit_code':process.returncode})
        for log in logs:log.close()
        current={p.name:digest(p) for p in source.iterdir() if p.suffix in ('.py','.mjs')}
        if current!=report['source_sha256'] or digest(fixture_path)!=report['fixture_sha256'] or digest(host_path)!=HOST_PIN:
            report['passed']=False;report['error']='diagnostic sources changed'
        (root/'qualification.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({'passed':report['passed'],'programs':len(report['programs']),'error':report.get('error'),'report':str(root/'qualification.json')}))
    return 0 if report['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
