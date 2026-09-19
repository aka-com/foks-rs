import { partiesOf, shortId } from '../model';
import type { AgentSnapshot, StoreRef } from '../model';
import type { ChatChannel, ChatConversation } from '../chat-contract';
import {
  CHAT_DESCRIPTION_MAX_CHARS,
  CHAT_DESCRIPTION_MIN_CHARS,
  CHAT_NAME_MAX_CHARS,
  CHAT_NAME_MIN_CHARS,
} from '../chat-limits';
/** The channel's own name, with no "#" — the crumbs and lists that sit in a
 * labelled context draw this. */
export function channelLabel(channel: ChatChannel): string {
  return channel.name || 'general';
}

export function channelTitle(channel: ChatChannel): string {
  return `#${channelLabel(channel)}`;
}

/**
 * Lowercased the way the agent lowercases: per Unicode scalar, keeping the
 * first scalar of the mapping (`c.to_lowercase().next()` in
 * `foks-client/src/realtime/operations/prepare.rs`). JavaScript's
 * `toLowerCase()` applies the full mapping, which can lengthen a string —
 * "İ" becomes two scalars — and would then count characters the agent never
 * stores.
 */
export function lowercaseChatText(value: string): string {
  return [...value]
    .map((scalar) => [...scalar.toLowerCase()][0] ?? scalar)
    .join('');
}

/** The protocol name: trimmed, lowercased, and empty for the general channel. */
export function normalizeChannelName(raw: string): string {
  const name = lowercaseChatText(raw.trim());
  return name === 'general' ? '' : name;
}

/**
 * Returns the validation error for a channel name, or `null` when valid. The
 * agent remains authoritative and also enforces a character table that is not
 * duplicated here. This check covers only errors that can be reported before
 * submission: the length range, the general-channel alias, consecutive hyphens,
 * spaces, and a name the team already has. `CHAT_NAME_*_CHARS` defines the
 * range accepted by `ChatLimits`, counted in Unicode scalars to match the
 * agent.
 *
 * `existing` is the team's channel names as the agent stores them, so the
 * general channel is the empty string.
 */
export function channelNameProblem(
  raw: string,
  existing: readonly string[] = [],
): string | null {
  const name = normalizeChannelName(raw);
  const taken = new Set(existing.map(normalizeChannelName));
  if (!name)
    return taken.has('') ? 'This team already has a general channel.' : null;
  if ([...name].length < CHAT_NAME_MIN_CHARS)
    return `Channel names are at least ${CHAT_NAME_MIN_CHARS} characters.`;
  if ([...name].length > CHAT_NAME_MAX_CHARS)
    return `Channel names are at most ${CHAT_NAME_MAX_CHARS} characters.`;
  if (/\s/.test(name)) return 'Channel names cannot contain spaces.';
  if (name.includes('--'))
    return 'Channel names cannot contain two hyphens in a row.';
  if (taken.has(name)) return `#${name} already exists in this team.`;
  return null;
}

/** The description the agent stores for what was typed: lowercased. */
export function normalizeChannelDescription(raw: string): string {
  return lowercaseChatText(raw);
}

/**
 * Returns the validation error for a channel description, or `null` when
 * valid. A non-empty description must fall within the
 * `CHAT_DESCRIPTION_*_CHARS` range enforced by `ChatLimits`.
 */
