import { useEffect, useState } from 'react';
import type { ChatChannel, ChatScope } from '../chat-contract';
import { CHAT_TEXT_BYTES } from '../chat-limits';
import { failure } from './actions';
import { useChatSends } from './send-provider';

export const TEXT_LIMIT_LABEL = `${CHAT_TEXT_BYTES / 1024} KiB`;

/**
 * Drafts held above the thread, keyed by channel. The thread remounts on every
 * channel, so the text of the channel being left has to be written somewhere
 * that outlives it; the application-owned send service owns the map for each
 * team and keeps it through ordinary navigation.
 */
export type ChannelDrafts = Map<string, string>;

/** The application service owns immutable submissions beyond the composer mount. */
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
  // The thread is keyed on the channel, but reads the current draft from its
  // application owner instead of retaining a separate copy in this mount.
  const draft = service.draft(storeId, channel.id);
  const draftBytes = new TextEncoder().encode(draft).length;
  // Another channel of the same team keeps the draft: the service's map
  // outlives the channel switch, so there is nothing to ask about.
  // Leaving chat, or switching team, also leaves the service mounted.
  const setDraft = (text: string) => {
    try {
      service.setDraft(storeId, channel.id, text);
      setSendError('');
    } catch (error) {
      setSendError(failure(error));
    }
  };
  const send = () => {
    if (!draft.trim()) return;
    setSendError('');
    // An acknowledged local intent survives navigation before preparation.
    // Once the agent has prepared it, its durable operation is recovered by
    // the service and shown in the outgoing row. Until local persistence
    // answers, the service retains the submission independently of this view.
    // A channel already saving a message queues this one instead of refusing
    // it, so the composer is ready for the next message immediately.
    if (!service.canSubmit(storeId, channel.id)) return;
    void service
      .submit(storeId, channel.id, draft)
      .catch((error) => setSendError(failure(error)));
  };
  // Read admission from the service when rendering and sending, so typing
  // registers neither a navigation guard nor a component-owned request.
  const loadError = service.loadError(storeId, channel.id);
  return {
    draft,
    draftBytes,
    setDraft,
    send,
    sendError,
    overLimit: draftBytes > CHAT_TEXT_BYTES,
    nearLimit: draftBytes > CHAT_TEXT_BYTES * 0.75,
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
