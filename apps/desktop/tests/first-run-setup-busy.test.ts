import assert from 'node:assert/strict';
import test from 'node:test';
import {
  setupBusy,
  setupLeaveBlocked,
  setupMutating,
} from '../src/screens/first-run/setup-busy';

const IDLE = {
  account: false,
  discovery: false,
  serverCheck: false,
  phrase: false,
};

test('setup busy names one owner, writes before checks before phrase preparation', () => {
  assert.equal(setupBusy(IDLE), null);
  assert.equal(setupBusy({ ...IDLE, phrase: true }), 'phrase');
  assert.equal(
    setupBusy({ ...IDLE, serverCheck: true, phrase: true }),
    'server-check',
  );
  assert.equal(
    setupBusy({ ...IDLE, discovery: true, serverCheck: true }),
    'discovery',
  );
  assert.equal(
    setupBusy({
      account: true,
      discovery: true,
      serverCheck: true,
      phrase: true,
    }),
    'account',
  );
});

test('phrase preparation is busy but not a mutation', () => {
  assert.equal(setupMutating(null), false);
  assert.equal(setupMutating('phrase'), false);
  assert.equal(setupMutating('server-check'), true);
  assert.equal(setupMutating('discovery'), true);
  assert.equal(setupMutating('account'), true);
});

test('leaving setup is refused only while an unacknowledged write is out', () => {
  assert.equal(setupLeaveBlocked(null, false), false);
  assert.equal(setupLeaveBlocked('phrase', false), false);
  assert.equal(setupLeaveBlocked('server-check', false), false);
  assert.equal(setupLeaveBlocked('discovery', false), true);
  assert.equal(setupLeaveBlocked('account', false), true);
  // An acknowledged provisioning is resumed by Finish later.
  assert.equal(setupLeaveBlocked('account', true), false);
  // A write behind a server check still blocks: the write owns the value.
  assert.equal(
    setupLeaveBlocked(
      setupBusy({
        account: true,
        discovery: false,
        serverCheck: true,
        phrase: false,
      }),
      false,
    ),
    true,
  );
});
