import type { ChatOperation } from '../../chat-contract';
import { copyOperation } from '../operations';

/** What a row shows for one outgoing message, derived by `phaseOf`. */
export type SendPhase =
  | 'queued'
  | 'saving'
  | 'preparing'
  | 'sending'
  | 'not-sent'
  | 'unconfirmed'
  | 'sent'
  | 'cancelled'
  | 'paused';

/**
 * How far one submission has reached. Before the agent prepares an operation
 * for it, the stage records what the agent holds; afterwards it records what
 * the agent last reported about that operation.
 */
export type Delivery =
  /**
   * Waiting for the message ahead of it in this channel. It is held in this
   * session's memory alone: nothing has been written for it, so closing the
   * window discards it.
   */
  | { stage: 'queued' }
  /** The agent has not acknowledged a saved copy that is still in the slot. */
  | { stage: 'saving' }
  /**
   * The agent acknowledged the saved copy. `ambiguous` once a submit may have
   * reached the agent: a retry then asks it to prepare the same submission
   * instead of submitting it again, and the saved copy is never cleared
   * before an operation exists.
   */
  | { stage: 'preparing'; ambiguous: boolean }
  | { stage: 'prepared'; op: ChatOperation }
  /**
   * The operation's outcome is unknown: the agent reported it uncertain, or
   * an attempt failed and no report has arrived since.
   */
  | { stage: 'uncertain'; op: ChatOperation }
  | { stage: 'terminal'; op: ChatOperation };

/**
 * The channel's one saved-message slot as this message holds it. `pending`
 * from the moment a save is attempted, because a save whose acknowledgement
 * is lost may still have landed.
 */
export type IntentSlot =
  'none' | 'pending' | 'cleared' | { cleanupFailed: string };

/** The step a running drive is on. `check` reconciles instead of attempting. */
export type AttemptStep = 'save' | 'prepare' | 'deliver' | 'check';

export interface OutgoingMessage {
  readonly id: string;
  readonly channel: string;
  readonly submission: string;
  readonly createdAt: number;
  /**
   * Whether recovery drives it without being asked. A row rebuilt from an
   * operation body is not: only a person's Retry sends it.
   */
  readonly automatic: boolean;
  text?: string;
  delivery: Delivery;
  intent: IntentSlot;
  /** The message was found in verified channel history. */
  observed: boolean;
  /**
   * The newest drive's token, and its step while it runs. Events a drive
   * emits carry its token, and an event whose token is not the newest one is
   * discarded, so a continuation of an earlier attempt cannot change the
   * state of a later one.
   */
  attempt: { token: number; step: AttemptStep | null };
  /**
   * Why the newest drive stopped short of delivery: it failed, or chat or
   * the channel was unavailable to it. Starting a drive resets it.
   */
  stopped: 'failed' | 'access' | null;
  error: string;
}

export type OutgoingEvent =
  /** Not admitted: another message holds or is ahead in the channel. */
  | { type: 'queued' }
  /** Admitted, but chat or the channel is unavailable, so no drive starts. */
  | { type: 'waiting' }
  /** A drive starts; it takes the next token. */
  | { type: 'started'; step: AttemptStep }
  /** The drive asks the agent to save the submission. */
  | { type: 'saving'; token: number }
  /** The agent acknowledged the save. */
  | { type: 'saved'; token: number }
  /**
   * The drive asks the agent to prepare, and to send when nothing was asked
   * before. From this request on, the preparation's outcome is open until an
   * operation is known.
   */
  | { type: 'preparing'; token: number }
  /** The agent prepared an operation; `check` when delivery is unknown. */
  | { type: 'prepared'; token: number; op: ChatOperation; check: boolean }
  /** The drive's attempt or reconcile returned this operation state. */
  | { type: 'attempted'; token: number; op: ChatOperation }
  | {
      type: 'failed';
      token: number;
      error: string;
      /** Chat access changed while the request was in flight. */
      access: boolean;
    }
  | { type: 'finished'; token: number }
  /** A reply reported this operation, from any request. */
  | { type: 'status'; op: ChatOperation }
  | { type: 'observed' }
  /** The channel's access was withdrawn and its text with it. */
  | { type: 'revoked' }
  /**
   * The team was retired: the service stopped, or the team was removed,
   * replaced or rejected. The message is discarded and its text with it.
   */
  | { type: 'retired' }
  /** Recovery read the operation's body back from the agent. */
  | { type: 'body'; text: string }
  | { type: 'intent-cleared' }
  | { type: 'cleanup-failed'; error: string };

