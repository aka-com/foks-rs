import { useState } from 'react';
import type { Bridge } from '../bridge';
import { Button } from '../components';
import { AccountMark } from '../screens/account-switcher';
import type { ChatSendService } from './send-service';
import {
  cleanupErrorOf,
  operationOf,
  outcomeUnknown,
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
  message,
  grouped = false,
  avatarName,
  storeId,
  service,
  bridge,
}: {
  message: OutgoingMessage;
  grouped?: boolean;
  avatarName: string;
  storeId: string;
  service: ChatSendService;
  bridge: Bridge;
}) {
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
  const retryable =
    ['not-sent', 'unconfirmed', 'paused'].includes(phase) &&
    !['rejected', 'cancelled'].includes(op?.state ?? '');
  const editable =
    (phase === 'not-sent' || queued) &&
    !busy &&
    !outcomeUnknown(message) &&
    message.text !== undefined;
  return (
    <article
      className={`chat-message chat-outgoing${grouped ? ' grouped' : ''}${sent ? ' sent' : ''}${sending ? ' sending' : ''}${queued ? ' queued' : ''}`}
      data-submission={message.submission}
      data-operation={op?.id}
      title={grouped ? messageTime(String(message.createdAt)) : undefined}
    >
      <AccountMark name={avatarName} className="chat-avatar" />
      <div className="chat-message-body">
        <div className="chat-outgoing-content">
          <header>
            <span className={grouped ? 'offscreen' : 'chat-sender you'}>
              You
            </span>
            {sending ? (
              <span
                className="chat-send-spinner"
                role="status"
                aria-label="Sending"
              />
            ) : (
              <time
                className={grouped ? 'offscreen' : undefined}
                dateTime={new Date(message.createdAt).toISOString()}
                title={messageTime(String(message.createdAt))}
              >
                {relativeMessageTime(String(message.createdAt))}
              </time>
            )}
            {!sending && statusLabel && (
              <span
                className={queued ? 'offscreen' : 'chat-send-state'}
                role="status"
              >
                {statusLabel}
              </span>
            )}
            {!sending && editable && (
              <button
                type="button"
                className="chat-edit-link"
                onClick={() => void run(true)}
              >
                Edit
              </button>
            )}
          </header>
          {message.text !== undefined && (
            <MessageText text={message.text} actions={bridge} />
          )}
        </div>
        {!sending && (message.error || retryable || cleanupError || error) ? (
          <div className="chat-send-status">
            {message.error && <span>{message.error}</span>}
            {retryable && (
              <Button size="sm" disabled={busy} onClick={() => void run(false)}>
                {phase === 'unconfirmed' || outcomeUnknown(message)
                  ? 'Check again'
                  : 'Retry'}
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
        ) : null}
      </div>
    </article>
  );
}
