import {
  readStorage,
  STORAGE_KEYS,
  writeStorage,
} from './platform/device-storage';

/** A display preference only: browsing scope and selection remain in LocationStore. */
export type FilesView = 'list' | 'grid';

export function storedFilesView(): FilesView {
  return readStorage(STORAGE_KEYS.filesView) === 'grid' ? 'grid' : 'list';
}

export function rememberFilesView(view: FilesView): void {
  // Restricted storage must not prevent changing the current view.
  writeStorage(STORAGE_KEYS.filesView, view);
}
