import type { ChatAction, ChatOperation, ChatResult } from '../chat-contract';

/** One authoritative operation collection, updated in request completion order. */
export function reconcileOperations(
  old: ChatOperation[],
  action: ChatAction,
  result: ChatResult,
): ChatOperation[] {
  if (result.kind === 'operation')
    return [
      ...old.filter((op) => op.id !== result.operation.id),
      ...(action.action === 'finalize' ? [] : [result.operation]),
    ];
  if (result.kind === 'pending')
    return [
      ...old.filter(
        (op) =>
          !['prepared', 'uncertain'].includes(op.state) &&
          !result.operations.some((next) => next.id === op.id),
      ),
      ...result.operations,
    ];
  if (result.kind === 'history')
    return old.filter(
      (op) => !result.messages.some((message) => message.id === op.id),
    );
  if (result.kind === 'channels')
    return old.filter(
      (op) =>
        !(
          op.create &&
          op.state === 'confirmed' &&
          result.channels.some((channel) => channel.id === op.channel)
        ),
    );
  return old;
}
