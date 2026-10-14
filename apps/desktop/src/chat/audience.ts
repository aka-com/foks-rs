/**
 * Who can read and post in a channel, from the team roster the snapshot
 * already holds and the channel's roles in the chat contract.
 *
 * The server admits a reader when the member's role is at or above both the
 * channel tier's floor (Admin for an admins-only channel) and the read role,
 * and a writer when it also is at or above the write role
 * (`foks-server-db` `realtime/policy.rs`). Basic chat serves only direct
 * same-host user membership, so team parties are not counted.
 */

import {
  admits,
  parseRole,
  partiesOf,
  partyName,
  plural,
  roleRank,
  visibilityOf,
} from '../model';
import type { AgentSnapshot, Party, RoleWire, StoreRef } from '../model';
import type { ChatChannel } from '../chat-contract';
import { channelRole } from './mentions';

const ADMIN: RoleWire = { role: 'Admin' };

/** One user of the team who can read the channel. */
export interface ChannelReader {
  party: Party;
  /** Whether the reader's role also admits the channel's write role. */
  post: boolean;
}

/** A channel's readers and the team users it leaves out. */
export interface ChannelAudience {
  /** The role a reader needs, with the admins-only tier's floor applied. */
  read: RoleWire;
  /** The role a writer needs; a writer must also be able to read. */
  write: RoleWire;
  /** Highest role first, then by name. */
  readers: ChannelReader[];
  /** The team's direct users who cannot read the channel, in the same order. */
  others: Party[];
}

/** The higher of two roles. */
function higher(left: RoleWire, right: RoleWire): RoleWire {
  return admits(left, right) ? left : right;
}

/** Highest role first; a higher visibility band first; then by name. */
function byRole(left: Party, right: Party): number {
  const a = parseRole(left.destination_role);
  const b = parseRole(right.destination_role);
  return (
    roleRank(b) - roleRank(a) ||
    (b && a ? visibilityOf(b) - visibilityOf(a) : 0) ||
    partyName(left).localeCompare(partyName(right))
  );
}

/**
 * The role a channel's reader needs, or `null` when the read role is not one
 * the chat contract writes. The admins-only tier raises it to Admin, as a Go
 * client can write a Member read role on such a channel.
 */
export function channelReadRole(
  channel: Pick<ChatChannel, 'read_role' | 'admin'>,
): RoleWire | null {
  const readRole = channelRole(channel.read_role);
  if (!readRole) return null;
  return channel.admin ? higher(readRole, ADMIN) : readRole;
}

/**
 * Whether a party is a direct user member on the team's own host, the only
 * membership the server admits to a channel (`membership_role` in
 * `realtime/policy.rs`). A party with a host scope is a member of another
 * host.
 */
function directUser(party: Party): boolean {
  return party.party_kind === 'user' && !party.scoped_host_id_hex;
}

/**
 * The roles a channel's reader and writer need, or `undefined` when a role is
 * not one the chat contract writes. The read role is
 * {@link channelReadRole}'s, and the write role is at least the read role.
 */
export function channelRoles(
  channel: ChatChannel,
): { read: RoleWire; write: RoleWire } | undefined {
  const read = channelReadRole(channel);
  const writeRole = channelRole(channel.write_role);
  if (!read || !writeRole) return undefined;
  return { read, write: higher(writeRole, read) };
}

/**
 * The channel's audience on the team roster, or `undefined` when it cannot be
 * stated: the roster has not arrived (a team always has at least the
 * reader's own membership, so no parties means not loaded), or a role is not
 * one the chat contract writes.
 */
export function channelAudience(
  snapshot: AgentSnapshot,
  storeId: StoreRef,
  channel: ChatChannel,
): ChannelAudience | undefined {
  const roles = channelRoles(channel);
  if (!roles) return undefined;
  const parties = partiesOf(snapshot, storeId);
  if (!parties.length) return undefined;
  const { read, write } = roles;
  const readers: ChannelReader[] = [];
  const others: Party[] = [];
  for (const party of parties.filter(directUser).sort(byRole)) {
    if (admits(party.destination_role, read))
      readers.push({ party, post: admits(party.destination_role, write) });
    else others.push(party);
  }
  return { read, write, readers, others };
}

/** Whether every user of the team can read the channel. */
export function everyoneReads(audience: ChannelAudience): boolean {
  return audience.others.length === 0;
}

/** Whether every user of the team can post in the channel. */
export function everyonePosts(audience: ChannelAudience): boolean {
  return everyoneReads(audience) && audience.readers.every((r) => r.post);
}

/** The visibility band a Member role names, or `null` for Admin and Owner. */
function memberBand(role: RoleWire): number | null {
  const parsed = parseRole(role);
  return parsed?.kind === 'member' ? visibilityOf(parsed) : null;
}

/**
 * Who a channel role admits, as the info panel's Who can read and Who can
 * post rows state it. `everyone` is whether it admits every user of the team
 * today; a Member role at band 0 or below reads "Everyone on the team" only
 * then. Without a roster the role alone is stated, which for such a role is
 * everyone.
 */
export function roleAudience(role: RoleWire, everyone = true): string {
  const band = memberBand(role);
  if (band === null)
    return parseRole(role)?.kind === 'owner' ? 'Owners' : 'Admins and owners';
  return everyone && band <= 0
    ? 'Everyone on the team'
    : `Members at visibility level ${band} and above`;
}

/**
 * The conversation header's line under the channel name. A channel that some
 * team users cannot read leads with its reader count, ahead of any
 * description; otherwise the description, or the member count when there is
 * none. Empty when there is nothing to state.
 */
export function audienceLine(
  audience: ChannelAudience | undefined,
  description: string | undefined,
): string {
  if (audience && !everyoneReads(audience))
    return [plural(audience.readers.length, 'reader'), description]
      .filter(Boolean)
      .join(' · ');
  if (description) return description;
  return audience ? plural(audience.readers.length, 'member') : '';
}

/**
 * The empty conversation's sentence on who will read the first message.
 * Without a roster it names the team only.
 */
export function audienceSentence(
  audience: ChannelAudience | undefined,
  teamName: string,
): string {
  if (!audience) return `Only the members of ${teamName} can read it.`;
  const count = audience.readers.length;
  const band = memberBand(audience.read);
  if (band === null) {
    const who =
      parseRole(audience.read)?.kind === 'owner'
        ? plural(count, 'owner')
        : `${count} ${count === 1 ? 'admin or owner' : 'admins and owners'}`;
    return `Only the ${who} of ${teamName} can read it.`;
  }
  const level = everyoneReads(audience)
    ? ''
    : ` at visibility level ${band} and above`;
  return `Only the ${plural(count, 'member')} of ${teamName}${level} can read it.`;
}
