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
