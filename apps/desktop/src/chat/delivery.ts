import type { ChatOperation } from '../chat-contract';
import type { TrackedOperation } from './operations';
import {
  operationOf,
  phaseOf,
  running,
  type OutgoingMessage,
} from './send/outgoing';

/**
 * The one vocabulary every row of your own unsent work uses, whether the row
 * is an outgoing message of this session or a tracked operation restored from
 * the agent's ledger. A sent message has no delivery state.
 *
 * - `sending`: a save, preparation or delivery is running.
 * - `queued`: waiting behind another message of the same channel.
 * - `not-sent`: saved and known not to have reached the server.
 * - `checking`: the operation's status is unknown after a failed request,
 *   and it is checked under the same ID rather than sent again.
 * - `unconfirmed`: the operation may have reached the server.
 * - `rejected`: the server refused it before committing it.
 * - `cancelled`: cancelled before it was sent.
 * - `paused`: chat or the channel was unavailable to the last attempt.
 */
export type DeliveryState =
  | 'sending'
  | 'queued'
  | 'not-sent'
  | 'checking'
  | 'unconfirmed'
  | 'rejected'
  | 'cancelled'
  | 'paused';

const LABELS: Record<DeliveryState, string> = {
  sending: 'Sending',
  queued: 'Queued',
  'not-sent': 'Not sent',
  checking: 'Checking delivery',
  unconfirmed: 'Couldn’t confirm delivery',
  rejected: 'Couldn’t post',
  cancelled: 'Cancelled',
  paused: 'Waiting for access',
};

/** The state's label; a refused channel creation reads "Couldn't create". */
export function deliveryLabel(
  state: DeliveryState,
  kind: ChatOperation['kind'] = 'send-message',
): string {
  return state === 'rejected' && kind === 'create-channel'
    ? 'Couldn’t create'
    : LABELS[state];
}

/**
 * Whether the state leaves work for the reader to settle: counted by the
 * unsent chip. Sending and Queued settle by themselves, and Cancelled is
 * settled.
 */
export function deliveryNeedsAttention(state: DeliveryState | null): boolean {
  return (
    state === 'not-sent' ||
    state === 'checking' ||
    state === 'unconfirmed' ||
    state === 'rejected' ||
    state === 'paused'
  );
}

/** The state of a tracked operation, or `null` once it is confirmed. */
export function operationDelivery(op: TrackedOperation): DeliveryState | null {
  if (
    op.statusUnknown &&
    !['confirmed', 'rejected', 'cancelled'].includes(op.state)
  )
    return 'checking';
  switch (op.state) {
    case 'prepared':
      return 'not-sent';
    case 'uncertain':
      return 'unconfirmed';
    case 'rejected':
      return 'rejected';
    case 'cancelled':
      return 'cancelled';
    case 'confirmed':
      return null;
  }
}

/** The state of an outgoing message, or `null` once it is sent. */
export function outgoingDelivery(
  message: OutgoingMessage,
): DeliveryState | null {
  const phase = phaseOf(message);
  switch (phase) {
    case 'saving':
    case 'preparing':
    case 'sending':
      return 'sending';
    case 'queued':
      return 'queued';
    case 'not-sent':
      return operationOf(message)?.state === 'rejected'
        ? 'rejected'
        : 'not-sent';
    case 'unconfirmed':
      // A check that is running reads as one; a settled unconfirmed message
      // waits for the reader's Check again.
      return running(message) ? 'checking' : 'unconfirmed';
    case 'paused':
      return 'paused';
    case 'cancelled':
      return 'cancelled';
    case 'sent':
      return null;
  }
}

/**
 * The server statuses the client treats as a definite rejection before
 * commit (`definite_rejection` in foks-client's `operations/attempt.rs`),
 * with their generated status names, which only Details shows.
 */
const REJECTION_STATUS: Readonly<Record<number, string>> = {
  1013: 'STATUS_PERMISSION_ERROR',
  1030: 'STATUS_BAD_ARGS_ERROR',
  1060: 'STATUS_OVER_QUOTA_ERROR',
  12002: 'STATUS_RT_CHANNEL_EXISTS_ERROR',
  12003: 'STATUS_RT_RACE',
  12005: 'STATUS_RT_NOT_FOUND_ERROR',
  12006: 'STATUS_RT_MSG_ORDER_ERROR',
};

/** The generated status name of a rejection code, for Details. */
export function rejectionStatusName(code: number | null): string | undefined {
  return code === null ? undefined : REJECTION_STATUS[code];
}

/**
 * Why the server refused an operation, in one sentence. `place` names the
 * channel for a message ("#general"), or the team for a channel creation;
 * it defaults to "this channel" and "this team". The numeric code is not part
 * of the sentence: Details shows it.
 */
export function rejectionReason(
  code: number | null,
  kind: ChatOperation['kind'],
  place?: string,
): string {
  if (kind === 'create-channel') {
    const team = place ?? 'this team';
    switch (code) {
      case 1013:
        return `Your role cannot create channels in ${team}.`;
      case 1030:
        return 'The server refused the channel as malformed.';
      case 1060:
        return `${team === 'this team' ? 'This team' : team} has reached the server’s limit on channels or members.`;
      case 12002:
        return `${team === 'this team' ? 'This team' : team} already has a channel with that name.`;
      case 12003:
        return `${team === 'this team' ? 'This team' : team} changed while the channel was waiting.`;
      default:
        return 'The server refused the channel.';
    }
  }
  const channel = place ?? 'this channel';
  switch (code) {
    case 1013:
      return `You can no longer post in ${channel}.`;
    case 1030:
      return 'The server refused the message as malformed.';
    case 1060:
      return 'The team has more members than the server delivers chat to.';
    case 12003:
      return `${channel === 'this channel' ? 'This channel' : channel} changed while the message was waiting.`;
    case 12005:
      return `${channel === 'this channel' ? 'This channel' : channel} no longer exists.`;
    case 12006:
      return `New messages arrived in ${channel} first.`;
    default:
      return 'The server refused the message.';
  }
}

/**
 * What a row in `state` says under its text, apart from a rejection, which
 * {@link rejectionReason} states.
 */
export function deliveryExplanation(
  state: DeliveryState,
  kind: ChatOperation['kind'] = 'send-message',
): string {
  const noun = kind === 'create-channel' ? 'channel' : 'message';
  switch (state) {
    case 'sending':
      return '';
    case 'queued':
      return `Waiting for the ${noun} ahead of it in this channel.`;
    case 'not-sent':
      return `Saved on this device. The ${noun} has not reached the server.`;
    case 'checking':
      return 'Delivery is checked under the same operation ID, so nothing is sent twice.';
    case 'unconfirmed':
      return `The server may have received this ${noun}. Check again before writing it again.`;
    case 'rejected':
      return '';
    case 'cancelled':
      return 'Cancelled before it was sent.';
    case 'paused':
      return 'Chat access is unavailable. Delivery resumes once it returns.';
  }
}
