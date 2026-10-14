import { useState } from 'react';
import { Button } from '../components';
import { useChatConversationContext } from './conversation-provider';
import { MessageRow } from './message-row';
import { useChatSendService } from './send-provider';
import {
  cleanupErrorOf,
  operationOf,
  phaseOf,
  running,
  type OutgoingMessage,
  type SendPhase,
} from './send/outgoing';
import { MessageText } from './message-text';
import { failure } from './actions';
import { messageTime, relativeMessageTime } from './presentation';

const statusLabels: Partial<Record<SendPhase, string>> = {
  queued: 'Queued',
  'not-sent': 'Not sent',
  unconfirmed: 'Delivery unconfirmed',
  paused: 'Waiting for access',
  cancelled: 'Cancelled',
};
export function OutgoingRow({
  rowKey,
  message,
  grouped = false,
  avatarName,
}: {
  /** The row's key in the list's toolbar tab stop. */
  rowKey?: string;
  message: OutgoingMessage;
  grouped?: boolean;
  avatarName: string;
}) {
  const { bridge, storeId, mentions } = useChatConversationContext();
  const service = useChatSendService();
  const [error, setError] = useState('');
  const run = async (edit: boolean) => {
    setError('');
    try {
      if (edit) await service.restoreDraft(storeId, message.id);
      else await service.retry(storeId, message.id);
    } catch (cause) {
      setError(failure(cause));
    }
  };
  const phase = phaseOf(message);
  const op = operationOf(message);
  const busy = running(message);
  const cleanupError = cleanupErrorOf(message);
  const sent = phase === 'sent';
  const sending = ['saving', 'preparing', 'sending'].includes(phase);
  // Waiting behind another message of this channel, and held in memory only.
  const queued = phase === 'queued';
  const statusLabel = statusLabels[phase];
  const actions = service.outgoingActions(storeId, message);
  const createdAt = String(message.createdAt);
  return (
    <MessageRow
      className={`chat-outgoing${sent ? ' sent' : ''}${sending ? ' sending' : ''}${queued ? ' queued' : ''}`}
      grouped={grouped}
      avatarName={avatarName}
      author="You"
      own
      at={createdAt}
      timeTitle={
        sending
          ? null
          : `${messageTime(createdAt)} · ${relativeMessageTime(createdAt)}`
      }
      attributes={{
        'data-submission': message.submission,
        'data-operation': op?.id,
      }}
      // Copy only: the message is not in history yet, so it has no sequence
      // or server time to detail, and quoting your own unsent text is not
      // offered.
      actions={{ rowKey, text: message.text }}
      status={
        sending ? (
          <span
            className="chat-send-spinner"
            role="status"
            aria-label="Sending"
          />
        ) : (
          <>
            {statusLabel && (
              <span
                className={queued ? 'offscreen' : 'chat-send-state'}
                role="status"
              >
                {statusLabel}
              </span>
            )}
            {actions.edit && (
              <button
                type="button"
                className="chat-edit-link"
                onClick={() => void run(true)}
              >
                Edit
              </button>
            )}
          </>
        )
      }
      footer={
        !sending &&
        (message.error || actions.retry || cleanupError || error) ? (
          <div className="chat-send-status">
            {message.error && <span>{message.error}</span>}
            {actions.retry && (
              <Button size="sm" disabled={busy} onClick={() => void run(false)}>
                {actions.retry === 'check' ? 'Check again' : 'Retry'}
              </Button>
            )}
            {cleanupError && (
              <>
                <span role="status">
                  Local message storage needs attention. {cleanupError}
                </span>
                <Button
                  size="sm"
                  disabled={busy}
                  onClick={() => void run(false)}
                >
                  Retry local cleanup
                </Button>
              </>
            )}
            {error && <span role="alert">{error}</span>}
          </div>
        ) : null
      }
    >
      {message.text !== undefined && (
        <MessageText text={message.text} actions={bridge} mentions={mentions} />
      )}
    </MessageRow>
  );
}
