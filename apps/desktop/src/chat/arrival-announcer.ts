import { useEffect, useState } from 'react';
import type { AgentSnapshot, StoreRef } from '../model';
import { systemChatClock, type ChatClock } from './clock';
import type { TeamInbox } from './inbox-service';
import { channelTitle, partyNames, senderName } from './presentation';
import { lastPosition } from './snapshots';
import { focusedWindow } from './visibility';

/** The shortest time between two announcements, in milliseconds. */
export const ARRIVAL_ANNOUNCE_MS = 5_000;
/**
 * How long an announcement stays in the live region. Clearing it before the
 * next one lets the same words be announced again.
 */
export const ARRIVAL_CLEAR_MS = 3_000;
/** Channels named in one announcement before the rest are counted. */
const NAMED_CHANNELS = 3;

/** Messages that reached one channel since the inbox last stated it. */
export interface ChannelArrival {
  team: StoreRef;
  channel: string;
  /** The channel's `#name`. */
  title: string;
  count: number;
  /**
   * The newest message's sender, when the inbox preview is a message whose
   * sender is authenticated (Basic text, displayed or oversized).
   */
  sender?: string;
}

/** The newest position each listed channel of each team stood at. */
export type InboxPositions = ReadonlyMap<StoreRef, ReadonlyMap<string, bigint>>;

/**
 * Compares the inbox with the positions it last stated and returns the
 * channels that gained messages from others. A team or channel seen for the
 * first time only sets its position. A team whose inbox carries no data keeps
 * its earlier positions. Muted, hidden, unreadable and stopped channels are
 * not reported. A channel whose newest message is the reader's own is not
 * reported either; with no preview, a channel is reported only while it
 * counts unread messages. Only metadata is read: the preview's text is not.
 */
export function channelArrivals(
  previous: InboxPositions,
  inbox: ReadonlyMap<StoreRef, TeamInbox>,
  names: (team: StoreRef) => ReadonlyMap<string, string>,
): { positions: InboxPositions; arrivals: ChannelArrival[] } {
  const positions = new Map<StoreRef, ReadonlyMap<string, bigint>>();
  const arrivals: ChannelArrival[] = [];
  for (const [team, entry] of inbox) {
    const before = previous.get(team);
    if (!entry.data || !entry.scope) {
      if (before) positions.set(team, before);
      continue;
    }
    const actor = entry.scope.actor;
    const current = new Map<string, bigint>();
    positions.set(team, current);
    for (const conversation of entry.data.conversations) {
      const { channel, preview } = conversation;
      const position = lastPosition(conversation);
      current.set(channel.id, position);
      const was = before?.get(channel.id);
      if (
        was === undefined ||
        position <= was ||
        conversation.hidden ||
        conversation.muted ||
        !channel.readable ||
        entry.blockedChannels.has(channel.id) ||
        (preview ? preview.sender === actor : BigInt(conversation.unread) <= 0n)
      )
        continue;
      const authenticated =
        preview?.sender &&
        (preview.content.kind === 'text' ||
          preview.content.kind === 'oversized');
      arrivals.push({
        team,
        channel: channel.id,
        title: channelTitle(channel),
        count: Number(position - was),
        ...(authenticated && {
          sender: senderName(preview.sender, names(team)),
        }),
      });
    }
  }
  return { positions, arrivals };
}

/**
 * What the live region says for `arrivals`: the sender and the channel, never
 * the message text, which follows the notification previews setting
 * elsewhere. `teamName` returns a team's name where the channel's own name
 * does not identify it.
 */
export function arrivalText(
  arrivals: readonly ChannelArrival[],
  teamName: (team: StoreRef) => string | undefined,
): string {
  const where = (arrival: ChannelArrival) => {
    const team = teamName(arrival.team);
    return team ? `${arrival.title} in ${team}` : arrival.title;
  };
  if (arrivals.length === 1) {
    const [{ count, sender }] = arrivals;
    if (count === 1)
      return sender
        ? `New message from ${sender} in ${where(arrivals[0])}`
        : `New message in ${where(arrivals[0])}`;
    return `${count} new messages in ${where(arrivals[0])}${sender ? `, latest from ${sender}` : ''}`;
  }
  const named = arrivals.slice(0, NAMED_CHANNELS).map(where);
  const more = arrivals.length - named.length;
  const list =
    more > 0
      ? `${named.join(', ')} and ${more} more ${more === 1 ? 'channel' : 'channels'}`
      : `${named.slice(0, -1).join(', ')} and ${named.at(-1)}`;
  return `New messages in ${list}`;
}

