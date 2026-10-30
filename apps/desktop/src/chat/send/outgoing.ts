import type { ChatOperation } from '../../chat-contract';

/** UI status phase for an outgoing message, derived by `phaseOf`. */
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
 * Delivery progress for a submission. Before the agent prepares an operation,
 * the stage tracks persisted intent state; afterwards it tracks the last
 * reported operation state.
 */
export type Delivery =
  /**
   * Queued behind an earlier message in this channel. Stored only in memory
   * for the current session; closing the window discards it.
   */
  | { stage: 'queued' }
  /** The agent has not yet acknowledged saving the intent to the channel slot. */
  | { stage: 'saving' }
  /**
   * The agent acknowledged the saved intent. `ambiguous` is true once a submit
   * request may have reached the agent; retries then call `prepare-message`
   * with the same submission ID instead of `submit-message`, and the saved
   * intent is retained until an operation exists.
   */
  | { stage: 'preparing'; ambiguous: boolean }
  | { stage: 'prepared'; op: ChatOperation }
  /**
   * The operation outcome is unknown because the agent reported `uncertain` or
   * a delivery attempt failed without a subsequent status report.
   */
  | { stage: 'uncertain'; op: ChatOperation }
  | { stage: 'terminal'; op: ChatOperation };

/**
 * State of this message's saved intent in the channel's single intent slot.
 * Set to `pending` as soon as a save starts, because a save whose
 * acknowledgement is lost may still have been persisted.
 */
export type IntentSlot =
  'none' | 'pending' | 'cleared' | { cleanupFailed: string };

/** Active step of a running drive. `check` reconciles instead of attempting delivery. */
export type AttemptStep = 'save' | 'prepare' | 'deliver' | 'check';

export interface OutgoingMessage {
  readonly id: string;
  readonly channel: string;
  readonly submission: string;
  readonly createdAt: number;
  /**
   * Whether background recovery automatically drives this message. False for
   * rows reconstructed from an operation body, which require an explicit
   * user retry.
   */
  readonly automatic: boolean;
  text?: string;
  delivery: Delivery;
  intent: IntentSlot;
  /** Whether the message appears in verified channel history. */
  observed: boolean;
  /**
   * Token and current step for the latest drive. Drive events must match this
   * token; events from earlier drives with stale tokens are ignored.
   */
  attempt: { token: number; step: AttemptStep | null };
  /**
   * Reason the latest drive stopped before completing delivery: a request
   * failure, or unavailable chat/channel access. Reset when a new drive starts.
   */
  stopped: 'failed' | 'access' | null;
  error: string;
}

export type OutgoingEvent =
  /** Not admitted because another message in the channel is ahead of it or holds the intent slot. */
  | { type: 'queued' }
  /** Admitted, but chat or channel access is unavailable, so no drive starts. */
  | { type: 'waiting' }
  /** Starts a new drive and increments the attempt token. */
  | { type: 'started'; step: AttemptStep }
  /** The drive is saving the submission intent to the agent. */
  | { type: 'saving'; token: number }
  /** The agent acknowledged saving the intent. */
  | { type: 'saved'; token: number }
  /**
   * The drive is issuing `submit-message` or `prepare-message`. Marks
   * preparation as ambiguous until an operation is returned.
   */
  | { type: 'preparing'; token: number }
  /** The agent returned a prepared operation; `check` is true when delivery status is uncertain. */
  | { type: 'prepared'; token: number; op: ChatOperation; check: boolean }
  /** The drive's `attempt` or `reconcile` request returned this operation state. */
  | { type: 'attempted'; token: number; op: ChatOperation }
  | {
      type: 'failed';
      token: number;
      error: string;
      /** Chat access changed while the request was in flight. */
      access: boolean;
    }
  | { type: 'finished'; token: number }
  /** An operation state reported by any agent reply. */
  | { type: 'status'; op: ChatOperation }
  | { type: 'observed' }
  /** Channel access was revoked; clears in-memory message text. */
  | { type: 'revoked' }
  /**
   * The team was retired (service stopped, or team removed, replaced, or
   * blocked); clears in-memory message text.
   */
  | { type: 'retired' }
  /** Recovery loaded the operation body from the agent. */
  | { type: 'body'; text: string }
  | { type: 'intent-cleared' }
  | { type: 'cleanup-failed'; error: string };

const terminalStates = new Set<ChatOperation['state']>([
  'confirmed',
  'cancelled',
  'rejected',
]);

/** Whether the operation is in a terminal state (`confirmed`, `cancelled`, or `rejected`). */
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

/** Whether this message still occupies the channel's saved-intent slot. */
export function intentHeld(message: Pick<OutgoingMessage, 'intent'>): boolean {
  return message.intent === 'pending' || typeof message.intent === 'object';
}

export function cleanupErrorOf(
  message: Pick<OutgoingMessage, 'intent'>,
): string {
  return typeof message.intent === 'object' ? message.intent.cleanupFailed : '';
}

