import type { ChatMessage } from '../chat-contract';
import { shortId } from '../model';
import type { TrackedOperation } from './operations';
import {
  continuesMessageGroup,
  messageDay,
  messageDayKey,
} from './presentation';
import { operationOf, type OutgoingMessage } from './send/outgoing';

/**
 * One row of a conversation, in display order. The history rows come first:
 * a `day` separator above the first message of each local day, the `new`
 * divider above the first unread message, and the `message` rows. The rows
 * this device has not seen in history follow them: the `outgoing` messages
 * the send service holds, then the tracked `operation`s no outgoing message
 * stands for.
 */
export type ThreadRow =
  | { kind: 'day'; key: string; label: string }
  | { kind: 'new'; key: string }
  | {
      kind: 'message';
      key: string;
      message: ChatMessage;
      own: boolean;
      /** "You" for the reader's own messages, else the sender's name. */
      author: string;
      /** The sender's name, which the avatar draws even for "You". */
      avatarName: string;
      /** Whether the row continues the previous message's run. */
      grouped: boolean;
    }
  | {
      kind: 'outgoing';
      key: string;
      message: OutgoingMessage;
      avatarName: string;
      grouped: boolean;
    }
  | { kind: 'operation'; key: string; operation: TrackedOperation };

export interface ThreadRowOptions {
  /** The reader's party identifier, or `null` while the scope is unknown. */
  actor: string | null;
  /** The read point the NEW divider follows, or `null` for no divider. */
  newFrom: string | null;
  /**
   * Whether the first new message is older than the loaded window, which
   * then begins after it: {@link newMessagesUnloaded}.
   */
  newUnloaded?: boolean;
  /** Display names by party identifier. */
  names: ReadonlyMap<string, string>;
  /** The key of a history row; the thread passes the send service's key. */
  keyOf?: (message: ChatMessage) => string;
  /** The current time, which the day separators are labelled against. */
  now?: number;
}

/** The name a sender reads as: the roster's name, else a shortened id. */
export function senderName(
  sender: string | null,
  names: ReadonlyMap<string, string>,
): string {
  return sender ? (names.get(sender) ?? shortId(sender)) : 'Team member';
}

/**
 * Whether the window holds older rows it has not loaded and its first row is
 * past the first message after `newFrom`, so the first new message is not
 * among the loaded rows.
 */
export function newMessagesUnloaded(
  messages: readonly ChatMessage[],
  newFrom: string | null,
  hasOlder: boolean,
): boolean {
  return (
    newFrom !== null &&
    hasOlder &&
    messages.length > 0 &&
    BigInt(messages[0].sequence) > BigInt(newFrom) + 1n
  );
}

/**
 * The rows a conversation draws for its loaded history, its outgoing
 * messages and its tracked operations. `messages` is the history window,
 * sorted by sequence.
 */
export function threadRows(
  messages: readonly ChatMessage[],
  outgoing: readonly OutgoingMessage[],
  operations: readonly TrackedOperation[],
  {
    actor,
    newFrom,
    newUnloaded = false,
    names,
    keyOf = (message) => message.id,
    now = Date.now(),
  }: ThreadRowOptions,
): ThreadRow[] {
  const rows: ThreadRow[] = [];
  messages.forEach((message, index) => {
    const previous = index > 0 ? messages[index - 1] : undefined;
    const key = keyOf(message);
    const own = actor !== null && message.sender === actor;
    // The divider marks where the new messages begin, so it is not drawn
    // over a window that starts after that, nor above the reader's own
    // message.
    const isNew =
      newFrom !== null &&
      !own &&
      BigInt(message.sequence) > BigInt(newFrom) &&
      (previous ? BigInt(previous.sequence) <= BigInt(newFrom) : !newUnloaded);
    // The day is carried once, above the first message of each local day,
    // so no row repeats it.
    const day = messageDayKey(message.insert_time);
    const newDay =
      Boolean(day) &&
      (!previous || messageDayKey(previous.insert_time) !== day);
    if (newDay)
      rows.push({
        kind: 'day',
        key: `day:${key}`,
        label: messageDay(message.insert_time, now),
      });
    if (isNew) rows.push({ kind: 'new', key: 'new' });
    const avatarName = senderName(message.sender, names);
    rows.push({
      kind: 'message',
      key,
      message,
      own,
      author: own ? 'You' : avatarName,
      avatarName,
      grouped: !isNew && !newDay && continuesMessageGroup(previous, message),
    });
  });
  const unsent = outgoing.filter((message) => !message.observed);
  const ownName = senderName(actor, names);
  unsent.forEach((message, index) =>
    rows.push({
      kind: 'outgoing',
      key: `outgoing:${message.id}`,
      message,
      avatarName: ownName,
      grouped: index > 0,
    }),
  );
  // An operation an outgoing message carries is drawn by that message's row.
  const carried = new Set(
    outgoing.flatMap((message) => {
      const operation = operationOf(message);
      return operation ? [operation.id] : [];
    }),
  );
  for (const operation of operations)
    if (!carried.has(operation.id))
      rows.push({
        kind: 'operation',
        key: `operation:${operation.id}`,
        operation,
      });
  return rows;
}
