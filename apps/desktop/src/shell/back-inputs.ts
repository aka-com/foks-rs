import { matchesShortcut, typingTarget } from '../shortcuts';

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
    const forwardKey = matchesShortcut(event, 'forward');
    const shortcut = forwardKey || matchesShortcut(event, 'back');
    if (!browserKey && !shortcut) return;
    if (!browserKey && typingTarget(event.target)) return;
    const forward = event.key === 'BrowserForward' || forwardKey;
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
