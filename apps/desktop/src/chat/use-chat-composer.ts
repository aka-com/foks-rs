import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ChatAction, ChatChannel, ChatReply } from '../chat-contract';
import { CHAT_TEXT_BYTES } from '../chat-contract';
import type { NavigationGuard } from '../location';
import { useNavigationGuard } from '../navigation-guard';
import { failure, preparationCanChange, submissionId } from './actions';
import { channelTitle } from './presentation';
import type { ChatIntentPersistence } from './intent';
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
  persistence?: ChatIntentPersistence,
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
  const [refreshError, setRefreshError] = useState('');
  const submission = useRef<Extract<
    ChatAction,
    { action: 'prepare-message' }
  > | null>(null);
  const sendGuard = useRef(false);
  const durable = useRef(false);
  const sendLifetime = useRef(0);
  const persistenceRef = useRef(persistence);
  persistenceRef.current = persistence;
  const persistenceKey = persistence?.key;
  const previousKey = useRef(persistenceKey);
  const [intentLoading, setIntentLoading] = useState(Boolean(persistence));
  const [intentLoadError, setIntentLoadError] = useState(false);
  const [restoreVersion, setRestoreVersion] = useState(0);
  useEffect(() => {
    const store = persistenceRef.current;
    let current = true;
    if (previousKey.current !== persistenceKey) {
      sendLifetime.current++;
      sendGuard.current = false;
      setSending(false);
      setRefreshError('');
      submission.current = null;
      durable.current = false;
      setDraftText('');
      draftsRef.current?.clear();
      previousKey.current = persistenceKey;
    }
    setIntentLoadError(false);
    setIntentLoading(Boolean(store));
    if (store)
      void store
        .load()
        .then((intent) => {
          if (!current) return;
          setSendError('');
          if (!intent) return;
          submission.current = {
            action: 'prepare-message',
            submission: intent.submission,
            channel: channel.id,
            text: intent.text,
          };
          durable.current = true;
          setDraftText(intent.text);
          draftsRef.current?.set(channel.id, intent.text);
        })
        .catch((error) => {
          if (current) {
            setIntentLoadError(true);
            setSendError(failure(error));
          }
        })
        .finally(() => {
          if (current) setIntentLoading(false);
        });
    return () => {
      current = false;
    };
  }, [channel.id, persistenceKey, restoreVersion]);
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
    if (sendGuard.current || intentLoading) return;
    if (intentLoadError) {
      setRestoreVersion((version) => version + 1);
      return;
    }
    if (!channel.writable || (!submission.current && !draft.trim())) return;
    const store = persistenceRef.current;
    if (!store) {
      setSendError(
        'A verified chat identity is required to save this message.',
      );
      return;
    }
    if (draftBytes > CHAT_TEXT_BYTES) {
      setSendError(
        `Messages can contain up to ${TEXT_LIMIT_LABEL} of UTF-8 text.`,
      );
      return;
    }
    const lifetime = ++sendLifetime.current;
    const current = () =>
      active.current &&
      sendLifetime.current === lifetime &&
      persistenceRef.current?.key === store.key;
    const refreshViews = (includeHistory: boolean) => {
      for (const read of includeHistory
        ? [refreshPending, load]
        : [refreshPending]) {
        void Promise.resolve()
          .then(() => (current() ? read() : undefined))
          .catch((error) => {
            if (current()) setRefreshError(failure(error));
          });
      }
    };
    sendGuard.current = true;
    setSending(true);
    setSendError('');
    setRefreshError('');
    submission.current ??= {
      action: 'prepare-message',
      submission: submissionId(),
      channel: channel.id,
      text: draft,
    };
    const input = submission.current;
    let preparedId: string | null = null;
    try {
      await store.save({ submission: input.submission, text: input.text });
      if (!current()) return;
      durable.current = true;
      const reply = await request(input);
      if (!current()) return;
      if (reply.result.kind !== 'operation')
        throw new Error('Invalid message preparation.');
      preparedId = reply.result.operation.id;
      await store.clear(input.submission);
      if (!current()) return;
      submission.current = null;
      durable.current = false;
      setDraft('');
      await request({
        action: 'attempt',
        operation: preparedId,
      });
      if (current()) refreshViews(true);
    } catch (e) {
      if (current()) {
        setSendError(failure(e));
        if (submission.current && preparationCanChange(e)) {
          try {
            await store.clear(input.submission);
            if (!current()) return;
            submission.current = null;
            durable.current = false;
          } catch (cleanup) {
            if (current()) setSendError(failure(cleanup));
          }
        }
        if (current()) refreshViews(false);
        if (preparedId && current()) {
          const operation = preparedId;
          void Promise.resolve()
            .then(() =>
              current() ? request({ action: 'status', operation }) : undefined,
            )
            .catch(() => {
              /* Keep the durable identity visible if status is unavailable. */
            });
        }
      }
    } finally {
      if (current()) {
        sendGuard.current = false;
        setSending(false);
      }
    }
  };
  // Read by the guard when it is asked, so typing re-registers nothing.
  const guardState = useRef({ draft, channel, storeId });
  guardState.current = { draft, channel, storeId };
  const draftGuard = useCallback<NavigationGuard>((intent) => {
    const { draft: text, channel: open, storeId: team } = guardState.current;
    // An acknowledged local intent survives navigation before preparation.
    // Once the agent has prepared it, its durable operation is recovered in
    // "Needs attention", where `recover-pending.ts` checks it and
    // `pending-row.tsx` sends or cancels it. Until local persistence answers,
    // keep the view mounted rather than assuming that the message was saved.
    if (sendGuard.current && !durable.current && text.trim())
      return {
        verdict: 'refuse',
        reason: 'Wait for the message to be saved on this device.',
      };
    if (durable.current || !text.trim()) return null;
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
    intentLoading,
    intentLoadError,
    sendError,
    refreshError,
    draftBytes,
    send,
    overLimit,
    nearLimit,
    recovering: submission.current !== null,
  };
}
