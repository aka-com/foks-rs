/** Keyboard shortcuts written the way the platform writes them. */

/** Whether the window runs on macOS, where shortcuts use ⌘ instead of Ctrl. */
export function isMac(): boolean {
  if (typeof navigator === 'undefined') return true;
  return /Mac|iPhone|iPad/i.test(
    `${navigator.platform} ${navigator.userAgent}`,
  );
}

/**
 * A command-key shortcut: `⌘C` and `⇧⌘C` on macOS, `Ctrl C` and
 * `Ctrl Shift C` elsewhere.
 */
export function shortcutLabel(key: string, shift = false): string {
  if (isMac()) return `${shift ? '⇧' : ''}⌘${key}`;
  return `Ctrl ${shift ? 'Shift ' : ''}${key}`;
}
