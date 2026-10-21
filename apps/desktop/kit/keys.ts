/**
 * Platform rules for key equivalents.
 *
 * The app states its shortcuts in macOS notation ("⌘N", "⇧⌘N", "⌃⌘S"). On a
 * Mac, ⌘ is the Command key and Control is left to text editing (⌃K kills
 * to the end of the line, ⌃N moves down a line), so a Command shortcut must
 * not also answer Control. Elsewhere the Control key stands in for Command.
 */

export function isMac(): boolean {
  if (typeof navigator === 'undefined') return true;
  return /Mac|iPhone|iPad/i.test(
    `${navigator.platform} ${navigator.userAgent}`,
  );
}

/**
 * Whether the event carries the platform's command modifier: Command alone
 * on a Mac (not Control), Control or Command elsewhere.
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
 * A shortcut in the notation of the platform it runs on: "⇧⌘N" as is on a
 * Mac, "Ctrl+Shift+N" elsewhere. Off a Mac, ⌃⌘ (two modifiers that are both
 * Control there) reads Ctrl+Alt, the binding `commandKey` callers accept for
 * it.
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
