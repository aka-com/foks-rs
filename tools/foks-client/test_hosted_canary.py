"""Test the shell's grant/revocation boundary independently of network fixtures."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

HERE = Path(__file__).resolve().parent


class HostedCanaryTests(unittest.TestCase):
    def run_canary(self, fail):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binaries = root / "target/release"
            scripts = root / "tools/foks-client"
            metadata = root / "crates/foks-server/protocol"
            for path in (binaries, scripts, metadata):
                path.mkdir(parents=True)
            (metadata / "upstream-v0.1.9.json").write_text("{}")
            shutil.copy(HERE / "hosted-canary.sh", scripts)
            (scripts / "onboarding-canary.py").write_text(
                "import os, sys\nprint('signup\\ndevice-administration\\nrecovery\\npassphrases')\n"
                "sys.exit(int(os.environ['TEST_LIFECYCLE_FAIL']))\n")
            client = binaries / "foks-rs"
            client.write_text('''#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
a = sys.argv[1:]
root = pathlib.Path(os.environ['FOKS_CANARY_STATE_DIR'])
if 'show' in a: print(json.dumps({'probe': 'foks.app:4430'}))
elif 'probe' in a: print(json.dumps({'host_id_hex': '02' + 'ab' * 32}))
elif 'put' in a: shutil.copy(a[a.index('--input') + 1], root / 'object')
elif 'get' in a: shutil.copy(root / 'object', a[a.index('--output') + 1])
elif 'remove' in a: (root / 'object').unlink()
elif 'apply-canary' in a: shutil.copy(a[a.index('--artifact') + 1], root / 'applied')
''')
            signer = binaries / "foks-compat-artifact"
            signer.write_text('''#!/usr/bin/env python3
import json, pathlib, sys
a = sys.argv[1:]
if a[0] == 'allocate-generation': print('100')
else:
    result = {'outcome': a[a.index('--outcome') + 1],
              'capabilities': [a[i+1] for i,v in enumerate(a) if v == '--capability']}
    pathlib.Path(a[a.index('--output') + 1]).write_text(json.dumps(result))
''')
            for path in (client, signer):
                path.chmod(0o700)
            env = {**os.environ, 'FOKS_CANARY_ROOT': directory,
                   'FOKS_CANARY_STATE_DIR': directory, 'FOKS_CANARY_PROFILE': 'baseline',
                   'FOKS_CAPABILITY_PROFILE': 'hosted', 'FOKS_CANARY_ACCOUNT': 'canary',
                   'FOKS_CANARY_SIGNING_KEY_FILE': str(root / 'key'),
                   'FOKS_CANARY_OUTPUT': str(root / 'published'),
                   'FOKS_CANARY_RUN_NUMBER': '1', 'FOKS_CANARY_RUN_ATTEMPT': '1',
                   'TEST_LIFECYCLE_FAIL': str(int(fail))}
            env.pop('FOKS_CANARY_PREVIOUS_ARTIFACT', None)
            env.pop('FOKS_CANARY_EXPECTED_TARGET', None)
            result = subprocess.run(['sh', str(scripts / 'hosted-canary.sh')], env=env,
                                    capture_output=True, timeout=30)
            self.assertEqual(result.returncode, int(fail), result.stderr.decode())
            published = (root / 'published').read_bytes()
            self.assertEqual(published, (root / 'applied').read_bytes())
            self.assertFalse((root / 'object').exists())
            return json.loads(published)

    def test_success_grants_only_exercised_capabilities(self):
        result = self.run_canary(False)
        self.assertEqual(result, {'outcome': 'compatible', 'capabilities': [
            'user-sync', 'kv', 'signup', 'device-administration', 'recovery', 'passphrases']})

    def test_failure_discards_partial_grants_and_applies_drift(self):
        self.assertEqual(self.run_canary(True), {'outcome': 'drift', 'capabilities': []})

    def test_signup_requires_operator_authorization_before_invoking_client(self):
        env = {**os.environ, 'FOKS_CANARY_ALLOW_DISPOSABLE_SIGNUP': ''}
        result = subprocess.run(['python3', str(HERE / 'onboarding-canary.py'),
                                 '--client', '/nonexistent', '--target', 'foks.app:4430'],
                                env=env, capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 1)
        self.assertIn(b'explicit disposable-signup authorization', result.stderr)
        self.assertEqual(result.stdout, b'')


if __name__ == '__main__':
    unittest.main()
