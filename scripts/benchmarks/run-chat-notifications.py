#!/usr/bin/env python3
"""Run isolated release trials sequentially; checkpoint every completed trial."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--implementation', choices=['current', 'baseline'], default='current')
parser.add_argument('--source', type=Path, default=Path('.'), help='UI source checkout or archive (uses the current notification design)')
parser.add_argument('--source-revision', help='Revision of an explicit source archive; otherwise read from its checkout')
parser.add_argument('--worker', type=Path, default=Path('target/release/examples/chat_notification_bench'), help='Release worker built from the measured server revision')
parser.add_argument('--agent', type=Path, default=Path('target/release/foks-agent'), help='Release agent for the measured revision')
parser.add_argument('--compare', action='store_true', help='Pair current and baseline in alternating order')
parser.add_argument('--fault-onset', choices=['setup', 'measurement'], default='setup')
parser.add_argument('--cases', nargs='+', choices=['steady', 'backlog', 'wide', 'slow'], default=['steady', 'backlog', 'wide', 'slow'])
parser.add_argument('--arms', nargs='+', choices=['off', 'on'], default=['off', 'on'])
parser.add_argument('--repetitions', type=int, default=3)
parser.add_argument('--smoke', action='store_true', help='One-second warmup, three-second measurement, two-second drain; NOT acceptance')
args = parser.parse_args()
if not 1 <= args.repetitions <= 3:
    parser.error('repetitions must be 1..3')
if args.source.resolve() != Path('.').resolve() and (args.compare or args.implementation == 'baseline'):
    parser.error('explicit source cannot be combined with the legacy baseline')
if args.output.exists():
    parser.error('refusing to overwrite results')
args.output.parent.mkdir(parents=True, exist_ok=True)
# Record the actual source and binaries used, including imports outside chat/.
artifacts = {
    'agent': args.agent,
    'worker': args.worker,
    'harness': Path(__file__).with_name('chat-notifications.ts'),
    'fixture': Path(__file__).with_name('chat-notification-fixture.ts'),
    'runner': Path(__file__),
    'package.json': args.source / 'package.json',
    'package-lock.json': args.source / 'package-lock.json',
}
artifacts.update({
    str(path.relative_to(args.source)): path
    for path in (args.source / 'apps/desktop/src').rglob('*')
    if path.is_file()
})
artifacts['crates/foks-agent-proto/chat-limits.json'] = args.source / 'crates/foks-agent-proto/chat-limits.json'
digests = {
    name: hashlib.sha256(path.read_bytes()).hexdigest()
    for name, path in sorted(artifacts.items())
}
revision_command = (
    ['git', 'rev-parse', args.source_revision + '^{commit}']
    if args.source_revision
    else ['git', '-C', str(args.source), 'rev-parse', 'HEAD']
)
revision = subprocess.check_output(revision_command, text=True).strip()
patch = subprocess.check_output(['git', '-C', str(args.source), 'diff', '--binary', 'HEAD']) if (args.source / '.git').exists() else b''
patch_digest = hashlib.sha256(patch).hexdigest() if patch else None

def run_trial(case, enabled, repetition, implementation):
    command = [
        'node', '--import', 'tsx', str(Path(__file__).with_name('chat-notifications.ts')),
        '--source', str(args.source.resolve()),
        '--worker', str(args.worker.resolve()),
        '--agent', str(args.agent.resolve()),
        '--case', case, '--notifications', enabled,
        '--implementation', implementation, '--fault-onset', args.fault_onset,
    ]
    if args.smoke:
        command += ['--warmup', '1', '--duration', '3', '--drain', '2']
    print(f'{case} {enabled} {implementation} repetition {repetition + 1}: started', flush=True)
    start = time.monotonic()
    # Keep worker setup/failure diagnostics visible during long fixture creation.
    run = subprocess.run(command, stdout=subprocess.PIPE, text=True, timeout=360)
    if run.returncode:
        raise RuntimeError(f'trial failed with exit status {run.returncode}; see worker stderr')
    trial = json.loads(run.stdout)
    trial.update(artifactSha256=digests, repetition=repetition + 1, sourceRevision=revision, sourcePatchSha256=patch_digest, smoke=args.smoke, elapsedSeconds=time.monotonic() - start)
    return trial

with args.output.open('x', encoding='utf-8') as output:
    for case in args.cases:
        for repetition in range(args.repetitions):
            arms = args.arms if repetition % 2 == 0 else list(reversed(args.arms))
            implementations = ['current', 'baseline'] if args.compare else [args.implementation]
            if repetition % 2:
                implementations.reverse()
            for enabled in arms:
                for implementation in implementations:
                    trial = run_trial(case, enabled, repetition, implementation)
                    output.write(json.dumps(trial, separators=(',', ':')) + '\n')
                    output.flush()
                    os.fsync(output.fileno())
                    print(f'{case} {enabled} {implementation}: confirmed={trial["confirmedSends"]}, discovered={trial["discoveredCandidates"]}, foreground p95={trial["foregroundHistory"]["p95"]}', flush=True)
                    if case == 'slow' and enabled == 'on' and (not trial['faultCalls'] or not trial['healthyAfterFault']):
                        raise RuntimeError('Recorded invalid fault trial: missing fault or healthy continuation')
