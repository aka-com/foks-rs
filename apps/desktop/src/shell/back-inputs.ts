/** History inputs share app navigation; editable fields keep their own shortcuts. */
export function mountBackInputs(options: {
  enabled: () => boolean;
  back: () => void;
  forward: () => void;
  root?: Document;
}): () => void {
  const root = options.root ?? document;
  const keyDown = (event: KeyboardEvent): void => {
    if (event.defaultPrevented) return;
    const browserKey =
      event.key === 'BrowserBack' || event.key === 'BrowserForward';
    const forward =
      event.key === 'BrowserForward' ||
      event.key === 'ArrowRight' ||
      event.key === ']';
    const shortcut =
      !event.shiftKey &&
      ((event.altKey &&
        !event.ctrlKey &&
        !event.metaKey &&
        (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) ||
        (event.metaKey &&
          !event.altKey &&
          !event.ctrlKey &&
          (event.key === '[' || event.key === ']')));
    if (!browserKey && !shortcut) return;
    const target = event.target;
    if (
      !browserKey &&
      target instanceof Element &&
      (target.closest('input, textarea, [role="textbox"]') ||
        (target instanceof HTMLElement && target.isContentEditable))
    )
      return;
    // Suppress webview history even when the app has no destination or is blocked.
    event.preventDefault();
    if (!event.repeat && options.enabled())
      (forward ? options.forward : options.back)();
  };
  const mouseBack = (event: MouseEvent): void => {
    if ((event.button !== 3 && event.button !== 4) || event.defaultPrevented)
      return;
    event.preventDefault();
    if (event.type === 'mouseup' && options.enabled())
      (event.button === 4 ? options.forward : options.back)();
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
