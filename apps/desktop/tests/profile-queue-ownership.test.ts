import assert from 'node:assert/strict';
import test from 'node:test';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

/**
 * The profile queue admits one request per profile at a time and drains
 * serially, so a request that awaits another same-profile submission from
 * inside its own callback waits for itself until the admission deadline.
 * One bridge method, `invitation`, queues itself; every caller of it has to
 * call it directly. These checks hold that contract at the source, since
 * the mock bridge does not queue and a render test would not notice.
 */

const source = join(import.meta.dirname, '../src');

function files(directory: string): string[] {
  return readdirSync(directory).flatMap((name) => {
    const path = join(directory, name);
    if (statSync(path).isDirectory()) return files(path);
    return /\.tsx?$/.test(name) ? [path] : [];
  });
}

test('invitation is the only bridge method that queues itself', () => {
  const owners = files(join(source, 'bridge'))
    .filter((path) => !/profile-work\.ts$|index\.ts$|snapshot/.test(path))
    .flatMap((path) => {
      const text = readFileSync(path, 'utf8');
      const matches = text.match(/(\w+):\s*\([^)]*\)\s*=>\s*enqueueProfileWork\(/g);
      return (matches ?? []).map((match) => match.split(':')[0]);
    });
  assert.deepEqual(owners, ['invitation']);
});

test('no caller queues a bridge method that queues itself', () => {
  const offenders = files(source).flatMap((path) => {
    const text = readFileSync(path, 'utf8');
    const pattern = /(enqueue|schedule)ProfileWork\([\s\S]{0,400}?\.invitation\(/g;
    return text.match(pattern) ? [path.slice(source.length + 1)] : [];
  });
  assert.deepEqual(offenders, []);
});
