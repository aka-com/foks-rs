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
  value: 'username: rae\npassword: ••••••••',
};

test('a learned password is spliced in verbatim, whatever characters it holds', () => {
  // `String.replace` treats a string replacement as a *pattern*: `$&` is the
  // match and `` $` `` is everything before it. A password containing them was
  // corrupted in the editor, and Save would have written the corrupted value.
  assert.equal(editableValue(login, 'a$&b'), 'username: rae\npassword: a$&b');
  assert.equal(editableValue(login, 'x$`y'), 'username: rae\npassword: x$`y');
  assert.equal(editableValue(login, "p$'q"), "username: rae\npassword: p$'q");
});

test('a skeleton with no password line does not drop the learned value', () => {
  const bare: Item = { ...login, value: 'username: rae' };
  assert.equal(editableValue(bare, 'secret'), 'secret');
});

test('a complete learned text is shown as-is', () => {
  assert.equal(
    editableValue(login, 'username: rae\npassword: hunter2'),
    'username: rae\npassword: hunter2',
  );
});
