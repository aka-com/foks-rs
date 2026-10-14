import { useCallback, useEffect, useState, useSyncExternalStore } from 'react';
import type { ChatChannel, ChatScope } from '../chat-contract';
import { CHAT_TEXT_BYTES } from '../chat-limits';
import { failure } from './actions';
import { useChatSendService, useChatSends } from './send-provider';

export const TEXT_LIMIT_LABEL = `${CHAT_TEXT_BYTES / 1024} KiB`;

/**
 * The channel's draft and its size against the text limit. Drafts are held
 * above the thread, keyed by channel: the thread remounts on every channel,
 * so the text of the channel being left has to be written somewhere that
 * outlives it. The application-owned send service holds them and keeps them
 * through ordinary navigation.
 *
 * Only the caller of this hook re-renders when the draft changes: the send
 * service reports a draft change to that channel's subscribers alone, not
 * through its revision.
 */
export function useChatDraft(storeId: string, channel: string) {
  const service = useChatSendService();
  const subscribe = useCallback(
    (listener: () => void) =>
      service.subscribeDraft(storeId, channel, listener),
    [service, storeId, channel],
  );
  const read = () => service.draft(storeId, channel);
  const draft = useSyncExternalStore(subscribe, read, read);
  // The store measured the text when it was stored; reading its length here
  // does not encode the draft a second time.
  const draftBytes = draft ? service.draftBytes(storeId, channel) : 0;
  return {
    draft,
    draftBytes,
    overLimit: draftBytes > CHAT_TEXT_BYTES,
    nearLimit: draftBytes > CHAT_TEXT_BYTES * 0.75,
  };
}

/**
 * The application service owns immutable submissions beyond the composer
 * mount. This hook does not read the draft, so its caller does not re-render
 * while the user types; {@link useChatDraft} reads it.
 */
export function useChatComposer(
  /** The team this conversation belongs to, as the location addresses it. */
  storeId: string,
  channel: ChatChannel,
  scope: ChatScope | null,
) {
  const { service, revision } = useChatSends();
  const [sendError, setSendError] = useState('');
  const scopeKey = scope ? JSON.stringify(scope) : '';
  useEffect(() => {
    if (scopeKey) void service.open(storeId, channel.id);
  }, [service, storeId, channel.id, scopeKey]);

  /**
   * Stores the draft while the user switches channels within the same team.
   * The service retains drafts beyond this mount, so leaving the channel
   * does not require a component-owned navigation guard.
   */
  // Another channel of the same team keeps the draft: the service's map
  // outlives the channel switch, so there is nothing to ask about.
  // Leaving chat, or switching team, also leaves the service mounted.
  const setDraft = (text: string) => {
    try {
      service.setDraft(storeId, channel.id, text);
      // Only an error being cleared re-renders the caller.
      if (sendError) setSendError('');
    } catch (error) {
      setSendError(failure(error));
    }
  };
  /** Returns whether the draft was submitted. */
  const send = (): boolean => {
    // Read when sending rather than when rendering, so that typing does not
    // re-render this hook's caller.
    const draft = service.draft(storeId, channel.id);
    if (!draft.trim()) return false;
    setSendError('');
    // An acknowledged local intent survives navigation before preparation.
    // Once the agent has prepared it, its durable operation is recovered by
    // the service and shown in the outgoing row. Until local persistence
    // answers, the service retains the submission independently of this view.
    // A channel already saving a message queues this one instead of refusing
    // it, so the composer is ready for the next message immediately.
    if (!service.canSubmit(storeId, channel.id)) return false;
    void service
      .submit(storeId, channel.id, draft)
      .catch((error) => setSendError(failure(error)));
    return true;
  };
  // Read admission from the service when rendering and sending, so typing
  // registers neither a navigation guard nor a component-owned request.
  const loadError = service.loadError(storeId, channel.id);
  return {
    setDraft,
    send,
    sendError,
    canSend: channel.writable && service.canSubmit(storeId, channel.id),
    loadError,
    retryLoad: () => {
      void service.open(storeId, channel.id, true);
    },
    /* Keep the durable identity visible if status is unavailable. */
    messages: service.messages(storeId, channel.id),
    /**
     * Whether the channel is holding as many unsent messages as it will. The
     * composer keeps the draft and says so rather than silently disabling
     * Send with no reason.
     */
    queueFull: service.queueFull(storeId, channel.id),
    cleanupError: service.cleanupError(storeId),
    service,
    revision,
  };
}
