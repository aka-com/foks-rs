import assert from 'node:assert/strict';
import test from 'node:test';

import {
  completedFirstRunSteps,
  decodeFirstRunCheckpoint,
  encodeFirstRunCheckpoint,
  initialFirstRun,
  transitionFirstRun,
} from '../src/first-run-state';

const checked = {
  profile: 'work',
  acceptance: 'inserted' as const,
  lookupName: 'foks.example',
  canonicalName: 'foks.example',
  hostId: `02${'2'.repeat(64)}`,
  chain: 9,
  epoch: 12,
};

const namedGroup = {
  name: 'Household',
  kind: 'named' as const,
  alias: 'household',
  teamIdHex: `03${'4'.repeat(64)}`,
};

function parsedCheckpoint(value: string): Record<string, unknown> {
  const parsed: unknown = JSON.parse(value);
  assert.ok(parsed && typeof parsed === 'object' && !Array.isArray(parsed));
  return parsed as Record<string, unknown>;
}

test('first-run re-entry clears completed server and account facts', () => {
  let state = initialFirstRun('invited', 'boot');
  state = transitionFirstRun(state, { type: 'initialize' });
  state = transitionFirstRun(state, { type: 'choose', path: 'invited' });
  state = transitionFirstRun(state, {
    type: 'profile-checked',
    address: 'foks.example',
    profile: checked,
  });
  state = transitionFirstRun(state, {
    type: 'account-complete',
    alias: 'personal',
    username: 'sol',
    deviceName: 'Sol Mac',
  });
  state = transitionFirstRun(state, { type: 'passphrase-set' });
  state = transitionFirstRun(state, { type: 'go', state: 'waiting' });
  state = transitionFirstRun(state, { type: 'reenter' });
  assert.equal(state.state, 'who');
  assert.equal(state.profile, undefined);
  assert.equal(state.account, undefined);
  assert.equal(state.passphraseSet, false);
  assert.equal(completedFirstRunSteps(state), 1);
});

test('initialization enters managed local setup when the refreshed profile is ready', () => {
  const state = transitionFirstRun(initialFirstRun('invited', 'boot'), {
    type: 'initialize',
    managedLocal: true,
  });
  assert.equal(state.state, 'local');
  assert.equal(state.path, 'own');
  assert.equal(state.managedLocal, true);
  assert.equal(state.initialized, true);
});

test('the managed local path reuses its authenticated profile and stops after recovery', () => {
  let state = initialFirstRun('own', 'local');
  state = transitionFirstRun(state, {
    type: 'managed-profile-selected',
    address: 'localhost:4430',
    profile: { ...checked, profile: 'local', acceptance: 'unchanged' },
  });
  assert.equal(state.state, 'account');
  assert.equal(state.managedLocal, true);
  state = transitionFirstRun(state, {
    type: 'account-complete',
    alias: 'personal',
    username: 'sol',
    deviceName: 'Sol Mac',
  });
  state = transitionFirstRun(state, { type: 'backup-committed' });
  state = transitionFirstRun(state, { type: 'finish-local', skipped: false });
  assert.equal(state.state, 'local-done');
  assert.equal(state.backupCommitted, true);
  assert.ok(decodeFirstRunCheckpoint(encodeFirstRunCheckpoint(state)));
});

test('skipping a revisited local recovery step retains completed protection', () => {
  const protectedState = {
    ...initialFirstRun('own', 'protect'),
    managedLocal: true,
    profile: { ...checked, profile: 'local', acceptance: 'unchanged' as const },
    serverAddress: 'localhost:4430',
    account: { alias: 'personal', username: 'sol', deviceName: 'Sol Mac' },
    backupCommitted: true,
  };
  const state = transitionFirstRun(protectedState, {
    type: 'finish-local',
    skipped: true,
  });
  assert.equal(state.protectSkipped, false);
  assert.equal(state.backupCommitted, true);
  assert.ok(decodeFirstRunCheckpoint(encodeFirstRunCheckpoint(state)));
});

test('choosing another server leaves all managed local account facts behind', () => {
  let selected = transitionFirstRun(initialFirstRun('own', 'local'), {
    type: 'managed-profile-selected',
    address: 'localhost:4430',
    profile: { ...checked, profile: 'local', acceptance: 'unchanged' },
  });
  selected = transitionFirstRun(selected, {
    type: 'account-complete',
    alias: 'personal',
    username: 'sol',
    deviceName: 'Sol Mac',
  });
  selected = transitionFirstRun(selected, { type: 'backup-committed' });
  const other = transitionFirstRun(selected, { type: 'choose', path: 'own' });
  assert.equal(other.state, 'address');
  assert.equal(other.managedLocal, false);
  assert.equal(other.profile, undefined);
  assert.equal(other.account, undefined);
  assert.equal(other.backupCommitted, false);
});

