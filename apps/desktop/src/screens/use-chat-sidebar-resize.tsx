import { useSidebarResize } from './use-sidebar-resize';

/** A device-local display preference, intentionally absent from Settings. */
export function useChatSidebarResize() {
  return useSidebarResize({
    storageKey: 'chatSidebarWidth',
    cssVariable: '--chat-inbox-w',
    className: 'chat-sidebar-resizer',
    label: 'Chat',
    defaultWidth: 264,
    compactWidth: 220,
  });
}
