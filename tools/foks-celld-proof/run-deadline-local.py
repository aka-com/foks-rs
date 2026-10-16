#!/usr/bin/env python3
"""Disposable deployed-SQL diagnostic, not a production deadline or HA proof."""
import argparse
import concurrent.futures
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

PIN = '92ededd2bcfebeb1e78cc2d97219f00214a79eb6606f0eef48208fca3f98c15a'
SIZES = (1000, 100000, 1000000, 8000000)
parser = argparse.ArgumentParser(description=__doc__)
for name in ('celld', 'moto', 'esbuild', 'output'):
    parser.add_argument('--' + name, required=True, type=pathlib.Path)
parser.add_argument('--operation-deadline-ms', type=int, choices=(250, 15000), default=250)
args = parser.parse_args()
for path in (args.celld, args.moto, args.esbuild):
    if not path.is_file():
        parser.error('missing executable: ' + str(path))
if hashlib.sha256(args.celld.read_bytes()).hexdigest() != PIN:
    parser.error('celld binary differs from reviewed 0.6.2 artifact')
args.output.mkdir(mode=0o700, parents=False, exist_ok=False)
root = args.output.resolve()
source = pathlib.Path(__file__).resolve().parent
token = secrets.token_urlsafe(32)
env = {
    'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'TMPDIR': str(root),
    'AWS_ACCESS_KEY_ID': 'local-deadline-dummy', 'AWS_SECRET_ACCESS_KEY': 'local-deadline-dummy-secret',
    'AWS_REGION': 'us-east-1', 'AWS_DEFAULT_REGION': 'us-east-1',
    'AWS_CONFIG_FILE': '/dev/null', 'AWS_SHARED_CREDENTIALS_FILE': '/dev/null',
    'AWS_EC2_METADATA_DISABLED': 'true', 'CELLD_OTEL': '0', 'NO_COLOR': '1',
}
children, logs, results = [], [], []
overall_deadline = time.monotonic() + 240
report = {'scope': __doc__, 'operation_deadline_ms': args.operation_deadline_ms,
          'passed': False, 'production_deadline_qualified': False, 'results': results,
          'overlap': 'unknown; controller start-marker observation is not native execution instrumentation',
          'physical_work': 'not inferred canceled from HTTP timeout', 'cleanup': []}


def remaining(deadline, maximum):
    left = min(deadline, overall_deadline) - time.monotonic()
    if left <= 0:
        raise TimeoutError('diagnostic controller deadline')
    return min(left, maximum)


def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def start(command, name, extra=None):
    log = (root / (name + '.log')).open('x')
    logs.append(log)
    process = subprocess.Popen(command, cwd=root, env=dict(env, **(extra or {})), stdout=log, stderr=subprocess.STDOUT)
    children.append((name, process))
    return process


def call(public, path, deadline, capability=token, method='POST'):
    began = time.monotonic()
    connection = http.client.HTTPConnection('127.0.0.1', public, timeout=remaining(deadline, 20))
    event = {'path': path, 'started_mono': began}
    cutoff = None
    try:
        connection.connect()
        sock = connection.sock
        def expire_socket():
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        # A socket inactivity timeout is not a total deadline. Independently
        # interrupt this diagnostic client's socket even during a trickled body.
        cutoff = threading.Timer(remaining(deadline, 20), expire_socket)
        cutoff.daemon = True
        cutoff.start()
        connection.request(method, path, body=None, headers={'authorization': 'Bearer ' + capability, 'content-length': '0'})
        response = connection.getresponse()
        event.update(status=response.status, headers_mono=time.monotonic())
        data = response.read(4097)
        if len(data) > 4096:
            raise ValueError('diagnostic response bound')
        try:
            event['body'] = json.loads(data)
        except ValueError:
            event['text'] = data.decode('utf-8', errors='replace')[:240]
    except (OSError, http.client.HTTPException, ValueError) as error:
        event['error'] = str(error).replace(token, '[redacted]')[:240]
    finally:
        if cutoff is not None:
            cutoff.cancel()
            cutoff.join(timeout=1)
        connection.close()
        event['finished_mono'] = time.monotonic()
        event['elapsed_seconds'] = event['finished_mono'] - began
    return event


