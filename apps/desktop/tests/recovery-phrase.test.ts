import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';
import { installDom } from './lib/dom-harness';
import { BIP39_ENGLISH } from '../src/bip39-english';
import {
  completions,
  normalizeToken,
  recallMatches,
  recallPositions,
  spreadTokens,
  tokenComplete,
  tokenKind,
  tokenLabel,
  tokenProblem,
} from '../src/recovery-phrase';
import {
  RECOVERY_SNOOZE_DAYS,
  RECOVERY_TEST_INTERVAL_DAYS,
  recordRecoveryPhraseTest,
  recoveryPhraseTestDue,
  recoveryPhraseTestStatus,
  recoveryPhraseTests,
  snoozeRecoveryPhraseReminder,
  subscribeRecoveryPhraseTests,
} from '../src/recovery-phrase-tests';
import {
  DeviceAlertRegistry,
  devicesAlertSummary,
} from '../src/screens/device-alert';
import { FIXTURE } from '../src/fixture';

installDom({ url: 'http://localhost/' });
test.afterEach(() => window.localStorage.clear());

const DAY = 24 * 60 * 60 * 1000;
const KEY = `10${'1'.repeat(64)}`;
const PHRASE =
  'cash 771 seed 674 glad 578 acquire 482 peace 385 door 289 three 192 level 96 cake';

test('the word list is the published BIP-39 English list', () => {
  assert.equal(BIP39_ENGLISH.length, 2048);
  assert.equal(
    createHash('sha256')
      .update(`${BIP39_ENGLISH.join('\n')}\n`)
      .digest('hex'),
    '2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda',
  );
});

test('tokens alternate word and number, and are labelled within their kind', () => {
  assert.deepEqual([0, 1, 2, 15, 16].map(tokenKind), [
    'word',
    'number',
    'word',
    'number',
    'word',
  ]);
  assert.equal(tokenLabel(0), 'Word 1');
  assert.equal(tokenLabel(6), 'Word 4');
  assert.equal(tokenLabel(11), 'Number 6');
  assert.equal(tokenLabel(16), 'Word 9');
});

test('each field reports the problem its position can detect', () => {
  // Words: in the dictionary, still being typed, unfinished, unknown, or a
  // number where a word belongs.
  assert.equal(tokenProblem(0, 'cage', false), null);
  assert.equal(tokenProblem(0, 'adv', false), null);
  assert.equal(tokenProblem(0, 'adv', true), 'unfinished-word');
  assert.equal(tokenProblem(0, 'advx', false), 'unknown-word');
  assert.equal(tokenProblem(0, 'Cage', false), 'unknown-word');
  assert.equal(tokenProblem(2, '32', false), 'expected-word');
  // Numbers: in range, out of range, or a word where a number belongs.
  assert.equal(tokenProblem(1, '0', true), null);
  assert.equal(tokenProblem(1, '8191', true), null);
  assert.equal(tokenProblem(1, '8192', false), 'number-range');
  assert.equal(tokenProblem(1, '-1', false), 'number-range');
  assert.equal(tokenProblem(1, 'cage', false), 'expected-number');
  assert.equal(tokenProblem(1, '', true), null);
  assert.equal(tokenComplete(1, ''), false);
  assert.equal(tokenComplete(0, 'cag'), false);
  assert.equal(tokenComplete(0, 'cage'), true);
  assert.equal(normalizeToken(0, ' Ca ge '), 'cage');
  assert.equal(normalizeToken(1, ' 3 2 '), '32');
});

test('completion lists dictionary words by prefix, in order', () => {
  assert.deepEqual(completions('advi'), ['advice']);
  assert.deepEqual(completions('zo'), ['zone', 'zoo']);
  assert.deepEqual(completions('ab', 3), ['abandon', 'ability', 'able']);
  assert.deepEqual(completions('qq'), []);
  assert.deepEqual(completions('3'), []);
  assert.deepEqual(completions(''), []);
});

test('a pasted phrase spreads over the fields from where it lands', () => {
  const empty = Array.from({ length: 17 }, () => '');
  const all = spreadTokens(empty, 0, `  ${PHRASE.replaceAll(' ', '\n')} extra`);
  assert.equal(all.join(' '), PHRASE);
  const tail = spreadTokens(empty, 14, 'Level 96 cake extra');
  // Words are lower-cased; the token past the last field is dropped.
  assert.deepEqual(tail.slice(14), ['level', '96', 'cake']);
});

