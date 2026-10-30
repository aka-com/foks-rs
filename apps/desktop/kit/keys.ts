/**
 * Platform rules for keyboard shortcuts.
 *
 * Shortcuts use macOS notation ("⌘N", "⇧⌘N", "⌃⌘S"). On macOS, ⌘ maps to
 * Command and Control is reserved for text editing (⌃K deletes to end of line,
 * ⌃N moves down a line), so Command shortcuts ignore Control. On other
 * platforms, Control maps to Command.
 */

export function isMac(): boolean {
  if (typeof navigator === 'undefined') return true;
  return /Mac|iPhone|iPad/i.test(
    `${navigator.platform} ${navigator.userAgent}`,
  );
}

/**
 * Returns whether the event uses the platform's command modifier: Command
 * without Control on macOS, or Control or Command on other platforms.
 */
export function commandKey(event: {
  metaKey: boolean;
  ctrlKey: boolean;
}): boolean {
  return isMac()
    ? event.metaKey && !event.ctrlKey
    : event.metaKey || event.ctrlKey;
}

const NAMES: Readonly<Record<string, string>> = {
  '⌃': 'Ctrl',
  '⌥': 'Alt',
  '⇧': 'Shift',
  '⌘': 'Ctrl',
  '⌫': 'Backspace',
  '↩': 'Enter',
};

/**
 * Formats a shortcut for the current platform: returns "⇧⌘N" unchanged on
 * macOS, and "Ctrl+Shift+N" on other platforms. Off macOS, ⌃⌘ (where both
 * modifiers would map to Control) formats as "Ctrl+Alt", matching the
 * non-macOS binding.
 */
export function shortcutLabel(shortcut: string): string {
  if (isMac()) return shortcut;
  const keys = [...shortcut];
  const modifiers = new Set(keys.filter((key) => '⌃⌥⇧⌘'.includes(key)));
  const rest = keys.filter((key) => !modifiers.has(key));
  const names: string[] = [];
  if (modifiers.has('⌘') || modifiers.has('⌃')) names.push('Ctrl');
  if (modifiers.has('⌥') || (modifiers.has('⌘') && modifiers.has('⌃')))
    names.push('Alt');
  if (modifiers.has('⇧')) names.push('Shift');
  return [...names, ...rest.map((key) => NAMES[key] ?? key)].join('+');
}
