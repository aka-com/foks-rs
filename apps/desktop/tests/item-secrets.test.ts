/**
 * The details panel's plaintext reducer: which transitions clear a revealed
 * value or an edit draft, which keep a draft, and that `PLAINTEXT_FIELDS`
 * names every field that can hold item plaintext.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { FIXTURE } from '../src/fixture';
import {
  CHANGED_ON_SERVER,
  EMPTY,
  PLAINTEXT_FIELDS,
  editorBound,
  itemSecrets,
  plaintextOf,
} from '../src/screens/item-secrets';
import type { ItemSecrets, SecretsAction } from '../src/screens/item-secrets';

const ITEM = FIXTURE.items.find((item) => item.kind === 'Secret')!;
const KEY = `${ITEM.store}|${ITEM.path}|${ITEM.version}`;
const SCOPE = 'scope-a';
const SECRET = 'plaintext-sentinel';
const DRAFT = 'draft-sentinel';

function run(state: ItemSecrets, ...actions: SecretsAction[]): ItemSecrets {
  return actions.reduce(itemSecrets, state);
}

/** A state with a revealed value and an open, edited editor. */
function populated(generation = 0): ItemSecrets {
  return run(
    EMPTY,
    {
      type: 'open-editor',
      value: SECRET,
      loginLines: [0],
      binding: { item: ITEM, scope: SCOPE, generation },
    },
    { type: 'edit', value: DRAFT },
    { type: 'show-password', shown: true },
    { type: 'replacement', path: '/tmp/replacement', error: null },
    { type: 'reveal-start', key: KEY },
    { type: 'reveal-done', key: KEY, value: SECRET },
    { type: 'show-rows', ids: ['password', 'user'], keep: false },
  );
}

test('the populated fixture holds plaintext in every declared field', () => {
  const state = populated();
  assert.deepEqual(plaintextOf(state), [SECRET, DRAFT, SECRET]);
  assert.equal(PLAINTEXT_FIELDS.length, plaintextOf(state).length);
});

test('PLAINTEXT_FIELDS names every field that holds plaintext', () => {
  const state = populated();
  const stripped = structuredClone(state) as unknown as Record<
    string,
    Record<string, unknown>
  >;
  for (const path of PLAINTEXT_FIELDS) {
    const [owner, field] = path.split('.') as [string, string];
    delete stripped[owner][field];
  }
  const json = JSON.stringify(stripped);
  assert.ok(!json.includes(SECRET));
  assert.ok(!json.includes(DRAFT));
});

test('conceal-signal leaves no plaintext', () => {
  const state = run(populated(), { type: 'conceal-signal' });
  assert.deepEqual(plaintextOf(state), []);
  assert.equal(state.read, null);
  assert.equal(state.editor, null);
  assert.ok(!JSON.stringify(state).includes(SECRET));
});

test('an item change the editor is no longer bound to leaves no plaintext', () => {
  for (const change of [
    { scope: 'scope-b', generation: 0 },
    { scope: SCOPE, generation: 1 },
  ]) {
    const state = run(populated(), {
      type: 'item-changed',
      ...change,
    });
    assert.deepEqual(plaintextOf(state), []);
    assert.equal(state.editor, null);
    assert.equal(state.editError, null);
    assert.equal(state.binaryFile, false);
  }
});

test('an item change the editor is still bound to keeps the draft and says so', () => {
  const state = run(populated(), {
    type: 'item-changed',
    scope: SCOPE,
    generation: 0,
  });
  assert.equal(state.read, null);
  assert.equal(state.editor?.value, DRAFT);
  assert.equal(state.editor?.binding.item, ITEM);
  assert.equal(state.editor?.passwordShown, false);
  assert.equal(state.editError, CHANGED_ON_SERVER);
});

test('conceal-inactive keeps an unsaved draft but covers it', () => {
  const state = run(populated(), { type: 'conceal-inactive', file: false });
  assert.equal(state.read, null);
  assert.deepEqual(state.revealed, []);
  assert.equal(state.editor?.value, DRAFT);
  assert.equal(state.editor?.concealed, true);
  assert.equal(state.editor?.passwordShown, false);
  const shown = run(state, { type: 'unconceal-draft' });
  assert.equal(shown.editor?.concealed, false);
  assert.equal(shown.editor?.value, DRAFT);
});

test('conceal-inactive does not cover a file replacement, which holds no text', () => {
  const file = run(populated(), { type: 'conceal-inactive', file: true });
  assert.equal(file.editor?.concealed, false);
  assert.equal(file.editor?.passwordShown, false);
  const binary = run(
    populated(),
    { type: 'not-text' },
    { type: 'conceal-inactive', file: false },
  );
  assert.equal(binary.binaryFile, true);
  assert.equal(binary.editor?.concealed, false);
});

test('close-editor clears the editor and its error but not a binary fact', () => {
  const state = run(
    populated(),
    { type: 'not-text' },
    { type: 'edit-error', message: 'refused' },
    { type: 'close-editor' },
  );
  assert.equal(state.editor, null);
  assert.equal(state.editError, null);
  assert.equal(state.binaryFile, true);
  assert.deepEqual(plaintextOf(state), []);
});

test('hide-value and read-blocked drop the revealed value', () => {
  for (const action of [
    { type: 'hide-value' },
    { type: 'read-blocked' },
  ] as const) {
    const state = run(populated(), action);
    assert.equal(state.read, null);
    assert.deepEqual(state.revealed, []);
  }
  assert.equal(
    run(populated(), { type: 'read-blocked' }).editor?.passwordShown,
    false,
  );
});

test('hide-row masks one row and keeps the text for the others', () => {
  const state = run(populated(), { type: 'hide-row', id: 'user' });
  assert.deepEqual(state.revealed, ['password']);
  assert.equal(state.read?.state, 'shown');
});

test('generation-changed drops only a read still loading', () => {
  const loading = run(EMPTY, { type: 'reveal-start', key: KEY });
  assert.equal(run(loading, { type: 'generation-changed' }).read, null);
  const shown = populated();
  assert.equal(run(shown, { type: 'generation-changed' }), shown);
});

test('opening an editor drops the revealed value and starts from the stored value', () => {
  const state = run(
    populated(),
    { type: 'close-editor' },
    {
      type: 'open-editor',
      value: SECRET,
      loginLines: null,
      binding: { item: ITEM, scope: SCOPE, generation: 0 },
    },
  );
  assert.equal(state.read, null);
  assert.equal(state.editor?.baseline, SECRET);
  assert.equal(state.editor?.replacementPath, null);
  assert.equal(state.editor?.passwordShown, false);
});

test('a restored draft has no baseline, so it counts as unsaved', () => {
  const state = run(EMPTY, {
    type: 'restore-draft',
    value: DRAFT,
    loginLines: null,
    binding: { item: ITEM, scope: SCOPE, generation: 2 },
  });
  assert.equal(state.editor?.baseline, null);
  assert.ok(editorBound(state.editor, SCOPE, 2));
  assert.ok(!editorBound(state.editor, SCOPE, 3));
  assert.ok(!editorBound(null, SCOPE, 2));
});
