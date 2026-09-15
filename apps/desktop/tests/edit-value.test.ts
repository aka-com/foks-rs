import assert from 'node:assert/strict';
import test from 'node:test';

import { editableValue } from '../src/screens/edit-value';
import type { Item } from '../src/model';

const login: Item = {
  store: 'acct:personal',
  path: '/logins/example.com',
  kind: 'Secret',
  size: 0,
  version: 1,
  read: 'Owner',
  write: 'Owner',
  value: 'username: satoshi\npassword: ••••••••',
};

test('editableValue preserves special regex replacement tokens in password value', () => {
  // Verifies that replacement tokens in password values are treated as literals.
  assert.equal(
    editableValue(login, 'a$&b'),
    'username: satoshi\npassword: a$&b',
  );
  assert.equal(
    editableValue(login, 'x$`y'),
    'username: satoshi\npassword: x$`y',
  );
  assert.equal(
    editableValue(login, "p$'q"),
    "username: satoshi\npassword: p$'q",
  );
});

test('editableValue returns decrypted value when item value lacks a password field', () => {
  const bare: Item = { ...login, value: 'username: satoshi' };
  assert.equal(editableValue(bare, 'secret'), 'secret');
});

test('editableValue returns full record unchanged when matching existing format', () => {
  assert.equal(
    editableValue(login, 'username: satoshi\npassword: hunter2'),
    'username: satoshi\npassword: hunter2',
  );
});
