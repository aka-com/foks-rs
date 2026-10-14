import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Button } from '../components';
import { failure } from './actions';
import {
  deliveryExplanation,
  deliveryLabel,
  operationDelivery,
  rejectionReason,
  type DeliveryState,
} from './delivery';
import type { DeliveryDetails } from './message-actions';
import type { OperationActions, TrackedOperation } from './operations';
import { useChatSendService } from './send-provider';

/**
 * The requests a tracked operation's row offers and the call that sends one.
 * The send service decides which requests the operation accepts and sends
 * them; the row only offers them.
 */
export function useOperationRequests(
  storeId: string,
  op: TrackedOperation,
  onChange: () => void,
): {
  actions: OperationActions;
  busy: boolean;
  error: string;
  run: (action: 'deliver' | 'cancel') => Promise<void>;
} {
  const service = useChatSendService();
  const actions = service.operationActions(storeId, op.id);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const active = useRef(true);
  const running = useRef(false);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);
  const run = async (action: 'deliver' | 'cancel') => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setError('');
    try {
      const sent =
        action === 'deliver'
          ? await service.deliverOperation(storeId, op.id)
          : await service.cancelOperation(storeId, op.id);
      if (sent && active.current) onChange();
    } catch (e) {
      if (active.current) setError(failure(e));
    } finally {
      running.current = false;
      if (active.current) setBusy(false);
    }
  };
  return { actions, busy, error, run };
}

/** What Details lists for a tracked operation. */
export function operationDetails(op: TrackedOperation): DeliveryDetails {
  return {
    operation: op.id,
    kind: op.kind,
    state: op.state,
    statusUnknown: op.statusUnknown === true,
    rejectionCode: op.rejection_code,
  };
}

/**
 * What a tracked operation's row says under its text: the rejection reason or
 * the state's explanation. `place` names the channel of a message, or the
 * team of a channel creation.
 */
export function operationLine(
  op: TrackedOperation,
  state: DeliveryState | null,
  place?: string,
): string {
  if (state === null) return '';
  return state === 'rejected'
    ? rejectionReason(op.rejection_code, op.kind, place)
    : deliveryExplanation(state, op.kind);
}

/** The state label after a delivery row's header. */
export function DeliveryStatus({
  state,
  kind,
}: {
  state: DeliveryState | null;
  kind?: TrackedOperation['kind'];
}): ReactNode {
  if (state === null) return null;
  if (state === 'sending')
    return (
      <span
        className="chat-send-spinner"
        role="status"
        aria-label={deliveryLabel(state)}
      />
    );
  return (
    <span
      // Queued settles by itself, so it is read out but not drawn.
      className={
        state === 'queued' ? 'offscreen' : `chat-send-state ${stateTone(state)}`
      }
      role="status"
    >
      {deliveryLabel(state, kind)}
    </span>
  );
}

function stateTone(state: DeliveryState): string {
  return state === 'rejected' || state === 'not-sent'
    ? 'crit'
    : state === 'unconfirmed' || state === 'paused' || state === 'checking'
      ? 'warn'
      : '';
}

/** The verbs a tracked operation accepts, as buttons. */
function OperationButtons({
  requests,
}: {
  requests: ReturnType<typeof useOperationRequests>;
}): ReactNode {
  const { actions, busy, run } = requests;
  return (
    <>
      {actions.deliver && (
        <Button size="sm" disabled={busy} onClick={() => void run('deliver')}>
          {actions.deliver === 'reconcile' ? 'Check again' : 'Retry'}
        </Button>
      )}
      {actions.cancel && (
        <Button size="sm" disabled={busy} onClick={() => void run('cancel')}>
          Cancel
        </Button>
      )}
    </>
  );
}

/**
 * The delivery line under a tracked operation's text in the thread: what
 * happened, the requests the operation accepts, and a failed request's error.
 */
export function OperationFooter({
  op,
  state,
  place,
  requests,
}: {
  op: TrackedOperation;
  state: DeliveryState | null;
  place?: string;
  requests: ReturnType<typeof useOperationRequests>;
}): ReactNode {
  const line = operationLine(op, state, place);
  const { actions, error } = requests;
  if (!line && !actions.deliver && !actions.cancel && !error) return null;
  return (
    <div className="chat-send-status">
      {line && <span>{line}</span>}
      <OperationButtons requests={requests} />
      {error && <span role="alert">{error}</span>}
    </div>
  );
}

/**
 * A tracked operation outside the open thread, listed in the unsent chip's
 * popover: a channel creation, or a message in another channel or in one the
 * reader can no longer read. It names where the work belongs, quotes the
 * saved text, and offers the operation's requests and, for a message in a
 * channel the reader can open, Jump.
 */
export function PendingRow({
  storeId,
  operation,
  channelName,
  teamName,
  onChange,
  onJump,
}: {
  storeId: string;
  operation: TrackedOperation;
  /** The channel's title, for a message in a listed channel. */
  channelName: string | undefined;
  teamName?: string;
  onChange: () => void;
  /** Opens the message's channel; omitted where it cannot be opened. */
  onJump?: () => void;
}): ReactNode {
  const op = operation;
  const state = operationDelivery(op);
  const requests = useOperationRequests(storeId, op, onChange);
  const line = operationLine(
    op,
    state,
    op.kind === 'create-channel' ? teamName : channelName,
  );
  return (
    <div className="chat-pending" data-operation={op.id}>
      <span className="chat-pending-title">
        {op.kind === 'create-channel'
          ? 'New channel'
          : channelName
            ? `Message in ${channelName}`
            : 'Message'}
        {state !== null && (
          <span className={`chat-send-state ${stateTone(state)}`}>
            {deliveryLabel(state, op.kind)}
          </span>
        )}
      </span>
      {op.text ? (
        <q className="chat-pending-text">{op.text}</q>
      ) : (
        op.kind === 'send-message' &&
        op.text === undefined && (
          <small>Saved text is unavailable in this view.</small>
        )
      )}
      {line && <p>{line}</p>}
      <div className="chat-pending-actions">
        <OperationButtons requests={requests} />
        {onJump && (
          <Button size="sm" variant="quiet" onClick={onJump}>
            Jump
          </Button>
        )}
      </div>
      {requests.error && (
        <p role="alert" className="action-error">
          {requests.error}
        </p>
      )}
    </div>
  );
}
