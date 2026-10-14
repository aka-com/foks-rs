import type { ChatMessage, ChatResult } from '../chat-contract';
import type { HistoryEvent } from './conversation-events';
import { CHAT_HISTORY_BYTES, CHAT_HISTORY_ROWS } from '../chat-limits';
import { channelIntegrity } from './errors';

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
 * Throws when the page conflicts with a held row or the merged window
 * exceeds the history limits. `held` is never modified.
 */
export function mergeHistoryPage(
  held: HistoryWindow | null,
  result: HistoryEvent,
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
  const bytes = messages.reduce(
    (n, m) =>
      n +
      (m.content.kind === 'text'
        ? new TextEncoder().encode(m.content.text).length
        : 0),
    0,
  );
  if (messages.length > CHAT_HISTORY_ROWS || bytes > CHAT_HISTORY_BYTES)
    throw {
      code: 'chat-limit',
      message:
        'This conversation view reached its history limit. Reopen it to load the newest messages.',
      fatal: false,
      retryable: false,
      ambiguous: false,
    };
  const verification = new Map(reset ? [] : previous?.verification);
  for (const message of incoming) verification.set(message.id, result.missing);
  return {
    channel: result.channel,
    messages,
    verification,
    before:
      previous && (result.incremental || (newer && !reset))
        ? previous.before
        : reset || older || !previous?.before
          ? result.before
          : previous.before,
    // A tail or incremental page reaches the head; an older page leaves the
    // newest end where it was.
    detached:
      result.detached ?? (older ? (previous?.detached ?? false) : false),
  };
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
