import assert from 'node:assert/strict';
import test from 'node:test';
import type { Bridge } from '../src/bridge';
import type { TeamStore } from '../src/model';
import { membershipResume } from '../src/screens/groups/operation-controller';
import { attemptMutation } from '../src/commands/command-policy';

const store: TeamStore = {
  id: 'team:local',
  kind: 'team',
  name: 'Local',
  alias: 'local',
  server: 'profile',
  account: 'account',
  active: true,
  team_kind: 'named',
  team_id_hex: '12'.repeat(33),
};

test('membership recovery selects only the matching durable operation and never starts work by selecting', async () => {
  const calls: unknown[] = [];
  const bridge = {
    resumeGroupMemberAddition: async (request: unknown) => {
      calls.push(request);
    },
    resumeGroupMemberEdit: async (storeId: string) => {
      calls.push(storeId);
    },
  } as unknown as Bridge;
  assert.equal(
    membershipResume(bridge, store, {
      kind: 'team-member-addition',
      alias: 'another',
      target: 'person',
    }),
    undefined,
  );
  assert.equal(
    membershipResume(bridge, store, {
      kind: 'team-member-addition',
      alias: 'local',
    }),
    undefined,
  );
  assert.equal(
    membershipResume(bridge, store, {
      kind: 'team-creation',
      alias: 'local',
    }),
    undefined,
  );
  const addition = membershipResume(bridge, store, {
    kind: 'team-member-addition',
    alias: 'local',
    target: 'person',
  });
  assert.ok(addition);
  assert.deepEqual(calls, []);
  await addition.run();
  const edit = membershipResume(bridge, store, {
    kind: 'team-member-edit',
    alias: 'local',
  });
  assert.ok(edit);
  await edit.run();
  assert.deepEqual(calls, [
    { storeId: 'team:local', username: 'person' },
    'team:local',
  ]);
});

test('an ambiguous membership resume remains pending for explicit recovery without replay', async () => {
  let resumes = 0;
  const bridge = {
    resumeGroupMemberEdit: async () => {
      resumes++;
      throw {
        code: 'io',
        message: 'lost reply',
        ambiguous: true,
        retryable: true,
        fatal: false,
      };
    },
  } as unknown as Bridge;
  const resume = membershipResume(bridge, store, {
    kind: 'team-member-edit',
    alias: 'local',
  });
  assert.ok(resume);
  const result = await attemptMutation(
    { kind: 'resumable', operation: 'team-member-edit' },
    resume.run,
    async () => assert.fail('ambiguous resume reported as applied'),
  );
  assert.equal(resumes, 1);
  assert.equal(result.outcome, 'unknown');
});
