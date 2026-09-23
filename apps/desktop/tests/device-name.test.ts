import assert from 'node:assert/strict';
import { test } from 'node:test';
import { fixDeviceName, boundedDeviceName } from '../src/device-name';

test('device-name presentation cleanup matches the native client', () => {
  for (const [input, expected] of [
    ['Daniel’s MacBook Pro', "Daniel's MacBook Pro"],
    ['a‘b’c‚d‛e', "a'b'c'd'e"],
    ['a‐b‑c–d—e−f', 'a-b-c-d-e-f'],
    [' Ｍａｃ　１２．３＋ ', 'Mac 12.3+'],
    ['  René\u00a0\t\n Mac  ', 'René Mac'],
    ["Élodie's Mac", "Élodie's Mac"],
    ['Mac\u200b', 'Mac\u200b'],
    ['Mac\ufeff', 'Mac\ufeff'],
    ['“Mac” 😀', '“Mac” 😀'],
  ]) {
    assert.equal(fixDeviceName(input), expected);
    assert.equal(fixDeviceName(expected), expected);
  }
  assert.equal(fixDeviceName('Ａ'.repeat(201)), 'A'.repeat(201));
});

test('device display bounds count UTF-8 bytes, not normalized characters', () => {
  assert.equal(boundedDeviceName('é'.repeat(200)), true);
  assert.equal(boundedDeviceName('e\u0301'.repeat(200)), true);
  assert.equal(boundedDeviceName('é'.repeat(2048)), true);
  assert.equal(boundedDeviceName('é'.repeat(2049)), false);
});