/**
 * Announces messages from others as they reach the inbox while the window
 * has focus, at most once per {@link ARRIVAL_ANNOUNCE_MS}. Arrivals within
 * that interval are merged into the next announcement. Arrivals while the
 * window is not focused are left to desktop alerts and are not announced
 * later.
 */
export class ArrivalAnnouncer {
  private positions: InboxPositions = new Map();
  private pending = new Map<string, ChannelArrival>();
  private teamName: (team: StoreRef) => string | undefined = () => undefined;
  private last = -Infinity;
  private flushTimer: unknown = null;
  private clearTimer: unknown = null;
  constructor(
    private clock: ChatClock,
    private say: (text: string) => void,
    private focused: () => boolean = focusedWindow,
  ) {}
  observe(
    inbox: ReadonlyMap<StoreRef, TeamInbox>,
    names: (team: StoreRef) => ReadonlyMap<string, string>,
    teamName: (team: StoreRef) => string | undefined,
  ): void {
    const { positions, arrivals } = channelArrivals(
      this.positions,
      inbox,
      names,
    );
    this.positions = positions;
    if (!arrivals.length || !this.focused()) return;
    for (const arrival of arrivals) {
      // Keyed by both identifiers without joining them into one string.
      const key = JSON.stringify([arrival.team, arrival.channel]);
      const held = this.pending.get(key);
      // The pending entry moves to the end, so channels are named in the
      // order they last gained a message.
      this.pending.delete(key);
      this.pending.set(
        key,
        held ? { ...arrival, count: held.count + arrival.count } : arrival,
      );
    }
    this.teamName = teamName;
    if (this.flushTimer !== null) return;
    const wait = this.last + ARRIVAL_ANNOUNCE_MS - this.clock.now();
    if (wait <= 0) this.flush();
    else this.flushTimer = this.clock.later(this.flush, wait);
  }
  private flush = () => {
    this.flushTimer = null;
    const arrivals = [...this.pending.values()];
    this.pending.clear();
    if (!arrivals.length || !this.focused()) return;
    this.last = this.clock.now();
    this.say(arrivalText(arrivals, this.teamName));
    if (this.clearTimer !== null) this.clock.cancel(this.clearTimer);
    this.clearTimer = this.clock.later(() => {
      this.clearTimer = null;
      this.say('');
    }, ARRIVAL_CLEAR_MS);
  };
  /** Cancels the timers; a later `observe` starts them again. */
  stop(): void {
    if (this.flushTimer !== null) this.clock.cancel(this.flushTimer);
    if (this.clearTimer !== null) this.clock.cancel(this.clearTimer);
    this.flushTimer = this.clearTimer = null;
    this.pending.clear();
  }
}

/**
 * The text of the Chat tab's arrival announcements, for a visually hidden
 * `role="status"` element. Channels are named with their team when the
 * inbox lists more than one team.
 */
export function useArrivalAnnouncement(
  snapshot: AgentSnapshot,
  inbox: ReadonlyMap<StoreRef, TeamInbox>,
  clock: ChatClock = systemChatClock,
): string {
  const [text, setText] = useState('');
  const [announcer] = useState(() => new ArrivalAnnouncer(clock, setText));
  useEffect(() => () => announcer.stop(), [announcer]);
  useEffect(() => {
    const several = inbox.size > 1;
    announcer.observe(
      inbox,
      (team) => partyNames(snapshot, team),
      (team) =>
        several
          ? snapshot.stores.find((store) => store.id === team)?.name
          : undefined,
    );
  }, [announcer, inbox, snapshot]);
  return text;
}
