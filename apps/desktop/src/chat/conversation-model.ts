import type { ChatMessage, ChatResult } from '../chat-contract';
import type { HistoryEvent } from './conversation-events';
import { channelIntegrity } from './errors';

/**
 * The rows and UTF-8 text bytes one channel's window is trimmed to. They sit
 * well below the history cache's budget across channels, so one long
 * scrollback leaves room for the other channels' windows.
 */
export interface WindowLimits {
  rows: number;
  bytes: number;
}
export const HISTORY_WINDOW: WindowLimits = {
  rows: 400,
  bytes: 3 * 1024 * 1024,
};

export interface HistoryWindow {
  channel: string;
  messages: ChatMessage[];
  before: string | null;
  verification: ReadonlyMap<string, boolean>;
  /**
   * The newest held row is below the channel's published head: the window
   * was opened at the first unread message, or paged forward, and has not
   * reached the newest row. A detached window takes no incremental `after`
   * reads; it pages forward with explicit range reads instead.
   */
  detached: boolean;
}
/** How an accepted history page relates to the window already held. */
export interface HistoryPlacement {
  /** Discard the held window: a reset, a rollback, or an anchored open. */
  replace?: boolean;
  /** The page is a range read past the held tail. */
  newer?: boolean;
  /**
   * Whether the page leaves the window short of the published head. Omitted,
   * an older page keeps the held flag and any other page clears it.
   */
  detached?: boolean;
}
/**
 * Accepts a verified history page into the team's history cache. A page
 * merged from several replies carries the replies' results in `sources`.
 */
export type AcceptHistory = (
  page: Extract<ChatResult, { kind: 'history' }>,
  before: string | null,
  placement?: HistoryPlacement,
  sources?: readonly ChatResult[],
) => HistoryWindow | void;
/**
 * Merges a verified history page into the window held for its channel and
 * returns the new window; a window held for another channel is discarded.
 * Throws when the page conflicts with a held row. `held` is never modified.
 *
 * A window past `limits` is trimmed at the end away from the page: an older
 * page drops held rows from the newest end and detaches the window, and any
 * other page drops them from the oldest end and moves the older cursor to
 * the oldest row kept. The page's own rows are never dropped, so a window
 * holds at least the page it accepted last.
 */
export function mergeHistoryPage(
  held: HistoryWindow | null,
  result: HistoryEvent,
  limits: WindowLimits = HISTORY_WINDOW,
): HistoryWindow {
  const previous = held?.channel === result.channel ? held : null;
  // A newer page is a range read past the held tail. It extends the window at
  // the newest end and keeps the older cursor. One that does not continue the
  // held tail, because the agent narrowed the range or the server omitted
  // rows, replaces the window, as a non-contiguous tail page does.
  const newer = result.newer === true;
  const older = newer ? null : result.requestBefore;
  const last = previous?.messages.at(-1);
  const incoming = [...result.messages].sort((a, b) =>
    BigInt(a.sequence) < BigInt(b.sequence) ? -1 : 1,
  );
  const reset =
    !older &&
    last &&
    incoming[0] &&
    BigInt(incoming[0].sequence) > BigInt(last.sequence) + 1n;
  const rows = new Map(
    (reset ? [] : (previous?.messages ?? [])).map((m) => [m.id, m]),
  );
  for (const message of incoming) {
    const old = rows.get(message.id);
    if (old && !sameMessage(old, message) && !opened(old, message))
      throw channelIntegrity('The message changed while refreshing.');
    rows.set(message.id, message);
  }
  const messages = [...rows.values()].sort((a, b) =>
    BigInt(a.sequence) < BigInt(b.sequence) ? -1 : 1,
  );
  const sequences = new Set(messages.map((m) => m.sequence));
  if (sequences.size !== messages.length)
    throw channelIntegrity('Conflicting messages occupy the same sequence.');
  let bytes = messages.reduce((n, m) => n + textBytes(m), 0);
  const fresh = new Set(incoming.map((m) => m.id));
  let first = 0;
  let end = messages.length;
  while (end - first > limits.rows || bytes > limits.bytes) {
    const row = older ? messages[end - 1] : messages[first];
    if (fresh.has(row.id)) break;
    bytes -= textBytes(row);
    if (older) end--;
    else first++;
  }
  const kept = messages.slice(first, end);
  const verification = new Map(reset ? [] : previous?.verification);
  for (const message of incoming) verification.set(message.id, result.missing);
  if (kept.length !== messages.length) {
    const ids = new Set(kept.map((m) => m.id));
    for (const id of verification.keys())
      if (!ids.has(id)) verification.delete(id);
  }
  return {
    channel: result.channel,
    messages: kept,
    verification,
    // Rows trimmed from the oldest end are read again below the oldest row
    // kept; the agent's cursor is the lowest sequence a page holds.
    before:
      first > 0 && kept[0]
        ? kept[0].sequence
        : previous && (result.incremental || (newer && !reset))
          ? previous.before
          : reset || older || !previous?.before
            ? result.before
            : previous.before,
    // A tail or incremental page reaches the head; an older page leaves the
    // newest end where it was, unless the trim dropped rows from it.
    detached:
      end < messages.length ||
      (result.detached ?? (older ? (previous?.detached ?? false) : false)),
  };
}

function textBytes(message: ChatMessage): number {
  return message.content.kind === 'text'
    ? new TextEncoder().encode(message.content.text).length
    : 0;
}

/**
 * A row is unverifiable when, among other causes, the reader's team view
 * lacks its key generation, and a later read with that generation opens it.
 * The server's mapping and claims are unchanged; foks-client's anchor ledger
 * still detects a substituted row.
 */
function opened(a: ChatMessage, b: ChatMessage): boolean {
  return (
    a.content.kind === 'unverifiable' &&
    b.content.kind !== 'unverifiable' &&
    a.id === b.id &&
    a.sequence === b.sequence &&
    a.sender === b.sender &&
    a.send_time === b.send_time &&
    a.insert_time === b.insert_time
  );
}

function sameMessage(a: ChatMessage, b: ChatMessage): boolean {
  return (
    a.id === b.id &&
    a.sequence === b.sequence &&
    a.sender === b.sender &&
    a.send_time === b.send_time &&
    a.insert_time === b.insert_time &&
    a.content.kind === b.content.kind &&
    (a.content.kind !== 'text' ||
      (b.content.kind === 'text' && a.content.text === b.content.text))
  );
}
