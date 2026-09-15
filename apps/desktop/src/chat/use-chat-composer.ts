import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ChatAction, ChatChannel, ChatReply } from '../chat-contract';
import { CHAT_TEXT_BYTES } from '../chat-contract';
import type { NavigationGuard } from '../location';
import { useNavigationGuard } from '../navigation-guard';
import { failure, preparationCanChange, submissionId } from './actions';
import { channelTitle } from './presentation';
export const TEXT_LIMIT_LABEL = `${CHAT_TEXT_BYTES / 1024} KiB`;

/**
 * Drafts held above the thread, keyed by channel. The thread remounts on every
 * channel, so the text of the channel being left has to be written somewhere
 * that outlives it; `screens/chat-screen.tsx` owns the map and is keyed on the
 * team, which is exactly how far a draft is kept.
 */
export type ChannelDrafts = Map<string, string>;

/** One mounted composer owns its immutable submission until preparation resolves. */
export function useChatComposer(
  channel: ChatChannel,
  request: (action: ChatAction) => Promise<ChatReply>,
  refreshPending: () => Promise<void>,
  load: () => Promise<void>,
  /**   * Stores the draft while the user switches channels within the same team.
   * With no map the draft belongs to this mount alone, and leaving the channel
   * at all is what the guard asks about.
   */
  drafts?: ChannelDrafts,
  /** The team this conversation belongs to, as the location addresses it. */
  storeId?: string,
) {
  const active = useRef(false);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);
  // The thread is keyed on the channel, so a mount reads the draft of its own
  // channel once and owns it from there.
  const [draft, setDraftText] = useState(() => drafts?.get(channel.id) ?? '');
  const draftsRef = useRef(drafts);
  draftsRef.current = drafts;
  const setDraft = (text: string): void => {
    setDraftText(text);
    const kept = draftsRef.current;
    if (!kept) return;
    if (text) kept.set(channel.id, text);
    else kept.delete(channel.id);
  };
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
  // Read by the guard when it is asked, so typing re-registers nothing.
  const guardState = useRef({ draft, channel, storeId });
  guardState.current = { draft, channel, storeId };
  const draftGuard = useCallback<NavigationGuard>((intent) => {
    const { draft: text, channel: open, storeId: team } = guardState.current;
    // A send in flight is not an unsent draft. The agent has been given a
    // durable preparation by then, and a navigation that lands mid-send leaves
    // it in the conversation's "Needs attention", where `recover-pending.ts`
    // checks it and `pending-row.tsx` sends or cancels it; nothing is dropped,
    // so nothing has to be refused.
    if (sendGuard.current || !text.trim()) return null;
    // Another channel of the same team keeps the draft: the map above the
    // thread outlives the channel switch, so there is nothing to ask about.
    // Leaving chat, or switching team, unmounts the map with the pane.
    if (
      draftsRef.current &&
      team !== undefined &&
      intent.kind === 'navigate' &&
      intent.location.kind === 'chat' &&
      intent.location.ref === team
    )
      return null;
    return {
      verdict: 'prompt',
      title: 'Discard message?',
      body: `Your unsent message in ${channelTitle(open)} will be lost.`,
      confirm: 'Discard',
      onConfirm: () => {
        setDraftText('');
        draftsRef.current?.delete(open.id);
      },
    };
  }, []);
  useNavigationGuard(draftGuard);
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
