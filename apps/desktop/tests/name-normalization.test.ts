import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  normalizeUsername,
  collapseWhitespace,
  serverTeamName,
  teamAliasOf,
} from '../src/name-normalization';

test('username normalization matches fixtures also checked by Rust', () => {
  const fixtures = readFileSync(
    new URL(
      '../../../crates/foks-verify/tests/fixtures/username-normalization.txt',
      import.meta.url,
    ),
    'utf8',
  );
  for (const line of fixtures.trimEnd().split('\n')) {
    const [input, expected] = line.split('=');
    assert.equal(normalizeUsername(input), expected || null, input);
  }
  assert.equal(
    collapseWhitespace(' Équipe\u0085  bleue ', '_'),
    'Équipe_bleue',
  );
  assert.equal(collapseWhitespace('abc\ufeff'), 'abc\ufeff');
  for (const input of ['abc\n', 'abc\r', 'abc\u2028', 'abc\u2029']) {
    assert.equal(normalizeUsername(input), null);
  }
});

test('team eligibility and aliases accept protocol-valid Unicode names', () => {
  assert.equal(serverTeamName('Développeurs'), 'Développeurs');
  assert.equal(serverTeamName(' Équipe\u0085 bleue '), 'Équipe_bleue');
  assert.equal(teamAliasOf('ééé'), 'eee');
  assert.equal(teamAliasOf('Équipe bleue'), 'equipe-bleue');
  for (const name of ['日本語', 'abc\ufeff', 'a__b', 'Ｆｏｏ']) {
    assert.equal(serverTeamName(name), null);
  }
});
