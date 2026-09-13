import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('summary', Path(__file__).with_name('summarize-chat-notifications.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

def matrix():
    return [dict(implementation='current', workload=case, enabled=enabled,
                 repetition=repetition, smoke=False, warmupMs=15000,
                 durationMs=60000, drainMs=10000,
                 channels=200 if case == 'wide' else 70,
                 confirmedSends=360, discoveredCandidates=360 if enabled else 0,
                 undiscoveredSends=0 if enabled else 360, sendErrors=0,
                 foregroundSlots=120, missedSlots=0, lateDisplays=0,
                 peakJobs=1, peakPerProfileJobs=1, peakBackgroundRPCs=1,
                 faultCalls=3 if case == 'slow' and enabled else 0,
                 healthyAfterFault=10 if case == 'slow' and enabled else 0,
                 foregroundHistory={'p95': 125 if enabled else 100})
            for case in module.CASES for enabled in (False, True)
            for repetition in (1, 2, 3)]

class AcceptanceTests(unittest.TestCase):
    def test_incomplete_or_duplicate_repetitions_never_pass(self):
        trials = matrix()
        self.assertTrue(module.summarize(trials)['passed'])
        self.assertFalse(module.summarize(trials[:-1])['complete'])
        trials[-1]['repetition'] = 2
        self.assertFalse(module.summarize(trials)['passed'])

    def test_undiscovered_counts_override_good_percentiles(self):
        trials = matrix()
        trials[3].update(discoveredCandidates=356, undiscoveredSends=4)
        self.assertFalse(module.summarize(trials)['passed'])

    def test_median_of_trial_p95_and_combined_trigger(self):
        trials = matrix()
        for sample, value in zip(trials[3:6], (150, 150, 1000)):
            sample['foregroundHistory']['p95'] = value
        result = module.summarize(trials)
        self.assertEqual(result['rows'][0]['onP95Ms'], 150)
        self.assertTrue(result['passed'])  # Exactly 50 ms is not above the trigger.
        trials[4]['foregroundHistory']['p95'] = 151
        self.assertFalse(module.summarize(trials)['passed'])

    def test_fault_execution_shutdown_and_bounds_are_required(self):
        original = matrix()
        for key, value in [('faultCalls', 0), ('healthyAfterFault', 0), ('lateDisplays', 1), ('peakJobs', 3), ('peakPerProfileJobs', 2), ('missedSlots', 1)]:
            with self.subTest(key=key):
                trials = copy.deepcopy(original)
                trials[-1][key] = value
                self.assertFalse(module.summarize(trials)['passed'])

if __name__ == '__main__':
    unittest.main()
