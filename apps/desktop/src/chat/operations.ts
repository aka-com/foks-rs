import type { ChatAction, ChatOperation, ChatResult } from '../chat-contract';
import { CHAT_HISTORY_BYTES, CHAT_PENDING_ROWS } from '../chat-limits';

export interface TrackedOperation extends ChatOperation {
  text?: string;
  observed?: boolean;
  statusUnknown?: boolean;
}
const terminal = (op: ChatOperation) =>
  !['prepared', 'uncertain'].includes(op.state);
/** Ledger status and accepted-history observation are independent facts. */
export function reconcileOperations(
  old: TrackedOperation[],
  action: ChatAction,
  result: ChatResult,
): TrackedOperation[] {
  if (result.kind === 'operation') {
    const previous = old.find((op) => op.id === result.operation.id);
    if (action.action === 'finalize' || result.operation.state === 'cancelled')
      return old.filter((op) => op.id !== result.operation.id);
    const next: TrackedOperation = {
      ...previous,
      ...result.operation,
      statusUnknown: false,
    };
    if (action.action === 'prepare-message' && !next.observed)
      next.text = action.text;
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
    const ids = new Set(result.messages.map((m) => m.id));
    return old.map((op) =>
      ids.has(op.id) ? { ...op, observed: true, text: undefined } : op,
    );
  }
  if (result.kind === 'channels')
    return old.filter(
      (op) =>
        !(
          op.create &&
          op.state === 'confirmed' &&
          result.channels.some((c) => c.id === op.channel)
        ),
    );
  return old;
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