const terminalStates = new Set<ChatOperation['state']>([
  'confirmed',
  'cancelled',
  'rejected',
]);

/** The agent will not change this operation again. */
export function terminal(op: ChatOperation): boolean {
  return terminalStates.has(op.state);
}

export function operationOf(
  message: Pick<OutgoingMessage, 'delivery'>,
): ChatOperation | undefined {
  return 'op' in message.delivery ? message.delivery.op : undefined;
}

export function running(message: Pick<OutgoingMessage, 'attempt'>): boolean {
  return message.attempt.step !== null;
}

/** The agent may hold this message's saved copy in the channel's slot. */
export function intentHeld(message: Pick<OutgoingMessage, 'intent'>): boolean {
  return message.intent === 'pending' || typeof message.intent === 'object';
}

export function cleanupErrorOf(
  message: Pick<OutgoingMessage, 'intent'>,
): string {
  return typeof message.intent === 'object' ? message.intent.cleanupFailed : '';
}

/**
 * A preparation request may have reached the agent and no operation is known
 * yet. Every later preparation of the submission only asks the agent to
 * prepare it, never to submit it again, and its saved copy is kept.
 */
export function preparationAmbiguous(
  message: Pick<OutgoingMessage, 'delivery'>,
): boolean {
  return message.delivery.stage === 'preparing' && message.delivery.ambiguous;
}

/**
 * The agent may already have sent the message, or may still send it: its
 * operation's outcome is unknown, or a preparation may have reached the
 * agent. Such a message is checked, never edited or submitted again.
 */
export function outcomeUnknown(
  message: Pick<OutgoingMessage, 'delivery'>,
): boolean {
  return (
    message.delivery.stage === 'uncertain' || preparationAmbiguous(message)
  );
}

/**
 * Chat or the channel was unavailable to the newest drive and the message
 * may still change, so recovery drives it again once access returns.
 */
export function awaitingAccess(message: OutgoingMessage): boolean {
  return (
    message.stopped === 'access' &&
    !message.observed &&
    message.delivery.stage !== 'terminal'
  );
}

/**
 * Whether only this session holds the submission: it has no operation and
 * the agent has not acknowledged a saved copy that is still in the slot.
 */
export function unsent(message: OutgoingMessage): boolean {
  return (
    message.text !== undefined &&
    !operationOf(message) &&
    !(message.delivery.stage === 'preparing' && intentHeld(message))
  );
}

const encoder = new TextEncoder();
/** Each record's text and its UTF-8 length, measured once per text. */
const measured = new WeakMap<
  OutgoingMessage,
  { text: string; bytes: number }
>();

/**
 * The UTF-8 length of the message's text, as the send service's retention
 * bound counts it. The length is kept with the record until its text
 * changes, so a bound check over every retained message encodes only texts
 * it has not measured before.
 */
export function outgoingBytes(message: OutgoingMessage): number {
  const text = message.text;
  if (!text) return 0;
  const known = measured.get(message);
  if (known?.text === text) return known.bytes;
  const bytes = encoder.encode(text).length;
  measured.set(message, { text, bytes });
  return bytes;
}

export function deliveryOf(op: ChatOperation): Delivery {
  return terminal(op)
    ? { stage: 'terminal', op }
    : op.state === 'uncertain'
      ? { stage: 'uncertain', op }
      : { stage: 'prepared', op };
}

/**
 * The delivery a report of `op` leaves. The agent does not change a terminal
 * operation again, so a report that would reopen one is out of order and
 * leaves the delivery as it is.
 */
function reported(delivery: Delivery, op: ChatOperation): Delivery {
  return delivery.stage === 'terminal' && !terminal(op)
    ? delivery
    : deliveryOf(op);
}

/**
 * A copy that shares nothing mutable with `message`, for callers outside the
 * send service.
 */
