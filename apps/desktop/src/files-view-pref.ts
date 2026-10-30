import { useSyncExternalStore } from 'react';

/** A display preference only: browsing scope and selection remain in LocationStore. */
export type FilesView = 'list' | 'grid';

export function storedFilesView(): FilesView {
  try {
    return typeof window !== 'undefined' &&
      window.localStorage.getItem('filesView') === 'grid'
      ? 'grid'
      : 'list';
  } catch {
    return 'list';
  }
}

export function rememberFilesView(view: FilesView): void {
  try {
    if (typeof window !== 'undefined')
      window.localStorage.setItem('filesView', view);
  } catch {
    // Restricted storage must not prevent changing the current view.
  }
}

const CHANGE = 'foks-files-view-change';
/** In-memory fallback used when `localStorage` is unavailable. */
let session: FilesView | undefined;

/** Persists the Files view mode and notifies subscribers. */
export function setFilesView(view: FilesView): void {
  rememberFilesView(view);
  session = storedFilesView() === view ? undefined : view;
  if (typeof window !== 'undefined') window.dispatchEvent(new Event(CHANGE));
}

function currentFilesView(): FilesView {
  return session ?? storedFilesView();
}

function subscribe(notify: () => void): () => void {
  window.addEventListener(CHANGE, notify);
  return () => window.removeEventListener(CHANGE, notify);
}

/**
 * Subscribes to the Files layout mode (`list` or `grid`) shared between the
 * topbar view menu and the Files list.
 */
export function useFilesView(): FilesView {
  return useSyncExternalStore(subscribe, currentFilesView, () => 'list');
}
