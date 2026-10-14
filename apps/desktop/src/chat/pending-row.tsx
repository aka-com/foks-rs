import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Button } from '../components';
import type { ChatOperation } from '../chat-contract';
import { shortId } from '../model';
import { failure } from './actions';
import type { TrackedOperation } from './operations';
import { useChatSendService } from './send-provider';
const PENDING_STATE: Record<ChatOperation['state'], string> = {
  prepared: 'Not sent',
  uncertain: 'Delivery unconfirmed',
  confirmed: 'Sent',
  rejected: 'Not sent',
  cancelled: 'Cancelled',
};

function pendingExplanation(op: ChatOperation): string {
  switch (op.state) {
    case 'prepared':
      return 'Saved on this device and not sent yet.';
    case 'uncertain':
      return 'The server may have received this message. Check delivery status before retrying.';
    case 'confirmed':
      return 'Sent. Local cleanup runs automatically.';
    case 'cancelled':
      return 'Cancelled before it was sent.';
    case 'rejected':
      return `The server rejected this operation${
        op.rejection_code !== null ? ` (${op.rejection_code})` : ''
      }. Prepare a new message after resolving the error.`;
  }
}

/**
 * A tracked operation shown by itself rather than in an outgoing message's
 * row: in the thread when no outgoing message carries it, and in 'Needs
 * attention' when it is outside the open channel. The send service decides
 * which requests the operation accepts and sends them; this row only offers
 * them.
 */
export function PendingRow({
  storeId,
  operation,
  channelName,
  onChange,
}: {
  storeId: string;
  operation: TrackedOperation;
  channelName: string | undefined;
  onChange: () => void;
}): ReactNode {
  const op = operation;
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
  return (
    <div className="chat-pending" data-operation={op.id}>
      <span className="chat-pending-title">
        {op.kind === 'create-channel' ? 'Channel' : 'Message'} ·{' '}
        {op.statusUnknown ? 'Checking status' : PENDING_STATE[op.state]}
      </span>
      {channelName && <small>{channelName}</small>}
      <details>
        <summary>Details</summary>
        <span title={op.id}>
          {shortId(op.id)} · {op.state}
        </span>
      </details>
      <p>
        {op.statusUnknown
          ? 'Delivery status is unavailable. This saved operation will be checked using the same ID.'
          : pendingExplanation(op)}
      </p>
      {op.kind === 'send-message' && op.text === undefined && (
        <p>
          Saved text is unavailable in this view. The original operation
          identity is retained; checking delivery will not resend it.
        </p>
      )}
      <div className="chat-pending-actions">
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
      </div>
      {error && (
        <p role="alert" className="action-error">
          {error}
        </p>
      )}
    </div>
  );
}