export function copyOutgoing(message: OutgoingMessage): OutgoingMessage {
  const op = operationOf(message);
  return {
    ...message,
    delivery: op
      ? ({
          ...message.delivery,
          op: copyOperation(op),
        } as Delivery)
      : { ...message.delivery },
    intent:
      typeof message.intent === 'object'
        ? { ...message.intent }
        : message.intent,
    attempt: { ...message.attempt },
  };
}

export function newOutgoing(input: {
  channel: string;
  submission: string;
  text?: string;
  createdAt: number;
  automatic?: boolean;
  delivery?: Delivery;
  intent?: IntentSlot;
  id?: string;
}): OutgoingMessage {
  return {
    id: input.id ?? input.submission,
    channel: input.channel,
    submission: input.submission,
    createdAt: input.createdAt,
    automatic: input.automatic ?? true,
    text: input.text,
    delivery: input.delivery ?? { stage: 'saving' },
    // A fresh submission has nothing saved for it yet; the drive records the
    // save it is about to attempt before it can be acknowledged or lost.
    intent: input.intent ?? 'none',
    observed: false,
    attempt: { token: 0, step: null },
    stopped: null,
    error: '',
  };
}

/**
 * The next state of one outgoing message. Pure: the send service applies
 * every change to a message through this function, by way of {@link move}.
 */
export function transition(
  message: OutgoingMessage,
  event: OutgoingEvent,
): OutgoingMessage {
  if ('token' in event && event.token !== message.attempt.token) return message;
  const op = operationOf(message);
  switch (event.type) {
    case 'queued':
      // Only a message with no operation and no saved copy can wait behind
      // another; anything else is already the channel's to finish. An open
      // preparation is never reset, so it is never submitted a second time.
      if (op || intentHeld(message) || preparationAmbiguous(message))
        return message;
      return {
        ...message,
        delivery: { stage: 'queued' },
        stopped: null,
        error: '',
      };
    case 'waiting':
      return {
        ...message,
        delivery:
          message.delivery.stage === 'queued'
            ? { stage: 'saving' }
            : message.delivery,
        stopped: 'access',
      };
    case 'started':
      return {
        ...message,
        delivery:
          message.delivery.stage === 'queued'
            ? { stage: 'saving' }
            : message.delivery,
        attempt: { token: message.attempt.token + 1, step: event.step },
        stopped: null,
        error: '',
      };
    case 'saving':
      if (op) return message;
      return {
        ...message,
        attempt: { ...message.attempt, step: 'save' },
        ...(intentHeld(message)
          ? {}
          : {
              intent: 'pending' as const,
              // Nothing remains of an earlier acknowledged copy, so this
              // save has to be acknowledged before the agent holds one.
              delivery: preparationAmbiguous(message)
                ? message.delivery
                : { stage: 'saving' as const },
            }),
      };
    case 'saved':
      return message.delivery.stage === 'saving'
        ? { ...message, delivery: { stage: 'preparing', ambiguous: false } }
        : message;
    case 'preparing':
      if (op) return message;
      return {
        ...message,
        attempt: { ...message.attempt, step: 'prepare' },
        delivery: { stage: 'preparing', ambiguous: true },
      };
    case 'prepared':
      return {
        ...message,
        delivery: reported(message.delivery, event.op),
        attempt: {
          ...message.attempt,
          step: event.check ? 'check' : 'deliver',
        },
      };
    case 'attempted':
      // The attempt's reply supersedes an access change seen while it ran.
      return {
        ...message,
        delivery: reported(message.delivery, event.op),
        stopped: null,
      };
    case 'failed': {
      if (!running(message)) return message;
      if (op)
        return {
          ...message,
          // The attempt may have changed the operation; until a reply
          // reports it, its outcome is unknown.
          delivery:
            message.delivery.stage === 'terminal'
              ? message.delivery
              : { stage: 'uncertain', op },
          stopped: event.access ? 'access' : 'failed',
          error: event.error,
        };
      // A lost save acknowledgement cannot mean the message was delivered,
      // so only a preparation already open reads as unconfirmed, whatever
      // ended the attempt; access is recorded only when nothing else is open.
      return {
        ...message,
        stopped:
          !preparationAmbiguous(message) && event.access ? 'access' : 'failed',
        error: event.error,
      };
    }
    case 'finished':
      return { ...message, attempt: { ...message.attempt, step: null } };
    case 'status': {
      if (op && op.id !== event.op.id) return message;
      const delivery = reported(message.delivery, event.op);
      // A report that would reopen a terminal operation changes nothing.
      if (delivery === message.delivery) return message;
      return {
        ...message,
        delivery,
        ...(event.op.state === 'confirmed' ? { error: '' } : {}),
      };
    }
    case 'observed':
      return {
        ...message,
        observed: true,
        text: intentHeld(message) ? message.text : undefined,
      };
    case 'revoked':
      if (message.text === undefined) return message;
      return { ...message, text: undefined, stopped: 'access' };
    case 'retired':
      return { ...message, text: undefined };
    case 'body':
      return message.observed ? message : { ...message, text: event.text };
    case 'intent-cleared':
      if (!intentHeld(message)) return message;
      return {
        ...message,
        intent: 'cleared',
        text: message.observed ? undefined : message.text,
        // The acknowledged copy is gone, so the agent no longer holds this
        // submission for the next session.
        delivery:
          message.delivery.stage === 'preparing' &&
          !preparationAmbiguous(message)
            ? { stage: 'saving' }
            : message.delivery,
      };
    case 'cleanup-failed':
      if (!intentHeld(message)) return message;
      return { ...message, intent: { cleanupFailed: event.error } };
  }
}

