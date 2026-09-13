#!/usr/bin/env python3
"""Validate the complete current matrix and print its measured acceptance summary."""
import argparse
import json
from statistics import median

CASES = ('steady', 'backlog', 'wide', 'slow')

def summarize(trials):
    current = [t for t in trials if t['implementation'] == 'current']
    groups = {(case, enabled): [t for t in current if t['workload'] == case and t['enabled'] == enabled] for case in CASES for enabled in (False, True)}
    complete = all(len(group) == 3 and {t['repetition'] for t in group} == {1, 2, 3} for group in groups.values())
    failures = []
    for trial in current:
        label = f"{trial['workload']} {'on' if trial['enabled'] else 'off'} #{trial['repetition']}"
        if trial.get('smoke') or (trial['warmupMs'], trial['durationMs'], trial['drainMs']) != (15000, 60000, 10000):
            failures.append(f'{label}: not an acceptance duration')
        if trial['channels'] != (200 if trial['workload'] == 'wide' else 70):
            failures.append(f'{label}: wrong channel count')
        if trial['confirmedSends'] <= 0 or trial['sendErrors']:
            failures.append(f'{label}: generator did not supply clean confirmed traffic')
        if trial['discoveredCandidates'] + trial['undiscoveredSends'] != trial['confirmedSends']:
            failures.append(f'{label}: inconsistent discovery accounting')
        if trial['foregroundHistory']['p95'] is None:
            failures.append(f'{label}: no successful foreground history')
        if trial['foregroundSlots'] != 120 or trial['missedSlots'] or trial['lateDisplays']:
            failures.append(f'{label}: missed foreground slots or late display')
        if trial['peakJobs'] > 2 or trial['peakPerProfileJobs'] > 1 or trial['peakBackgroundRPCs'] > 1:
            failures.append(f'{label}: notification concurrency exceeded bounds')
        if trial['enabled'] and trial['workload'] in ('steady', 'wide') and trial['discoveredCandidates'] < .99 * trial['confirmedSends']:
            failures.append(f'{label}: discovery below 99%')
        if trial['enabled'] and trial['workload'] == 'slow' and (not trial['faultCalls'] or not trial['healthyAfterFault']):
            failures.append(f'{label}: missing permanent fault or healthy continuation')
    rows = []
    for case in CASES:
        off, on = groups[case, False], groups[case, True]
        if not off or not on or any(t['foregroundHistory']['p95'] is None for t in off + on):
            continue
        off_p95 = median(t['foregroundHistory']['p95'] for t in off)
        on_p95 = median(t['foregroundHistory']['p95'] for t in on)
        delta = on_p95 - off_p95
        relative = delta / off_p95
        if case != 'slow' and delta > 50 and relative > .2:
            failures.append(f'{case}: combined foreground latency review trigger exceeded')
        rows.append({'workload': case, 'offTrials': len(off), 'onTrials': len(on), 'offP95Ms': off_p95, 'onP95Ms': on_p95, 'deltaMs': delta, 'deltaPercent': relative * 100, 'confirmed': sum(t['confirmedSends'] for t in on), 'discovered': sum(t['discoveredCandidates'] for t in on), 'undiscovered': sum(t['undiscoveredSends'] for t in on), 'minimumDiscoveryPercent': min(100 * t['discoveredCandidates'] / max(1, t['confirmedSends']) for t in on)})
    return {'complete': complete, 'passed': complete and not failures, 'failures': failures, 'rows': rows}

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('results')
    args = parser.parse_args()
    with open(args.results, encoding='utf-8') as source:
        result = summarize([json.loads(line) for line in source if line.strip()])
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if result['passed'] else 1 if result['complete'] else 2)
