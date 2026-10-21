import { useColumnResize } from './use-column-resize';

/** A device-local display preference, intentionally absent from Settings. */
export function useChatSidebarResize() {
  return useColumnResize({
    storageKey: 'chatSidebarWidth',
    cssVariable: '--chat-inbox-w',
    className: 'chat-sidebar-resizer',
    label: 'Chat',
    defaultWidth: 300,
    compactWidth: 260,
  });
}
