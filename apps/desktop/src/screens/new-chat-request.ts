/**
 * Wires the window header's "New channel" button to the active Chat tab.
 *
 * Because the header renders outside the Chat tab, it delegates channel creation
 * to the active tab. When ChatTab is mounted, this opens the channel creation
 * sheet for the currently open conversation. If ChatTab is unmounted, calling
 * this function is a no-op. The request lives apart from chat-tab.tsx so the
 * header does not load the Chat tab's chunk to wire its button.
 */
let newChatRequest: (() => void) | null = null;

/** Opens the Chat tab's channel-creation sheet. The header's button calls this. */
export function requestNewChat(): void {
  newChatRequest?.();
}

/**
 * Makes `handler` the receiver of {@link requestNewChat}; the returned
 * function removes it again if it is still the receiver.
 */
export function receiveNewChat(handler: () => void): () => void {
  newChatRequest = handler;
  return () => {
    if (newChatRequest === handler) newChatRequest = null;
  };
}
