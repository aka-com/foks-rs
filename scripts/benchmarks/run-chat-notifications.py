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
parser.add_argument('--compare', action='store_true', help='Pair current and baseline in alternating order')
parser.add_argument('--fault-onset', choices=['setup', 'measurement'], default='setup')
parser.add_argument('--cases', nargs='+', choices=['steady', 'backlog', 'wide', 'slow'], default=['steady', 'backlog', 'wide', 'slow'])
parser.add_argument('--arms', nargs='+', choices=['off', 'on'], default=['off', 'on'])
parser.add_argument('--repetitions', type=int, default=3)
parser.add_argument('--smoke', action='store_true', help='One-second warmup, three-second measurement, two-second drain; NOT acceptance')
args = parser.parse_args()
if not 1 <= args.repetitions <= 3:
    parser.error('repetitions must be 1..3')
if args.output.exists():
    parser.error('refusing to overwrite results')
args.output.parent.mkdir(parents=True, exist_ok=True)
artifacts = ['target/release/foks-agent', 'target/release/examples/chat_notification_bench', 'scripts/benchmarks/chat-notifications.ts']
artifacts += [str(path) for path in Path('apps/desktop/src/chat').glob('*.ts')]
artifacts += ['apps/desktop/src/scheduling/profile-work.ts', 'apps/desktop/src/bridge.ts']
digests = {name: hashlib.sha256(Path(name).read_bytes()).hexdigest() for name in sorted(artifacts)}
revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()

def run_trial(case, enabled, repetition, implementation):
    command = ['node', '--import', 'tsx', 'scripts/benchmarks/chat-notifications.ts', '--case', case, '--notifications', enabled, '--implementation', implementation, '--fault-onset', args.fault_onset]
    if args.smoke:
        command += ['--warmup', '1', '--duration', '3', '--drain', '2']
    print(f'{case} {enabled} {implementation} repetition {repetition + 1}: started', flush=True)
    start = time.monotonic()
    run = subprocess.run(command, capture_output=True, text=True, timeout=360)
    if run.returncode:
        raise RuntimeError(f'trial failed: {run.stderr[-4000:]}')
    trial = json.loads(run.stdout)
    trial.update(artifactSha256=digests, repetition=repetition + 1, sourceRevision=revision, smoke=args.smoke, elapsedSeconds=time.monotonic() - start)
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
