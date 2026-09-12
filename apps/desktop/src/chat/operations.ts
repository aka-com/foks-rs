import type { ChatOperation } from '../chat-contract';
import type { ConversationEvent } from './conversation-events';
import { CHAT_HISTORY_BYTES, CHAT_PENDING_ROWS } from '../chat-limits';

export interface TrackedOperation extends ChatOperation {
  text?: string;
  observed?: boolean;
  statusUnknown?: boolean;
  bodyUnavailable?: boolean;
}
const terminal = (op: ChatOperation) =>
  !['prepared', 'uncertain'].includes(op.state);
/** Ledger status and accepted-history observation are independent facts. */
export function reconcileOperations(
  old: TrackedOperation[],
  result: ConversationEvent,
): TrackedOperation[] {
  if (result.kind === 'body-recovered')
    return bound(
      old.map((op) =>
        op.id === result.operation &&
        op.channel === result.channel &&
        op.kind === 'send-message' &&
        !op.observed
          ? {
              ...op,
              text: result.text ?? op.text,
              bodyUnavailable: result.text === null,
            }
          : op,
      ),
    );
  if (result.kind === 'operation') {
    const previous = old.find((op) => op.id === result.operation.id);
    if (result.discard)
      return old.filter((op) => op.id !== result.operation.id);
    const next: TrackedOperation = {
      ...previous,
      ...result.operation,
      statusUnknown: false,
    };
    if (result.preparedText !== undefined && !next.observed)
      next.text = result.preparedText;
    return bound([...old.filter((op) => op.id !== next.id), next]);
  }
  if (result.kind === 'pending') {
    const incoming = new Map(result.operations.map((op) => [op.id, op]));
    const rows = old.map((op) => {
      const next = incoming.get(op.id);
      incoming.delete(op.id);
      return next
        ? { ...op, ...next, statusUnknown: false }
        : terminal(op) || op.observed
          ? op
          : { ...op, statusUnknown: true };
    });
    return bound([...rows, ...incoming.values()]);
  }
  // This event is dispatched only after the entire page is accepted into history.
  if (result.kind === 'history') {
    return observeMessages(old, new Set(result.messages.map((m) => m.id)));
  }
  if (result.kind === 'channels')
    return old.filter(
      (op) =>
        !(
          op.kind === 'create-channel' &&
          op.state === 'confirmed' &&
          result.channels.some((c) => c.id === op.channel)
        ),
    );
  return old;
}
export function observeMessages(
  old: TrackedOperation[],
  ids: ReadonlySet<string>,
): TrackedOperation[] {
  return old.map((op) =>
    op.kind === 'send-message' && ids.has(op.id)
      ? { ...op, observed: true, text: undefined }
      : op,
  );
}
function bound(rows: TrackedOperation[]): TrackedOperation[] {
  // Observed terminal operations no longer occur in pending queries. Foreground
  // requests are serialized; no older operation response can overtake this one.
  if (rows.length > CHAT_PENDING_ROWS)
    rows = rows.filter((op) => !(terminal(op) && op.observed));
  if (rows.length > CHAT_PENDING_ROWS)
    throw {
      code: 'chat-limit',
      message:
        'This view reached its saved-operation limit. Resolve saved operations or reopen Chat for durable recovery.',
      fatal: false,
      retryable: false,
      ambiguous: false,
    };
  let bytes = 0;
  return rows.map((op) => {
    bytes += op.text ? new TextEncoder().encode(op.text).length : 0;
    return bytes > CHAT_HISTORY_BYTES ? { ...op, text: undefined } : op;
  });
}
