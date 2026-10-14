import type { ChatMessage } from '../chat-contract';
import type { TrackedOperation } from './operations';
import {
  continuesMessageGroup,
  messageDay,
  messageDayKey,
  senderName,
} from './presentation';
import { operationOf, type OutgoingMessage } from './send/outgoing';

/**
 * One row of a conversation, in display order. The history rows come first:
 * a `day` separator above the first message of each local day, the `new`
 * divider above the first unread message, and the `message` rows. The rows
 * this device has not seen in history follow them: the `outgoing` messages
 * the send service holds, with a `day` separator where an outgoing message's
 * local day differs from the row above it, then the tracked `operation`s no
 * outgoing message stands for. {@link splitThreadRows} separates the two
 * sections.
 */
export type ThreadRow =
  | {
      kind: 'day';
      key: string;
      label: string;
      /** Whether the separator labels an unsent row rather than history. */
      unsent: boolean;
    }
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
      /**
       * Whether the history page that loaded the message reported earlier
       * messages it could not check.
       */
      incomplete: boolean;
    }
  | {
      kind: 'outgoing';
      key: string;
      message: OutgoingMessage;
      avatarName: string;
      grouped: boolean;
    }
  | {
      kind: 'operation';
      key: string;
      operation: TrackedOperation;
      /** The reader's name, which the avatar draws beside "You". */
      avatarName: string;
    };

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
  /**
   * Per message ID, whether the page that loaded it reported earlier
   * messages it could not check (`HistoryWindow.verification`).
   */
  verification?: ReadonlyMap<string, boolean>;
  /** The key of a history row; the thread passes the send service's key. */
  keyOf?: (message: ChatMessage) => string;
  /** The current time, which the day separators are labelled against. */
  now?: number;
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
    verification,
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
        unsent: false,
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
      incomplete: verification?.get(message.id) ?? false,
    });
  });
  // An unsent row draws its clock time, so it takes a day separator when
  // its local day differs from the row above it. With no history loaded,
  // including while the first page loads, the first unsent row carries its
  // own separator.
  let lastDay = messages.length
    ? messageDayKey(messages[messages.length - 1].insert_time)
    : '';
  const unsent = outgoing.filter((message) => !message.observed);
  const ownName = senderName(actor, names);
  unsent.forEach((message, index) => {
    const createdAt = String(message.createdAt);
    const day = messageDayKey(createdAt);
    const newDay = Boolean(day) && day !== lastDay;
    if (newDay)
      rows.push({
        kind: 'day',
        key: `day:outgoing:${message.id}`,
        label: messageDay(createdAt, now),
        unsent: true,
      });
    if (day) lastDay = day;
    rows.push({
      kind: 'outgoing',
      key: `outgoing:${message.id}`,
      message,
      avatarName: ownName,
      grouped: index > 0 && !newDay,
    });
  });
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
        avatarName: ownName,
      });
  return rows;
}

/**
 * The rows of {@link threadRows} split where the unsent section begins: the
 * history rows, and the unsent rows with their day separators. A detached
 * window draws its newer-history edge between the two.
 */
export function splitThreadRows(rows: readonly ThreadRow[]): {
  held: ThreadRow[];
  unsent: ThreadRow[];
} {
  const start = rows.findIndex(
    (row) =>
      row.kind === 'outgoing' ||
      row.kind === 'operation' ||
      (row.kind === 'day' && row.unsent),
  );
  return start < 0
    ? { held: [...rows], unsent: [] }
    : { held: rows.slice(0, start), unsent: rows.slice(start) };
}
