import type {
  ChatAction,
  ChatChannel,
  ChatMessage,
  ChatOperation,
  ChatResult,
} from '../chat-contract';

/** Local model inputs, translated once from already validated transport replies. */
export type ConversationEvent =
  | {
      kind: 'body-recovered';
      operation: string;
      channel: string;
      text: string | null;
    }
  | {
      kind: 'history';
      channel: string;
      messages: ChatMessage[];
      before: string | null;
      requestBefore: string | null;
      incremental?: true;
      /** A range read past the held tail, set by the caller that asked for it. */
      newer?: true;
      /** Whether the page leaves the window short of the published head. */
      detached?: boolean;
      missing: boolean;
    }
  | {
      kind: 'operation';
      operation: ChatOperation;
      discard: boolean;
      preparedText?: string;
    }
  | { kind: 'pending'; operations: ChatOperation[] }
  | { kind: 'channels'; channels: ChatChannel[] }
  | { kind: 'none' };
export type HistoryEvent = Extract<ConversationEvent, { kind: 'history' }>;

/**
 * The history event for a page read with `requestBefore` as its `before`
 * cursor. `placement` carries what only the caller that asked for the page
 * knows: whether it is a range read past the held tail, and whether it leaves
 * the window short of the published head.
 */
export function historyEvent(
  page: Extract<ChatResult, { kind: 'history' }>,
  requestBefore: string | null,
  placement: { newer?: boolean; detached?: boolean } = {},
): HistoryEvent {
  return {
    kind: 'history',
    ...(page.gap === false ? { incremental: true as const } : {}),
    ...(placement.newer ? { newer: true as const } : {}),
    ...(placement.detached === undefined
      ? {}
      : { detached: placement.detached }),
    channel: page.channel,
    messages: page.messages,
    before: page.before,
    requestBefore,
    missing: page.missing_predecessors.length > 0,
  };
}

export function eventFromReply(
  action: ChatAction,
  result: ChatResult,
): ConversationEvent {
  switch (result.kind) {
    case 'operation-body':
      return {
        kind: 'body-recovered',
        operation: result.operation,
        channel: result.channel,
        text: result.text,
      };
    case 'history':
      return historyEvent(
        result,
        action.action === 'history' ? action.before : null,
      );
    case 'operation':
      return {
        kind: 'operation',
        operation: result.operation,
        discard:
          action.action === 'finalize' ||
          result.operation.state === 'cancelled',
        preparedText:
          action.action === 'prepare-message' ||
          action.action === 'submit-message'
            ? action.text
            : undefined,
      };
    case 'pending':
      return { kind: 'pending', operations: result.operations };
    case 'channels':
      return { kind: 'channels', channels: result.channels };
    case 'intent':
    case 'cleanup-pending':
    case 'inbox':
    case 'poll':
    case 'read':
      return { kind: 'none' };
  }
}
