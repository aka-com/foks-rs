/** Explicit Back inputs share app history; editable fields keep their own shortcuts. */
export function mountBackInputs(options: {
  enabled: () => boolean;
  back: () => void;
  root?: Document;
}): () => void {
  const root = options.root ?? document;
  const keyDown = (event: KeyboardEvent): void => {
    if (event.defaultPrevented) return;
    const browserBack = event.key === 'BrowserBack';
    const shortcut =
      !event.shiftKey &&
      ((event.altKey &&
        !event.ctrlKey &&
        !event.metaKey &&
        event.key === 'ArrowLeft') ||
        (event.metaKey &&
          !event.altKey &&
          !event.ctrlKey &&
          event.key === '['));
    if (!browserBack && !shortcut) return;
    const target = event.target;
    if (
      !browserBack &&
      target instanceof Element &&
      (target.closest('input, textarea, [role="textbox"]') ||
        (target instanceof HTMLElement && target.isContentEditable))
    )
      return;
    // Suppress webview history even when the app has no destination or is blocked.
    event.preventDefault();
    if (!event.repeat && options.enabled()) options.back();
  };
  const mouseBack = (event: MouseEvent): void => {
    if (event.button !== 3 || event.defaultPrevented) return;
    event.preventDefault();
    if (event.type === 'mouseup' && options.enabled()) options.back();
  };
  root.addEventListener('keydown', keyDown);
  root.addEventListener('mousedown', mouseBack);
  root.addEventListener('mouseup', mouseBack);
  root.addEventListener('auxclick', mouseBack);
  return () => {
    root.removeEventListener('keydown', keyDown);
    root.removeEventListener('mousedown', mouseBack);
    root.removeEventListener('mouseup', mouseBack);
    root.removeEventListener('auxclick', mouseBack);
  };
}
