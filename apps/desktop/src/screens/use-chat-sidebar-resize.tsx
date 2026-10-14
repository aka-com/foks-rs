import { STORAGE_KEYS } from '../platform/device-storage';
import { useSidebarResize } from './use-sidebar-resize';

/** A device-local display preference, intentionally absent from Settings. */
export function useChatSidebarResize() {
  return useSidebarResize({
    storageKey: STORAGE_KEYS.chatSidebarWidth,
    cssVariable: '--chat-inbox-w',
    className: 'chat-sidebar-resizer',
    label: 'Chat',
    defaultWidth: 264,
    compactWidth: 220,
  });
}