/**
 * Whether a submit or prepare request may have reached the agent without
 * returning an operation yet. Subsequent drives must use `prepare-message`
 * rather than `submit-message` and keep the saved intent.
 */
export function preparationAmbiguous(
  message: Pick<OutgoingMessage, 'delivery'>,
): boolean {
  return message.delivery.stage === 'preparing' && message.delivery.ambiguous;
}

/**
 * Whether the message may already have been sent or may still be sent because
 * its operation state is uncertain or preparation is ambiguous. Callers must
 * reconcile rather than edit or resubmit.
 */
export function outcomeUnknown(
  message: Pick<OutgoingMessage, 'delivery'>,
): boolean {
  return (
    message.delivery.stage === 'uncertain' || preparationAmbiguous(message)
  );
}

/**
 * Whether the latest drive stopped due to unavailable chat or channel access
 * while the message is not yet observed or terminal, so recovery should retry
 * when access returns.
 */
export function awaitingAccess(message: OutgoingMessage): boolean {
  return (
    message.stopped === 'access' &&
    !message.observed &&
    message.delivery.stage !== 'terminal'
  );
}

/**
 * Whether the submission exists only in memory for this session (no prepared
 * operation and no acknowledged saved intent still held in the slot).
 */
export function unsent(message: OutgoingMessage): boolean {
  return (
    message.text !== undefined &&
    !operationOf(message) &&
    !(message.delivery.stage === 'preparing' && intentHeld(message))
  );
}

export function deliveryOf(op: ChatOperation): Delivery {
  return terminal(op)
    ? { stage: 'terminal', op }
    : op.state === 'uncertain'
      ? { stage: 'uncertain', op }
      : { stage: 'prepared', op };
}

/**
 * Computes the next delivery state from a reported `op`, ignoring stale
 * non-terminal reports once `delivery` is already terminal.
 */
function reported(delivery: Delivery, op: ChatOperation): Delivery {
  return delivery.stage === 'terminal' && !terminal(op)
    ? delivery
    : deliveryOf(op);
}

/**
 * Returns a deep clone of `message` so external callers cannot mutate service
 * state.
 */
export function copyOutgoing(message: OutgoingMessage): OutgoingMessage {
  const op = operationOf(message);
  return {
    ...message,
    delivery: op
      ? ({
          ...message.delivery,
          op: {
            ...op,
            confirmation: op.confirmation ? { ...op.confirmation } : null,
          },
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
    // Starts as `'none'`; the drive transitions to `'pending'` immediately
    // before issuing `save-intent`.
    intent: input.intent ?? 'none',
    observed: false,
    attempt: { token: 0, step: null },
    stopped: null,
    error: '',
  };
}

/**
 * Pure state transition function for an outgoing message.
 */
export function transition(
  message: OutgoingMessage,
  event: OutgoingEvent,
): OutgoingMessage {
  if ('token' in event && event.token !== message.attempt.token) return message;
  const op = operationOf(message);
  switch (event.type) {
    case 'queued':
      // Only a message without an operation, held intent, or ambiguous
      // preparation can be queued; otherwise it is already active in the
      // channel and must not be reset to `queued`.
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
              // No acknowledged intent is currently held, so return to
              // `saving` unless preparation is already ambiguous.
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
      // A completed attempt reply clears any access-change state from the drive.
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
          // A failed attempt may still have mutated the operation on the
          // agent, so mark non-terminal delivery as `uncertain` until a status
          // reply arrives.
          delivery:
            message.delivery.stage === 'terminal'
              ? message.delivery
              : { stage: 'uncertain', op },
          stopped: event.access ? 'access' : 'failed',
          error: event.error,
        };
      // A failed save cannot have delivered the message, so only an already
      // ambiguous preparation is treated as unconfirmed; record `'access'`
      // only when preparation is not ambiguous.
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
      // Ignore stale non-terminal reports for an already terminal operation.
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
        // With the saved intent cleared before an ambiguous preparation, revert
        // delivery stage to `saving` so a future drive saves it again.
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

/** Derives the UI `SendPhase` for an outgoing message from its current state. */
export function phaseOf(message: OutgoingMessage): SendPhase {
  if (message.observed) return 'sent';
  const { delivery } = message;
  if (delivery.stage === 'terminal')
    return delivery.op.state === 'confirmed'
      ? 'sent'
      : delivery.op.state === 'cancelled'
        ? 'cancelled'
        : 'not-sent';
  // When the latest drive stopped due to unavailable access and no drive is
  // running, show `paused` even if the operation outcome is uncertain;
  // recovery will reconcile once access returns.
  if (message.stopped === 'access' && !running(message)) return 'paused';
  if (delivery.stage === 'uncertain') return 'unconfirmed';
  if (message.stopped === 'access') return 'paused';
  // Only report the active step while `stopped` is null so a recorded failure
  // takes precedence before `finished` is emitted.
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
