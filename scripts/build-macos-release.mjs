import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repository = new URL('../', import.meta.url);
// Node parses dotenv syntax without executing shell expressions and leaves
// already-exported values intact, so CI and explicit overrides take priority.
try {
  process.loadEnvFile(fileURLToPath(new URL('.env', repository)));
} catch (error) {
  if (error.code !== 'ENOENT') throw error;
}

const result = spawnSync(
  'bash',
  [fileURLToPath(new URL('scripts/build-macos-release.sh', repository))],
  { cwd: fileURLToPath(repository), stdio: 'inherit' },
);
if (result.error) throw result.error;
if (result.signal) {
  throw new Error(`Release build terminated with ${result.signal}`);
}
process.exitCode = result.status ?? 1;
