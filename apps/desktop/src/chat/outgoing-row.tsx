import { useState } from 'react';
import type { Bridge } from '../bridge';
import { Button } from '../components';
import type { ChatSendService, OutgoingMessage } from './send-service';
import { MessageText } from './message-text';
import { failure } from './actions';

const labels = {
  saving: 'Sending…',
  preparing: 'Sending…',
  sending: 'Sending…',
  'not-sent': 'Not sent',
  unconfirmed: 'Delivery unconfirmed',
  sent: 'Sent',
  paused: 'Waiting for access',
  cancelled: 'Cancelled',
};
export function OutgoingRow({
  message,
  storeId,
  service,
  bridge,
}: {
  message: OutgoingMessage;
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
  const sent = message.phase === 'sent';
  const retryable =
    ['not-sent', 'unconfirmed', 'paused'].includes(message.phase) &&
    !['rejected', 'cancelled'].includes(message.operation?.state ?? '');
  const editable =
    message.phase === 'not-sent' &&
    !message.running &&
    (!message.ambiguousPreparation || !!message.operation) &&
    message.text !== undefined;
  return (
    <article
      className={`chat-message chat-outgoing${sent ? ' sent' : ''}`}
      data-submission={message.submission}
      data-operation={message.operation?.id}
    >
      <div className="chat-outgoing-content">
        <header>
          <span className="chat-sender you">You</span>
          <time dateTime={new Date(message.createdAt).toISOString()}>
            {new Date(message.createdAt).toLocaleTimeString([], {
              hour: 'numeric',
              minute: '2-digit',
            })}
          </time>
        </header>
        {message.text !== undefined && (
          <MessageText text={message.text} actions={bridge} />
        )}
      </div>
      <div className="chat-send-status">
        <span role="status">{labels[message.phase]}</span>
        {retryable && (
          <Button
            size="sm"
            disabled={message.running}
            onClick={() => void run(false)}
          >
            {message.phase === 'unconfirmed' ? 'Check again' : 'Retry'}
          </Button>
        )}
        {editable && (
          <Button size="sm" onClick={() => void run(true)}>
            Edit
          </Button>
        )}
        {message.cleanupError && (
          <span role="status">
            Local message storage needs attention. {message.cleanupError}
          </span>
        )}
        {error && <span role="alert">{error}</span>}
        <details>
          <summary>Details</summary>
          {message.submission && <div>Submission: {message.submission}</div>}
          {message.operation && (
            <div>
              Operation: {message.operation.id} · {message.operation.state}
            </div>
          )}
          {message.error && <div>{message.error}</div>}
          {message.cleanupError && (
            <Button
              size="sm"
              disabled={message.running}
              onClick={() => void run(false)}
            >
              Retry local cleanup
            </Button>
          )}
        </details>
      </div>
    </article>
  );
}
