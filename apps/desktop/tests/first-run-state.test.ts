import assert from 'node:assert/strict';
import test from 'node:test';

import {
  completedFirstRunSteps,
  decodeFirstRunCheckpoint,
  encodeFirstRunCheckpoint,
  initialFirstRun,
  reconcileFirstRunCheckpoint,
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

test('re-entering first-run resets server and account state', () => {
  let state = initialFirstRun('invited', 'boot');
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
  assert.equal(completedFirstRunSteps(state), 0);
});

test('bootstrap is not persisted as onboarding progress', () => {
  const encoded = encodeFirstRunCheckpoint(initialFirstRun('invited', 'boot'));
  assert.equal(parsedCheckpoint(encoded).version, 2);
  assert.equal(parsedCheckpoint(encoded).state, 'who');
  assert.equal('initialized' in parsedCheckpoint(encoded), false);
  assert.equal(decodeFirstRunCheckpoint(encoded)?.state, 'who');
  assert.equal(
    decodeFirstRunCheckpoint(
      JSON.stringify({ ...parsedCheckpoint(encoded), initialized: true }),
    ),
    null,
  );
});

test('managed local setup completes through backup to local-done', () => {
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

test('skipping backup on completed local setup retains backupCommitted status', () => {
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

test('switching to another server resets managed local state', () => {
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

test('checkpoint encoding excludes sensitive draft fields', () => {
  const state = {
    ...initialFirstRun('invited', 'phrase'),
    profile: checked,
    serverAddress: 'foks.example',
    account: { alias: 'personal', username: 'sol', deviceName: 'Sol Mac' },
    // Verify transient secret fields are omitted from serialized checkpoints.
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
    'resuming from phrase reveal defaults to protect state',
  );
});

test('skipped steps are excluded from completed step count', () => {
  const base = {
    ...initialFirstRun('own', 'checklist-own'),
    profile: checked,
    serverAddress: 'foks.example',
    account: { alias: 'personal', username: 'rae', deviceName: 'Rae Mac' },
    protectSkipped: true,
  };
  assert.equal(completedFirstRunSteps(base), 2);
});

test('skipping protection step does not clear an already completed passphrase', () => {
  const protectedState = {
    ...initialFirstRun('own', 'protect'),
    profile: checked,
    serverAddress: 'foks.example',
    account: { alias: 'personal', username: 'rae', deviceName: 'Rae Mac' },
    passphraseSet: true,
  };
  const after = transitionFirstRun(protectedState, { type: 'skip-protect' });
  assert.equal(after.passphraseSet, true);
  assert.equal(after.protectSkipped, false);
  assert.equal(completedFirstRunSteps(after), 3);
});

test('checkpoint decoding rejects invalid versions and malformed profile fields', () => {
  assert.equal(decodeFirstRunCheckpoint('{"version":2}'), null);
  assert.equal(
    decodeFirstRunCheckpoint(
      JSON.stringify({
        ...parsedCheckpoint(
          encodeFirstRunCheckpoint(initialFirstRun('own', 'who')),
        ),
        version: 1,
        initialized: true,
      }),
    ),
    null,
  );
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
      encodeFirstRunCheckpoint(initialFirstRun('invited', 'added')),
    ),
    null,
  );
});

test('checkpoint decoding rejects invalid entity kinds, nulls, and conflicting completion flags', () => {
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
      `${key}: explicit null must not be treated as an omitted optional field`,
    );
  }

  const account = { alias: 'personal', username: 'rae', deviceName: 'Mac' };
  const coherent = parsedCheckpoint(
    encodeFirstRunCheckpoint({
      ...initialFirstRun('invited', 'added'),
      profile: checked,
      serverAddress: 'foks.example',
      account,
      group: namedGroup,
      added: true,
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

test('invited and own paths transition to distinct completion states', () => {
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

  const own = transitionFirstRun(initialFirstRun('own', 'protect'), {
    type: 'go',
    state: 'checklist-own',
  });
  assert.equal(own.state, 'checklist-own');
  assert.equal(own.group, undefined);
});

test('setting a passphrase after skipping produces a valid decodable checkpoint', () => {
  // Verify that setting a passphrase after skipping clears protectSkipped
  // so the checkpoint passes decoder validation.
  const atProtect = (): ReturnType<typeof initialFirstRun> => {
    let state = initialFirstRun('own', 'boot');
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

  // Verify backup-committed similarly clears protectSkipped.
  let viaBackup = transitionFirstRun(atProtect(), { type: 'skip-protect' });
  viaBackup = transitionFirstRun(viaBackup, { type: 'backup-committed' });
  assert.equal(viaBackup.protectSkipped, false);
  assert.ok(decodeFirstRunCheckpoint(encodeFirstRunCheckpoint(viaBackup)));
});

test('editing a checked server clears verification and downstream setup state', () => {
  const state = transitionFirstRun(
    {
      ...initialFirstRun('own', 'checked'),
      returning: true,
      profile: checked,
      serverAddress: 'foks.example',
    },
    { type: 'server-edited', address: 'different.example' },
  );
  assert.equal(state.state, 'address');
  assert.equal(state.profile, undefined);
  assert.equal(state.serverAddress, 'different.example');
  assert.equal(state.returning, true);
  const restored = decodeFirstRunCheckpoint(encodeFirstRunCheckpoint(state));
  assert.ok(restored);
  assert.equal(restored.state, 'address');
  assert.equal(restored.profile, undefined);
  assert.equal(restored.serverAddress, 'different.example');
});

test('incomplete authoritative inventory preserves onboarding progress', () => {
  const saved = {
    ...initialFirstRun('invited', 'added'),
    profile: checked,
    serverAddress: 'foks.example',
    account: { alias: 'personal', username: 'old-name', deviceName: 'Mac' },
    passphraseSet: true,
    group: namedGroup,
    added: true,
  };
  assert.equal(
    reconcileFirstRunCheckpoint(saved, {
      profile: 'unknown',
      account: 'unknown',
      group: 'unknown',
    }),
    saved,
  );
});

test('confirmed missing prerequisites rewind to the earliest valid resume point', () => {
  const saved = {
    ...initialFirstRun('invited', 'added'),
    profile: checked,
    serverAddress: 'foks.example',
    account: { alias: 'personal', username: 'rae', deviceName: 'Mac' },
    passphraseSet: true,
    group: namedGroup,
    added: true,
  };
  const profileMissing = reconcileFirstRunCheckpoint(saved, {
    profile: 'missing',
    account: 'unknown',
    group: 'unknown',
  });
  assert.equal(profileMissing.state, 'address');
  assert.equal(profileMissing.serverAddress, 'foks.example');
  assert.equal(profileMissing.profile, undefined);

  const accountMissing = reconcileFirstRunCheckpoint(saved, {
    profile: 'present',
    account: 'missing',
    group: 'unknown',
  });
  assert.equal(accountMissing.state, 'account');
  assert.equal(accountMissing.profile, checked);
  assert.equal(accountMissing.account, undefined);
  assert.equal(accountMissing.passphraseSet, false);

  const groupMissing = reconcileFirstRunCheckpoint(saved, {
    profile: 'present',
    account: 'present',
    group: 'missing',
  });
  assert.equal(groupMissing.state, 'waiting');
  assert.equal(groupMissing.account, saved.account);
  assert.equal(groupMissing.group, undefined);
  assert.equal(groupMissing.added, false);
});
