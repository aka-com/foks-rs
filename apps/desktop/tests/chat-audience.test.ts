/**
 * Unit tests for a channel's audience: its readers and writers on the team
 * roster, and the sentences the header, the empty conversation and the info
 * panel draw from them.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { FIXTURE } from '../src/fixture';
import {
  audienceLine,
  audienceSentence,
  channelAudience,
  channelReadRole,
  channelRoles,
  everyonePosts,
  everyoneReads,
  roleAudience,
  type ChannelAudience,
} from '../src/chat/audience';
import { channelRoster } from '../src/chat/mentions';
import { partyName, type AgentSnapshot, type Party } from '../src/model';
import type { ChatChannel } from '../src/chat-contract';

function channel(
  read_role: string,
  write_role = read_role,
  admin = false,
): ChatChannel {
  return {
    id: 'c1'.repeat(16),
    name: 'oncall',
    description: null,
    admin,
    readable: true,
    writable: true,
    read_role,
    write_role,
  };
}

/** Engineering: an Owner, two Admins, two Members and a federated team. */
function audience(
  of: ChatChannel,
  snapshot: AgentSnapshot = FIXTURE,
): ChannelAudience {
  const result = channelAudience(snapshot, 'team:eng', of);
  assert.ok(result);
  return result;
}

const names = (parties: readonly Party[]) => parties.map(partyName);
const readers = (of: ChannelAudience) =>
  of.readers.map(({ party, post }) => `${partyName(party)} ${post}`);

test('an admins-only channel counts its admins and owners, not the roster', () => {
  const admins = audience(channel('Admin', 'Admin', true));
  assert.deepEqual(readers(admins), [
    'sam.ortiz true',
    'priya.n true',
    'vitalik true',
  ]);
  // The federated team is in neither list: chat serves direct user members.
  assert.deepEqual(names(admins.others), ['dana.okafor', 'deploy-bot']);
  assert.equal(everyoneReads(admins), false);
  assert.equal(audienceLine(admins, undefined), '3 readers');
  assert.equal(audienceLine(admins, 'Pager rota'), '3 readers · Pager rota');
  assert.equal(
    audienceSentence(admins, 'Engineering'),
    'Only the 3 admins and owners of Engineering can read it.',
  );
  assert.equal(
    roleAudience(admins.read, everyoneReads(admins)),
    'Admins and owners',
  );
  // The admins-only tier is a floor: a Member read role on it, as a Go
  // client can write, still admits only admins and owners.
  assert.deepEqual(
    readers(audience(channel('Member (0)', 'Member (0)', true))),
    readers(admins),
  );
  assert.deepEqual(channelRoles(channel('Member (0)', 'Member (0)', true)), {
    read: { role: 'Admin' },
    write: { role: 'Admin' },
  });
});

test('a channel every member reads marks who can post', () => {
  const team = audience(channel('Member (0)', 'Admin'));
  assert.deepEqual(readers(team), [
    'sam.ortiz true',
    'priya.n true',
    'vitalik true',
    'dana.okafor false',
    'deploy-bot false',
  ]);
  assert.equal(everyoneReads(team), true);
  assert.equal(everyonePosts(team), false);
  assert.equal(
    roleAudience(team.read, everyoneReads(team)),
    'Everyone on the team',
  );
  assert.equal(
    roleAudience(team.write, everyonePosts(team)),
    'Admins and owners',
  );
  // The description leads; without one, the member count.
  assert.equal(audienceLine(team, 'Daily notes'), 'Daily notes');
  assert.equal(audienceLine(team, undefined), '5 members');
  assert.equal(
    audienceSentence(team, 'Engineering'),
    'Only the 5 members of Engineering can read it.',
  );
});