/** The one display value for a message, derived from its state. */
export function phaseOf(message: OutgoingMessage): SendPhase {
  if (message.observed) return 'sent';
  const { delivery } = message;
  if (delivery.stage === 'terminal')
    return delivery.op.state === 'confirmed'
      ? 'sent'
      : delivery.op.state === 'cancelled'
        ? 'cancelled'
        : 'not-sent';
  // Chat or the channel was unavailable to the newest drive and none is
  // running, so a Retry or Check again cannot act until access returns; the
  // row says so even when the operation's outcome is unknown. Recovery
  // checks it once access returns.
  if (message.stopped === 'access' && !running(message)) return 'paused';
  if (delivery.stage === 'uncertain') return 'unconfirmed';
  if (message.stopped === 'access') return 'paused';
  // A recorded failure is shown, not the step the drive failed at, even
  // before the drive has finished.
  if (message.stopped === null)
    switch (message.attempt.step) {
      case 'save':
        return 'saving';
      case 'prepare':
        return 'preparing';
      case 'deliver':
        return 'sending';
      case 'check':
        return 'unconfirmed';
    }
  switch (delivery.stage) {
    case 'queued':
      return 'queued';
    case 'prepared':
      return 'not-sent';
    case 'preparing':
      if (delivery.ambiguous) return 'unconfirmed';
  }
  return message.stopped === 'failed' ? 'not-sent' : 'saving';
}

/** What a row offers for an outgoing message in its current state. */
export interface OutgoingActions {
  /**
   * Drives the message again: `check` when it may already have been sent, so
   * it is checked rather than sent a second time; `retry` otherwise.
   */
  retry?: 'retry' | 'check';
  /** Returns the text to the composer and removes the message. */
  edit: boolean;
}

/**
 * `unconfirmed` is set when the message's operation may already have reached
 * the server for a reason the message does not record, such as a status left
 * unknown by a failed request; the message is then checked, never edited.
 */
export function outgoingActions(
  message: OutgoingMessage,
  unconfirmed = false,
): OutgoingActions {
  const phase = phaseOf(message);
  const state = operationOf(message)?.state;
  const retryable =
    (phase === 'not-sent' || phase === 'unconfirmed' || phase === 'paused') &&
    state !== 'rejected' &&
    state !== 'cancelled';
  const check = unconfirmed || outcomeUnknown(message);
  return {
    retry: retryable
      ? phase === 'unconfirmed' || check
        ? 'check'
        : 'retry'
      : undefined,
    edit:
      (phase === 'not-sent' || phase === 'queued') &&
      !running(message) &&
      !check &&
      message.text !== undefined,
  };
}

/**
 * Applies `event` to `message` in place. The record keeps its identity,
 * because a drive in flight holds it across its awaits.
 */
export function move(message: OutgoingMessage, event: OutgoingEvent) {
  Object.assign(message, transition(message, event));
}