def ready(public, process):
    until = min(overall_deadline, time.monotonic() + 30)
    while time.monotonic() < until:
        if process.poll() is not None:
            raise RuntimeError('celld exited during readiness')
        event = call(public, '/.well-known/celld/health', min(until, time.monotonic() + 1), method='GET')
        if event.get('status') == 200:
            return
        time.sleep(.1)
    raise TimeoutError('celld readiness')


try:
    endpoint = 'http://127.0.0.1:' + str(port())
    moto = start([str(args.moto.resolve()), '-H', '127.0.0.1', '-p', endpoint.rsplit(':', 1)[1]], 'moto')
    s3 = boto3.client('s3', endpoint_url=endpoint, region_name='us-east-1',
                      aws_access_key_id=env['AWS_ACCESS_KEY_ID'], aws_secret_access_key=env['AWS_SECRET_ACCESS_KEY'],
                      config=Config(retries={'max_attempts': 0}, connect_timeout=1, read_timeout=2))
    until = min(overall_deadline, time.monotonic() + 20)
    while True:
        try:
            s3.list_buckets()
            break
        except Exception:
            if moto.poll() is not None or time.monotonic() >= until:
                raise
            time.sleep(.1)
    bucket = 'foks-deadline-diagnostic'
    s3.create_bucket(Bucket=bucket)
    project = root / 'worker'
    project.mkdir(mode=0o700)
    shutil.copyfile(source / 'deadline-worker.mjs', project / 'worker.mjs')
    config = {'name': 'foks-deadline-diagnostic', 'main': 'worker.mjs', 'compatibility_date': '2026-01-01',
              'vars': {'PROBE_TOKEN': token}, 'durable_objects': {'bindings': [{'name': 'PROBES', 'class_name': 'DeadlineProbe'}]},
              'migrations': [{'tag': 'v1', 'new_sqlite_classes': ['DeadlineProbe']}]}
    config_path = project / 'wrangler.jsonc'
    config_path.write_text(json.dumps(config))
    config_path.chmod(0o600)
    common = ['--bucket', 's3://' + bucket, '--endpoint', endpoint, '--region', 'us-east-1']
    deployed = subprocess.run([str(args.celld.resolve()), 'deploy', str(project)] + common,
                              cwd=root, env=dict(env, CELLD_ESBUILD=str(args.esbuild.resolve())),
                              capture_output=True, text=True, timeout=remaining(overall_deadline, 45))
    (root / 'deploy.log').write_text((deployed.stdout + deployed.stderr).replace(token, '[redacted]'))
    if deployed.returncode:
        raise RuntimeError('diagnostic deployment failed')
    report['sha256'] = {str(path): hashlib.sha256(path.read_bytes()).hexdigest()
                        for path in (args.celld, source / 'deadline-worker.mjs', pathlib.Path(__file__), project / 'worker.mjs', config_path)}
    public, internal = port(), port()
    watch = root / 'watch'
    watch.mkdir()
    node = start([str(args.celld.resolve())] + common + ['--listen', f'127.0.0.1:{public}',
                 '--internal-listen', f'127.0.0.1:{internal}', '--advertise', f'127.0.0.1:{internal}'], 'celld',
                 {'CELLD_NODE': 'deadline-probe', 'CELLD_WATCH': str(watch), 'CELLD_READY_FLEET_GATE_MS': '0',
                  'CELLD_DURABILITY': 'bucket', 'CELLD_MAX_RESIDENT_CELLS': '4', 'CELLD_V8_HEAP_LIMIT_MB': '64',
                  'CELLD_TOKIO_THREADS': '2', 'CELLD_OPERATION_DEADLINE_MS': str(args.operation_deadline_ms)})
    ready(public, node)
    denied = call(public, '/main/init/0/1', time.monotonic() + 5, capability='wrong')
    report['denied'] = denied
    if denied.get('status') != 403:
        raise RuntimeError('private capability not enforced')
    report['prewarm'] = []
    for cell in ('main', 'other'):
        warm = call(public, f'/{cell}/init/0/1', time.monotonic() + 20)
        report['prewarm'].append(warm)
        if warm.get('status') != 200 or warm.get('body', {}).get('initialized') is not True:
            raise RuntimeError('prewarm inconclusive; no SQL interruption claim')
    for case_id, steps in enumerate(SIZES):
        deadline = time.monotonic() + 20
        row = {'case_id': case_id, 'attempt': 1, 'steps': steps, 'reads': []}
        results.append(row)
        # Every submitted client has its own finite socket budget. The executor
        # joins them; it is not used as a cancellation proof for actor work.
        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as clients:
            running = clients.submit(call, public, f'/main/run/{case_id}/1', deadline)
            if case_id == 3:
                marker = f'foks_deadline_sql_start {case_id}/1'
                start_seen = False
                log_path = root / 'celld.log'
                while not running.done() and time.monotonic() < min(deadline, overall_deadline):
                    if log_path.stat().st_size > 1024 * 1024:
                        raise RuntimeError('diagnostic log bound')
                    if marker in log_path.read_text(errors='replace'):
                        start_seen = True
                        break
                    time.sleep(.005)
                row['start_marker_observed'] = start_seen
                row['read_issue_mono'] = time.monotonic()
                reads = [clients.submit(call, public, f'/{cell}/read/{case_id}/1', deadline) for cell in ('main', 'other')]
                row['reads'] = [future.result() for future in reads]
            row['request'] = running.result()
        if row['request'].get('error') or time.monotonic() >= deadline:
            row['outcome'] = 'unknown; controller retires disposable process'
            raise TimeoutError('probe client failed or case deadline expired')
        observation = call(public, f'/main/read/{case_id}/1', deadline)
        row['observation'] = observation
        if observation.get('status') != 200:
            row['outcome'] = 'unknown'
            raise RuntimeError('marker observation unavailable')
        observed = observation.get('body', {})
        if (observed.get('case_id'), observed.get('attempt')) != (case_id, 1):
            raise RuntimeError('observation binding mismatch')
        expected = str(steps * (steps + 1) // 2)
        marker = observed.get('marker')
        if marker is None:
            row['outcome'] = 'no committed marker observed; current local actor only'
        elif marker == {'stage': 1, 'total': expected}:
            row['outcome'] = 'committed marker observed; sync success requires successful run response'
        else:
            raise RuntimeError('partial or incorrect transaction marker')
        if row['request'].get('status') == 200:
            body = row['request'].get('body', {})
            if ((body.get('case_id'), body.get('attempt'), body.get('total'), body.get('synced')) !=
                    (case_id, 1, expected, True) or marker != {'stage': 1, 'total': expected}):
                raise RuntimeError('success did not match post-sync marker')
        elif case_id == 0:
            raise RuntimeError('small baseline SQL did not succeed; timing diagnostic inconclusive')
        row['success_exceeded_actor_timer'] = (row['request'].get('status') == 200 and
            row['request']['elapsed_seconds'] * 1000 > args.operation_deadline_ms)
    report['passed'] = True
    report['timer_claim'] = ('successful request exceeded configured actor timer; the setting is not a total request deadline; SQL-only duration remains unmeasured'
                            if any(row['success_exceeded_actor_timer'] for row in results)
                            else 'inconclusive; no successful request exceeded configured actor timer')
except Exception as error:
    report['error'] = str(error).replace(token, '[redacted]')[:400]
finally:
    for name, process in reversed(children):
        forced = False
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                forced = True
                process.kill()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    report['passed'] = False
                    report['error'] = 'disposable child did not reap after kill; physical completion unknown'
        report['cleanup'].append({'process': name, 'reaped': process.poll() is not None, 'forced_kill': forced, 'exit_code': process.returncode})
    for log in logs:
        log.close()
    (root / 'qualification.json').write_text(json.dumps(report, indent=2))
    print(json.dumps({'passed': report['passed'], 'report': str(root / 'qualification.json'), 'error': report.get('error'), 'timer_claim': report.get('timer_claim')}))
raise SystemExit(0 if report['passed'] else 1)
