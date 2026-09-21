import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export function bundleScriptForPlatform(platform) {
  if (platform === 'darwin') return 'bundle:macos';
  if (platform === 'linux') return 'bundle:deb';
  throw new Error(`Desktop production builds are unsupported on ${platform}`);
}

export function buildDesktop(platform = process.platform) {
  const script = bundleScriptForPlatform(platform);
  const npm = platform === 'win32' ? 'npm.cmd' : 'npm';
  const result = spawnSync(npm, ['run', script], { stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.signal) {
    throw new Error(`npm run ${script} terminated with ${result.signal}`);
  }
  return result.status ?? 1;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  process.exitCode = buildDesktop();
}
