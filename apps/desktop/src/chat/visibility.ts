/** Shared focus predicate; notification visibility never advances read-through. */
export function focusedWindow(): boolean {
  return document.visibilityState !== 'hidden' && document.hasFocus();
}
export function messageVisible(
  store: string,
  channel: string,
  id: string,
): boolean {
  if (!focusedWindow()) return false;
  // Iterate attributes rather than interpolating untrusted identity into a selector.
  for (const viewport of document.querySelectorAll<HTMLElement>(
    '[data-chat-channel]',
  )) {
    if (
      viewport.dataset.chatStore !== store ||
      viewport.dataset.chatChannel !== channel
    )
      continue;
    const clip = viewport.getBoundingClientRect();
    for (const item of viewport.querySelectorAll<HTMLElement>(
      '[data-message]',
    )) {
      if (item.dataset.message !== id) continue;
      const rect = item.getBoundingClientRect();
      return (
        rect.width > 0 &&
        rect.height > 0 &&
        rect.bottom > Math.max(0, clip.top) &&
        rect.top < Math.min(window.innerHeight, clip.bottom) &&
        rect.right > Math.max(0, clip.left) &&
        rect.left < Math.min(window.innerWidth, clip.right)
      );
    }
  }
  return false;
}
