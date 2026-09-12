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
  | { kind: 'access'; readable: ReadonlySet<string> }
  | { kind: 'status-unresolved'; operation: string }
  | { kind: 'reset' }
  | { kind: 'none' };

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
      return {
        kind: 'history',
        channel: result.channel,
        messages: result.messages,
        before: result.before,
        requestBefore: action.action === 'history' ? action.before : null,
        missing: result.missing_predecessors.length > 0,
      };
    case 'operation':
      return {
        kind: 'operation',
        operation: result.operation,
        discard:
          action.action === 'finalize' ||
          result.operation.state === 'cancelled',
        preparedText:
          action.action === 'prepare-message' ? action.text : undefined,
      };
    case 'pending':
      return { kind: 'pending', operations: result.operations };
    case 'channels':
      return { kind: 'channels', channels: result.channels };
    case 'inbox':
    case 'poll':
    case 'read':
      return { kind: 'none' };
  }
}
