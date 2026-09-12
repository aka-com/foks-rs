import { useEffect, useMemo, useRef, useState } from 'react';
import type { ChatAction, ChatChannel, ChatReply } from '../chat-contract';
import { CHAT_TEXT_BYTES } from '../chat-contract';
import { failure, preparationCanChange, submissionId } from './actions';
export const TEXT_LIMIT_LABEL = `${CHAT_TEXT_BYTES / 1024} KiB`;

/** One mounted composer owns its immutable submission until preparation resolves. */
export function useChatComposer(
  channel: ChatChannel,
  request: (action: ChatAction) => Promise<ChatReply>,
  refreshPending: () => Promise<void>,
  load: () => Promise<void>,
) {
  const active = useRef(false);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);
  const [draft, setDraft] = useState('');
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState('');
  const submission = useRef<Extract<
    ChatAction,
    { action: 'prepare-message' }
  > | null>(null);
  const sendGuard = useRef(false);
  const draftBytes = useMemo(
    () => new TextEncoder().encode(draft).length,
    [draft],
  );
  useEffect(
    () => () => {
      submission.current = null;
    },
    [],
  );
  const send = async () => {
    if (
      !channel.writable ||
      sendGuard.current ||
      (!submission.current && !draft.trim())
    )
      return;
    if (draftBytes > CHAT_TEXT_BYTES) {
      setSendError(
        `Messages can contain up to ${TEXT_LIMIT_LABEL} of UTF-8 text.`,
      );
      return;
    }
    sendGuard.current = true;
    setSending(true);
    setSendError('');
    submission.current ??= {
      action: 'prepare-message',
      submission: submissionId(),
      channel: channel.id,
      text: draft,
    };
    let preparedId: string | null = null;
    try {
      const reply = await request(submission.current);
      if (!active.current) return;
      if (reply.result.kind !== 'operation')
        throw new Error('Invalid message preparation.');
      preparedId = reply.result.operation.id;
      submission.current = null;
      setDraft('');
      await refreshPending();
      if (!active.current) return;
      await request({
        action: 'attempt',
        operation: preparedId,
      });
      if (!active.current) return;
      await refreshPending();
      await load();
    } catch (e) {
      if (active.current) {
        if (submission.current && preparationCanChange(e))
          submission.current = null;
        setSendError(failure(e));
        if (preparedId) {
          try {
            await request({ action: 'status', operation: preparedId });
          } catch {
            /* Keep the durable identity visible if status is unavailable. */
          }
        }
        void refreshPending().catch(() => {});
      }
    } finally {
      sendGuard.current = false;
      if (active.current) setSending(false);
    }
  };
  const overLimit = draftBytes > CHAT_TEXT_BYTES;
  const nearLimit = draftBytes > CHAT_TEXT_BYTES * 0.75;
  return {
    draft,
    setDraft,
    sending,
    sendError,
    draftBytes,
    send,
    overLimit,
    nearLimit,
    recovering: submission.current !== null,
  };
}