test('the checkpoint encoder cannot persist any first-run secret draft', () => {
  const state = {
    ...initialFirstRun('invited', 'phrase'),
    profile: checked,
    serverAddress: 'foks.example',
    account: { alias: 'personal', username: 'sol', deviceName: 'Sol Mac' },
    // Simulate future form fields accidentally being attached at runtime.
    invite: 'INVITE-SENTINEL',
    passphrase: 'PASSPHRASE-SENTINEL',
    recoveryPhrase: 'RECOVERY-SENTINEL',
    backupPhrase: 'BACKUP-SENTINEL',
  };
  const encoded = encodeFirstRunCheckpoint(state);
  assert.doesNotMatch(
    encoded,
    /INVITE-SENTINEL|PASSPHRASE-SENTINEL|RECOVERY-SENTINEL|BACKUP-SENTINEL/,
  );
  assert.equal(
    decodeFirstRunCheckpoint(encoded)?.state,
    'protect',
    'a one-time phrase reveal resumes closed',
  );
});

test('skipped safeguards remain open and do not inflate the completed count', () => {
  const base = {
    ...initialFirstRun('own', 'checklist-own'),
    initialized: true,
    profile: checked,
    serverAddress: 'foks.example',
    account: { alias: 'personal', username: 'rae', deviceName: 'Rae Mac' },
    protectSkipped: true,
    groupSkipped: true,
  };
  assert.equal(completedFirstRunSteps(base), 3);
});

test('skipping a revisit cannot erase an already completed safeguard', () => {
  const protectedState = {
    ...initialFirstRun('own', 'protect'),
    initialized: true,
    profile: checked,
    serverAddress: 'foks.example',
    account: { alias: 'personal', username: 'rae', deviceName: 'Rae Mac' },
    passphraseSet: true,
  };
  const after = transitionFirstRun(protectedState, { type: 'skip-protect' });
  assert.equal(after.passphraseSet, true);
  assert.equal(after.protectSkipped, false);
  assert.equal(completedFirstRunSteps(after), 4);
});

test('skipping a group revisit retains the authenticated completed group', () => {
  const completed = {
    ...initialFirstRun('own', 'create-group'),
    initialized: true,
    profile: checked,
    serverAddress: 'foks.example',
    account: { alias: 'personal', username: 'rae', deviceName: 'Rae Mac' },
    passphraseSet: true,
    group: namedGroup,
  };
  const after = transitionFirstRun(completed, { type: 'skip-group' });
  assert.deepEqual(after.group, namedGroup);
  assert.equal(after.groupSkipped, false);
  assert.equal(completedFirstRunSteps(after), 5);
});

test('checkpoint decoding rejects unknown versions and malformed authenticated display facts', () => {
  assert.equal(decodeFirstRunCheckpoint('{"version":2}'), null);
  const encoded = encodeFirstRunCheckpoint({
    ...initialFirstRun('own', 'checked'),
    profile: checked,
    serverAddress: 'foks.example',
  });
  assert.equal(
    decodeFirstRunCheckpoint(encoded)?.profile?.hostId,
    checked.hostId,
  );
  for (const acceptance of ['advanced', 'unchanged'] as const) {
    const resumed = decodeFirstRunCheckpoint(
      encodeFirstRunCheckpoint({
        ...initialFirstRun('own', 'checked'),
        profile: { ...checked, profile: 'local', acceptance },
        serverAddress: 'localhost',
      }),
    );
    assert.equal(resumed?.profile?.profile, 'local');
    assert.equal(resumed?.profile?.acceptance, acceptance);
  }
  assert.equal(
    decodeFirstRunCheckpoint(encoded.replace('"chain":9', '"chain":-1.5')),
    null,
  );
  assert.equal(
    decodeFirstRunCheckpoint(encoded.replace('"chain":9', '"chain":-1')),
    null,
  );
  assert.equal(
    decodeFirstRunCheckpoint(
      JSON.stringify({ ...JSON.parse(encoded), serverAddress: 7 }),
    ),
    null,
  );
  assert.equal(
    decodeFirstRunCheckpoint(
      JSON.stringify({ ...JSON.parse(encoded), unexpected: true }),
    ),
    null,
  );
  assert.equal(
    decodeFirstRunCheckpoint(
      encoded.replace('"epoch":12', '"epoch":12,"unexpected":true'),
    ),
    null,
  );
  assert.equal(
    decodeFirstRunCheckpoint(
      encodeFirstRunCheckpoint(initialFirstRun('own', 'done')),
    ),
    null,
  );
});

