import { useSidebarResize } from './use-sidebar-resize';

/** A device-local Files tree width preference, intentionally absent from Settings. */
export function useFilesSidebarResize() {
  return useSidebarResize<HTMLDivElement>({
    storageKey: 'filesSidebarWidth',
    cssVariable: '--files-tree-w',
    className: 'files-sidebar-resizer',
    label: 'Files',
    defaultWidth: 232,
    compactWidth: 220,
  });
}