test('a banded read role leaves out members below its band', () => {
  const lee: Party = {
    ...FIXTURE.parties.find((party) => party.username === 'dana.okafor')!,
    username: 'lee',
    party_id_hex: '01' + 'ee'.repeat(32),
    destination_role: { role: 'Member', visibility: 2 },
  };
  const snapshot = { ...FIXTURE, parties: [...FIXTURE.parties, lee] };
  const banded = audience(channel('Member (1)', 'Admin'), snapshot);
  assert.deepEqual(readers(banded), [
    'sam.ortiz true',
    'priya.n true',
    'vitalik true',
    'lee false',
  ]);
  assert.deepEqual(names(banded.others), ['dana.okafor', 'deploy-bot']);
  assert.equal(audienceLine(banded, undefined), '4 readers');
  assert.equal(
    audienceSentence(banded, 'Engineering'),
    'Only the 4 members of Engineering at visibility level 1 and above can read it.',
  );
  assert.equal(
    roleAudience(banded.read, everyoneReads(banded)),
    'Members at visibility level 1 and above',
  );
  // The write role is at least the read role.
  assert.deepEqual(channelRoles(channel('Member (1)', 'Member (0)'))?.write, {
    role: 'Member',
    visibility: 1,
  });
});

test('a user party of another host is not counted', () => {
  const remote: Party = {
    ...FIXTURE.parties.find((party) => party.username === 'priya.n')!,
    username: 'kai',
    party_id_hex: '01' + 'dd'.repeat(32),
    locally_manageable: false,
    scoped_host_id_hex: '9f31c2aa07',
  };
  const snapshot = { ...FIXTURE, parties: [...FIXTURE.parties, remote] };
  const admins = audience(channel('Admin', 'Admin', true), snapshot);
  // The server admits only direct members of the team's own host.
  assert.deepEqual(readers(admins), [
    'sam.ortiz true',
    'priya.n true',
    'vitalik true',
  ]);
  assert.deepEqual(names(admins.others), ['dana.okafor', 'deploy-bot']);
  assert.equal(audienceLine(admins, undefined), '3 readers');
});

test('the composer roster and the reader list share one read rule', () => {
  // A Member read role on an admins-only channel admits admins and owners.
  const need = channelReadRole(channel('Member (0)', 'Member (0)', true));
  assert.deepEqual(need, { role: 'Admin' });
  const roster = channelRoster(
    [
      { username: 'sam.ortiz', role: { role: 'Owner' } },
      { username: 'priya.n', role: { role: 'Admin' } },
      { username: 'dana.okafor', role: { role: 'Member', visibility: 0 } },
    ],
    need,
  );
  assert.deepEqual(
    roster.members.map((member) => member.username),
    ['sam.ortiz', 'priya.n'],
  );
  assert.equal(roster.excluded, 1);
  assert.deepEqual(channelReadRole(channel('Member (1)')), {
    role: 'Member',
    visibility: 1,
  });
  assert.equal(channelReadRole(channel('Reader')), null);
});

test('an owners-only channel names its owners', () => {
  const owners = audience(channel('Owner'));
  assert.deepEqual(readers(owners), ['sam.ortiz true']);
  assert.equal(
    audienceSentence(owners, 'Engineering'),
    'Only the 1 owner of Engineering can read it.',
  );
  assert.equal(roleAudience(owners.read, false), 'Owners');
});

test('no audience is stated without a roster or for a role the contract does not write', () => {
  const unloaded = { ...FIXTURE, parties: [] };
  assert.equal(
    channelAudience(unloaded, 'team:eng', channel('Admin', 'Admin', true)),
    undefined,
  );
  assert.equal(
    channelAudience(FIXTURE, 'team:eng', channel('Reader')),
    undefined,
  );
  assert.equal(channelRoles(channel('Reader')), undefined);
  assert.equal(audienceLine(undefined, undefined), '');
  assert.equal(audienceLine(undefined, 'Daily notes'), 'Daily notes');
  assert.equal(
    audienceSentence(undefined, 'Engineering'),
    'Only the members of Engineering can read it.',
  );
  // Without a roster, the role alone is stated.
  assert.equal(
    roleAudience({ role: 'Member', visibility: 0 }),
    'Everyone on the team',
  );
});