test('checkpoint decoding rejects wrong entity kinds, explicit nulls and incoherent completion flags', () => {
  const checkedState = {
    ...initialFirstRun('own', 'checked'),
    profile: checked,
    serverAddress: 'foks.example',
  };
  const checkedObject = parsedCheckpoint(
    encodeFirstRunCheckpoint(checkedState),
  );
  assert.equal(
    decodeFirstRunCheckpoint(
      JSON.stringify({
        ...checkedObject,
        profile: { ...checked, hostId: `01${'2'.repeat(64)}` },
      }),
    ),
    null,
  );
  for (const key of ['profile', 'account', 'group'] as const) {
    assert.equal(
      decodeFirstRunCheckpoint(
        JSON.stringify({
          ...parsedCheckpoint(
            encodeFirstRunCheckpoint(initialFirstRun('own', 'who')),
          ),
          [key]: null,
        }),
      ),
      null,
      `${key}: null must not behave like an omitted optional fact`,
    );
  }

  const account = { alias: 'personal', username: 'rae', deviceName: 'Mac' };
  const coherent = parsedCheckpoint(
    encodeFirstRunCheckpoint({
      ...initialFirstRun('own', 'done'),
      profile: checked,
      serverAddress: 'foks.example',
      account,
      group: namedGroup,
    }),
  );
  assert.ok(decodeFirstRunCheckpoint(JSON.stringify(coherent)));
  assert.equal(
    decodeFirstRunCheckpoint(
      JSON.stringify({
        ...coherent,
        group: { ...namedGroup, teamIdHex: `14${'4'.repeat(64)}` },
      }),
    ),
    null,
  );
  assert.equal(
    decodeFirstRunCheckpoint(
      JSON.stringify({
        ...coherent,
        group: {
          ...namedGroup,
          kind: 'adhoc',
          teamIdHex: `03${'4'.repeat(64)}`,
        },
      }),
    ),
    null,
  );

  for (const patch of [
    { passphraseSet: true },
    { backupCommitted: true },
    { protectSkipped: true },
    { group: namedGroup },
    { groupSkipped: true },
    { added: true },
  ]) {
    assert.equal(
      decodeFirstRunCheckpoint(
        JSON.stringify({
          ...parsedCheckpoint(
            encodeFirstRunCheckpoint(initialFirstRun('own', 'who')),
          ),
          ...patch,
        }),
      ),
      null,
    );
  }
  assert.equal(
    decodeFirstRunCheckpoint(
      JSON.stringify({ ...coherent, groupSkipped: true }),
    ),
    null,
  );
  assert.equal(
    decodeFirstRunCheckpoint(
      JSON.stringify({
        ...coherent,
        passphraseSet: true,
        protectSkipped: true,
      }),
    ),
    null,
  );
  assert.equal(
    decodeFirstRunCheckpoint(
      JSON.stringify({
        ...parsedCheckpoint(
          encodeFirstRunCheckpoint(initialFirstRun('invited', 'who')),
        ),
        returning: true,
      }),
    ),
    null,
  );
});

test('invited and own completion transitions remain distinct and resumable', () => {
  const invited = transitionFirstRun(initialFirstRun('invited', 'waiting'), {
    type: 'group-discovered',
    group: {
      name: 'Engineering',
      kind: 'named',
      alias: 'eng',
      teamIdHex: `03${'3'.repeat(64)}`,
    },
  });
  assert.deepEqual(
    { state: invited.state, added: invited.added },
    { state: 'added', added: true },
  );

  const own = transitionFirstRun(initialFirstRun('own', 'create-group'), {
    type: 'group-complete',
    group: {
      name: 'Household',
      kind: 'adhoc',
      alias: 'household',
      teamIdHex: `14${'4'.repeat(64)}`,
    },
  });
  assert.deepEqual(
    { state: own.state, group: own.group },
    {
      state: 'done',
      group: {
        name: 'Household',
        kind: 'adhoc',
        alias: 'household',
        teamIdHex: `14${'4'.repeat(64)}`,
      },
    },
  );
});

test('setting a passphrase after skipping leaves a checkpoint that still decodes', () => {
  // `skip-protect` then `passphrase-set` used to hold both `protectSkipped`
  // and `passphraseSet`, a combination the decoder rejects — so the next
  // launch threw the entire first run away with no message.
  const atProtect = (): ReturnType<typeof initialFirstRun> => {
    let state = initialFirstRun('own', 'boot');
    state = transitionFirstRun(state, { type: 'initialize' });
    state = transitionFirstRun(state, { type: 'choose', path: 'own' });
    state = transitionFirstRun(state, {
      type: 'profile-checked',
      address: 'foks.example',
      profile: checked,
    });
    return transitionFirstRun(state, {
      type: 'account-complete',
      alias: 'personal',
      username: 'sol',
      deviceName: 'Sol Mac',
    });
  };

  let state = transitionFirstRun(atProtect(), { type: 'skip-protect' });
  assert.equal(state.protectSkipped, true);
  assert.ok(decodeFirstRunCheckpoint(encodeFirstRunCheckpoint(state)));

  state = transitionFirstRun(state, { type: 'go', state: 'protect' });
  state = transitionFirstRun(state, { type: 'passphrase-set' });
  assert.equal(state.passphraseSet, true);
  assert.equal(state.protectSkipped, false);
  assert.ok(
    decodeFirstRunCheckpoint(encodeFirstRunCheckpoint(state)),
    'a passphrase set after a skip must still round-trip',
  );

  // Committing a backup already cleared the flag; the two transitions agree now.
  let viaBackup = transitionFirstRun(atProtect(), { type: 'skip-protect' });
  viaBackup = transitionFirstRun(viaBackup, { type: 'backup-committed' });
  assert.equal(viaBackup.protectSkipped, false);
  assert.ok(decodeFirstRunCheckpoint(encodeFirstRunCheckpoint(viaBackup)));
});
