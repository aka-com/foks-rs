import { useState } from 'react';
import { Button } from '../components';
import { useChatConversationContext } from './conversation-provider';
import { MessageRow } from './message-row';
import { useChatSendService } from './send-provider';
import {
  cleanupErrorOf,
  operationOf,
  running,
  type OutgoingMessage,
} from './send/outgoing';
import { MessageText } from './message-text';
import { failure } from './actions';
import { messageTime, relativeMessageTime } from './presentation';
import {
  deliveryExplanation,
  outgoingDelivery,
  rejectionReason,
} from './delivery';
import { DeliveryStatus, operationDetails } from './pending-row';

/**
 * A message this session sent or is sending, drawn as the reader's own
 * message with its delivery state (`outgoingDelivery`) after the time and,
 * below the text, what happened and the actions the state allows.
 */
export function OutgoingRow({
  rowKey,
  message,
  grouped = false,
  avatarName,
  place,
}: {
  /** The row's key in the list's toolbar tab stop. */
  rowKey?: string;
  message: OutgoingMessage;
  grouped?: boolean;
  avatarName: string;
  /** The channel's title, which a rejection reason names. */
  place?: string;
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
  const state = outgoingDelivery(message);
  const op = operationOf(message);
  const busy = running(message);
  const cleanupError = cleanupErrorOf(message);
  const sent = state === null;
  const sending = state === 'sending';
  // Waiting behind another message of this channel, and held in memory only.
  const queued = state === 'queued';
  // A rejection is stated as its reason; otherwise a failed request's error
  // says what happened, and the state's explanation stands in without one.
  const line =
    state === 'rejected' && op
      ? rejectionReason(op.rejection_code, op.kind, place)
      : message.error ||
        (state === null || sending || queued ? '' : deliveryExplanation(state));
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
      // The message is not in history yet, so it has no sequence or server
      // time to detail; Details lists its operation once the agent prepared
      // one. Quoting your own unsent text is not offered.
      actions={{
        rowKey,
        text: message.text,
        delivery: op && !sent ? operationDetails(op) : undefined,
      }}
      status={
        sending ? (
          <DeliveryStatus state={state} />
        ) : (
          <>
            <DeliveryStatus state={state} kind={op?.kind} />
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
        !sending && (line || actions.retry || cleanupError || error) ? (
          <div className="chat-send-status">
            {line && <span>{line}</span>}
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
