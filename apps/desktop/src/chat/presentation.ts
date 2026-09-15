import { partiesOf, shortId } from '../model';
import type { AgentSnapshot, StoreRef } from '../model';
import type { ChatChannel, ChatConversation } from '../chat-contract';
export function channelTitle(channel: ChatChannel): string {
  return `#${channel.name || 'general'}`;
}

/** The team's people by party identifier, so a sender reads as a name. */
export function partyNames(
  snapshot: AgentSnapshot,
  store: StoreRef,
): Map<string, string> {
  return new Map(
    partiesOf(snapshot, store)
      .filter((party) => party.party_kind === 'user')
      .map((party) => [
        party.party_id_hex,
        party.username ?? party.label ?? shortId(party.party_id_hex),
      ]),
  );
}

/** A channel of the open team, as the column and the conversation list it. */
export interface ListedChannel {
  channel: ChatChannel;
  conversation?: ChatConversation;
}

/**
 * One team's channels: every conversation that is not hidden, then the
 * channels that have no conversation yet. The column counts this list and the
 * conversation opens from it, so both see the same channels.
 */
export function listChannels(
  channels: readonly ChatChannel[],
  conversations: readonly ChatConversation[],
): ListedChannel[] {
  const current = new Map(channels.map((channel) => [channel.id, channel]));
  const known = new Set(
    conversations.map((conversation) => conversation.channel.id),
  );
  return [
    ...conversations
      .filter((conversation) => !conversation.hidden)
      .map((conversation) => ({
        channel: current.get(conversation.channel.id) ?? conversation.channel,
        conversation,
      })),
    ...channels
      .filter((channel) => !known.has(channel.id))
      .map((channel) => ({ channel, conversation: undefined })),
  ];
}

/**
 * A channel role as a reader says it: "Owner", "Admin", "Member". The agent
 * writes a member's visibility band as "Member (0)", which is a FOKS internal.
 */
export function roleName(role: string): string {
  return role.replace(/\s*\(-?\d+\)$/, '');
}

export function accessSummary(channel: ChatChannel): string {
  if (!channel.readable) return 'Read access required';
  if (channel.read_role === channel.write_role)
    return `${roleName(channel.read_role)} can read and write`;
  const read = roleName(channel.read_role);
  const write = roleName(channel.write_role);
  // Two member bands differ in the band alone, so the band is what is named.
  return read === write
    ? `Read ${channel.read_role} · Write ${channel.write_role}`
    : `Read ${read} · Write ${write}`;
}

export function messageDate(milliseconds: string): Date | null {
  const value = BigInt(milliseconds);
  if (value > 8_640_000_000_000_000n) return null;
  const date = new Date(Number(value));
  return Number.isNaN(date.valueOf()) ? null : date;
}

export function messageTime(milliseconds: string): string {
  const date = messageDate(milliseconds);
  if (!date) return `Time ${milliseconds}`;
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(date);
}