test('the recall check asks three distinct positions with both kinds', () => {
  for (let seed = 0; seed < 200; seed++) {
    let state = seed;
    const random = () => {
      state = (state * 1103515245 + 12345) % 2 ** 31;
      return state / 2 ** 31;
    };
    const positions = recallPositions(random);
    assert.equal(new Set(positions).size, 3);
    assert.deepEqual(
      positions,
      [...positions].sort((a, b) => a - b),
    );
    assert.ok(positions.every((index) => index >= 0 && index < 17));
    assert.equal(new Set(positions.map(tokenKind)).size, 2);
  }
  // Numbers compare by value, as the agent parses them.
  assert.ok(recallMatches(PHRASE, 1, '0771'));
  assert.ok(recallMatches(PHRASE, 0, 'Cash'));
  assert.ok(!recallMatches(PHRASE, 0, 'cast'));
  assert.ok(!recallMatches(PHRASE, 1, '772'));
  assert.ok(!recallMatches(PHRASE, 1, ''));
});

test('a phrase is never tested, then current, then overdue until snoozed or tested', () => {
  const now = Date.UTC(2026, 9, 11);
  let notified = 0;
  const unsubscribe = subscribeRecoveryPhraseTests(() => notified++);
  const status = (at = now) =>
    recoveryPhraseTestStatus(recoveryPhraseTests(), KEY, at);
  assert.deepEqual(status(), { state: 'never', snoozed: false });
  assert.ok(recoveryPhraseTestDue(status()));

  recordRecoveryPhraseTest(KEY, now);
  assert.deepEqual(status(), { state: 'current', testedAt: now });
  assert.ok(!recoveryPhraseTestDue(status()));
  // The record is per backup key.
  assert.deepEqual(
    recoveryPhraseTestStatus(recoveryPhraseTests(), `10${'2'.repeat(64)}`, now),
    { state: 'never', snoozed: false },
  );

  const later = now + (RECOVERY_TEST_INTERVAL_DAYS + 1) * DAY;
  assert.deepEqual(status(later), {
    state: 'overdue',
    testedAt: now,
    snoozed: false,
  });
  assert.ok(recoveryPhraseTestDue(status(later)));
  snoozeRecoveryPhraseReminder(KEY, later);
  assert.ok(!recoveryPhraseTestDue(status(later)));
  assert.ok(
    recoveryPhraseTestDue(status(later + (RECOVERY_SNOOZE_DAYS + 1) * DAY)),
  );
  // A new match clears the snooze and restarts the interval.
  recordRecoveryPhraseTest(KEY, later);
  assert.deepEqual(recoveryPhraseTests()[KEY], {
    testedAt: later,
  });
  assert.equal(notified, 3);
  // Unchanged storage keeps the snapshot's identity for React.
  assert.equal(recoveryPhraseTests(), recoveryPhraseTests());
  // Only times are stored.
  assert.doesNotMatch(
    window.localStorage.getItem('foks.recoveryPhraseTests') ?? '',
    /phrase|cash/,
  );
  unsubscribe();
});

test('the Devices dot lists every reason, untested phrases included', () => {
  const store = FIXTURE.stores.find((entry) => entry.kind === 'account');
  assert.ok(store && store.kind === 'account');
  const registry = new DeviceAlertRegistry();
  const backup = {
    backupAlias: 'paper-backup',
    accountAlias: store.account,
    backupId: `10${'4'.repeat(64)}`,
  };
  registry.reportRecoveryPhrase(store.id, [backup]);
  const now = Date.UTC(2026, 9, 11);
  assert.deepEqual(
    devicesAlertSummary(FIXTURE, registry.getSnapshot(), {}, now),
    { description: 'Recovery phrase paper-backup has never been tested' },
  );
  registry.reportPairingOffer(store.id, true);
  assert.deepEqual(
    devicesAlertSummary(FIXTURE, registry.getSnapshot(), {}, now),
    {
      description:
        'A pairing offer is open, and recovery phrase paper-backup has never been tested',
    },
  );
  registry.reportPairingOffer(store.id, false);

  const tested = {
    [backup.backupId]: { testedAt: now },
  };
  assert.equal(
    devicesAlertSummary(FIXTURE, registry.getSnapshot(), tested, now),
    null,
  );
  const overdue = now + (RECOVERY_TEST_INTERVAL_DAYS + 1) * DAY;
  assert.deepEqual(
    devicesAlertSummary(FIXTURE, registry.getSnapshot(), tested, overdue),
    { description: 'Recovery phrase paper-backup is due for a test' },
  );
  assert.equal(
    devicesAlertSummary(
      FIXTURE,
      registry.getSnapshot(),
      {
        [backup.backupId]: {
          testedAt: now,
          snoozedUntil: overdue + DAY,
        },
      },
      overdue,
    ),
    null,
  );

  // Forgetting the account drops its phrases from the dot as well.
  registry.forgetRecoveryPhrase(store.id);
  assert.equal(
    devicesAlertSummary(FIXTURE, registry.getSnapshot(), {}, now),
    null,
  );
});
