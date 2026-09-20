import type { ChatMessage } from '../chat-contract';
import type { ConversationEvent } from './conversation-events';
import { CHAT_HISTORY_BYTES, CHAT_HISTORY_ROWS } from '../chat-limits';
import { channelIntegrity } from './errors';
import { reconcileOperations, observeMessages } from './operations';
import type { TrackedOperation } from './operations';

export interface HistoryWindow {
  channel: string;
  messages: ChatMessage[];
  before: string | null;
  verification: ReadonlyMap<string, boolean>;
}
export interface ConversationState {
  operations: TrackedOperation[];
  history: HistoryWindow | null;
}
export const emptyConversation = (): ConversationState => ({
  operations: [],
  history: null,
});

/** History acceptance and delivery observation commit together, or neither commits. */
export function conversationResult(
  state: ConversationState,
  result: ConversationEvent,
): ConversationState {
  if (result.kind === 'reset') return emptyConversation();
  if (result.kind === 'access')
    return {
      operations: state.operations.map((op) =>
        result.readable.has(op.channel) ? op : { ...op, text: undefined },
      ),
      history:
        state.history && result.readable.has(state.history.channel)
          ? state.history
          : null,
    };
  if (result.kind === 'status-unresolved')
    return {
      ...state,
      operations: state.operations.map((op) =>
        op.id === result.operation ? { ...op, statusUnknown: true } : op,
      ),
    };
  if (result.kind !== 'history') {
    let operations = reconcileOperations(state.operations, result);
    if (state.history)
      operations = observeMessages(
        operations,
        new Set(state.history.messages.map((m) => m.id)),
      );
    return { ...state, operations };
  }
  const previous =
    state.history?.channel === result.channel ? state.history : null;
  const older = result.requestBefore;
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
    if (old && !sameMessage(old, message))
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
    operations: reconcileOperations(state.operations, result),
    history: {
      channel: result.channel,
      messages,
      verification,
      before:
        result.incremental && previous
          ? previous.before
          : reset || older || !previous?.before
            ? result.before
            : previous.before,
    },
  };
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