export function channelDescriptionProblem(raw: string): string | null {
  const length = [...normalizeChannelDescription(raw)].length;
  if (!length) return null;
  if (length < CHAT_DESCRIPTION_MIN_CHARS)
    return `Descriptions must be at least ${CHAT_DESCRIPTION_MIN_CHARS} characters or empty.`;
  if (length > CHAT_DESCRIPTION_MAX_CHARS)
    return `Descriptions are at most ${CHAT_DESCRIPTION_MAX_CHARS} characters.`;
  return null;
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
 * One team's channels: every conversation that is listed, then the channels
 * that have no conversation yet, then the hidden conversations. The column
 * counts this list and the conversation opens from it, so both see the same
 * channels. Hidden conversations remain in the list, dimmed and marked
 * Hidden, so a team with only a hidden conversation does not appear to have no
 * channels. They are placed last, leaving the first visible channel first.
 */
export function listChannels(
  channels: readonly ChatChannel[],
  conversations: readonly ChatConversation[],
): ListedChannel[] {
  const current = new Map(channels.map((channel) => [channel.id, channel]));
  const known = new Set(
    conversations.map((conversation) => conversation.channel.id),
  );
  const listed = (conversation: ChatConversation) => ({
    channel: current.get(conversation.channel.id) ?? conversation.channel,
    conversation,
  });
  return [
    ...conversations.filter((conversation) => !conversation.hidden).map(listed),
    ...channels
      .filter((channel) => !known.has(channel.id))
      .map((channel) => ({ channel, conversation: undefined })),
    ...conversations.filter((conversation) => conversation.hidden).map(listed),
  ];
}

const NOTHING_BLOCKED: ReadonlySet<string> = new Set();

/**
 * What a channel says about itself beside its name: the column's rows, a
 * single-channel team's row and the group tab's channel list all draw this, so
 * none of them can disagree about a channel being stopped, restricted, hidden
 * or muted. `blocked` is the team's quarantined channels, which only the inbox
 * service knows.
 */
export function channelMeta(
  listed: ListedChannel,
  blocked: ReadonlySet<string> = NOTHING_BLOCKED,
): string {
  if (blocked.has(listed.channel.id)) return 'Channel stopped';
  return [
    !listed.channel.readable ? 'Restricted' : '',
    listed.conversation?.hidden ? 'Hidden' : '',
    listed.conversation?.muted ? 'Muted' : '',
  ]
    .filter(Boolean)
    .join(' · ');
}

/**
 * The channel a location opens: the one it names, else the first listed
 * channel when it names none. The column and the conversation resolve it the
 * same way, so the row drawn as current is the one the pane mounted.
 */
export function openChannel(
  listed: readonly ListedChannel[],
  wanted: string | undefined,
): ChatChannel | undefined {
  return (
    listed.find(({ channel }) => channel.id === wanted)?.channel ??
    (wanted ? undefined : listed[0]?.channel)
  );
}

/**
 * Removes the internal visibility band from a channel role label. The agent
 * represents a member role as, for example, "Member (0)". The shell's
 * `roleName` takes a parsed `Role`; this function accepts the role text from
 * the chat contract.
 */
export function roleTextWithoutBand(role: string): string {
  return role.replace(/\s*\(-?\d+\)$/, '');
}

export function accessSummary(channel: ChatChannel): string {
  if (!channel.readable) return 'Read access required';
  if (channel.read_role === channel.write_role)
    return `${roleTextWithoutBand(channel.read_role)} can read and write`;
  const read = roleTextWithoutBand(channel.read_role);
  const write = roleTextWithoutBand(channel.write_role);
  // Two member bands differ in the band alone, so the band is what is named.
  return read === write
    ? `Read ${channel.read_role} · Write ${channel.write_role}`
    : `Read ${read} · Write ${write}`;
}

/**
 * Formats an inbox preview as the sender display name followed by the
 * truncated message text. An empty string means there is no preview.
 */
export function previewLine(
  conversation: ChatConversation | undefined,
  actor: string | null = null,
  names?: ReadonlyMap<string, string>,
): string {
  const preview = conversation?.preview;
  if (!preview) return '';
  const sender =
    actor !== null && preview.sender === actor
      ? 'You'
      : preview.sender
        ? (names?.get(preview.sender) ?? shortId(preview.sender))
        : 'Team member';
  const text =
    preview.content.kind === 'text'
      ? preview.content.text
      : 'Unsupported message';
  return `${sender}: ${text}`;
}

/**
 * The time an inbox row carries beside a preview: the clock time today, the
 * weekday within the past week, the date before that. An unreadable stamp
 * carries nothing rather than a number no reader can place.
 */
export function previewTime(
  milliseconds: string,
  nowMilliseconds: number = Date.now(),
): string {
  const date = messageDate(milliseconds);
  if (!date) return '';
  const now = new Date(nowMilliseconds);
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  if (sameDay)
    return new Intl.DateTimeFormat(undefined, { timeStyle: 'short' }).format(
      date,
    );
  const days = (nowMilliseconds - date.valueOf()) / 86_400_000;
  if (days > 0 && days < 6)
    return new Intl.DateTimeFormat(undefined, { weekday: 'short' }).format(
      date,
    );
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
  }).format(date);
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

export function relativeMessageTime(
  milliseconds: string,
  nowMilliseconds: number = Date.now(),
): string {
  const date = messageDate(milliseconds);
  if (!date) return `Time ${milliseconds}`;
  const difference = date.valueOf() - nowMilliseconds;
  const absolute = Math.abs(difference);
  if (absolute < 60_000) return 'just now';
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ['year', 31_536_000_000],
    ['month', 2_592_000_000],
    ['week', 604_800_000],
    ['day', 86_400_000],
    ['hour', 3_600_000],
    ['minute', 60_000],
  ];
  const [unit, size] = units.find(([, size]) => absolute >= size) ?? units[5];
  return new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' }).format(
    Math.round(difference / size),
    unit,
  );
}
