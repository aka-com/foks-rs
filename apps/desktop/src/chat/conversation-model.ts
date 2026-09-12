import type { ChatAction, ChatMessage, ChatResult } from '../chat-contract';
import { CHAT_HISTORY_BYTES, CHAT_HISTORY_ROWS } from '../chat-limits';
import { channelIntegrity } from './client';
import { reconcileOperations } from './operations';
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
  action: ChatAction,
  result: ChatResult,
): ConversationState {
  if (result.kind !== 'history') {
    let operations = reconcileOperations(state.operations, action, result);
    if (state.history)
      operations = reconcileOperations(
        operations,
        { action: 'history', channel: state.history.channel, before: null },
        {
          kind: 'history',
          channel: state.history.channel,
          before: state.history.before,
          messages: state.history.messages,
          missing_predecessors: [],
        },
      );
    return { ...state, operations };
  }
  const previous =
    state.history?.channel === result.channel ? state.history : null;
  const older = action.action === 'history' ? action.before : null;
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
    if (old && JSON.stringify(old) !== JSON.stringify(message))
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
  for (const message of incoming)
    verification.set(message.id, result.missing_predecessors.length > 0);
  return {
    operations: reconcileOperations(state.operations, action, result),
    history: {
      channel: result.channel,
      messages,
      verification,
      before:
        reset || older || !previous?.before ? result.before : previous.before,
    },
  };
}
