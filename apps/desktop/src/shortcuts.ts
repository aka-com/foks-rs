/**
 * The app's keyboard shortcuts, as one registry: each entry names a command,
 * the part of the app where it applies, and the key combinations that run
 * it. The listeners match keys with `matchesShortcut`, the hints draw them
 * with `shortcutLabel`, and the shortcut sheet lists the registry, so a
 * shortcut's keys are written once. The entries for moving focus in the
 * Files list, the Trash list and the folder tree describe the keys
 * `features/files/files-keyboard.ts` maps; their tests check that each
 * listed key moves focus. The chat entries describe the keys the Chat
 * tab's own handlers take; the channel and message steps match through the
 * registry, and the composer's formatting keys and Find, which take ⌘ on
 * macOS and Ctrl elsewhere rather than either, are listed per platform.
 */

/** Where a shortcut applies, in the order the sheet lists the groups. */
export type ShortcutScope =
  | 'app'
  | 'files'
  | 'trash'
  | 'tree'
  | 'chat'
  | 'chatChannels'
  | 'chatMessages'
  | 'chatComposer'
  | 'chatMentions'
  | 'chatFind'
  | 'search'
  | 'servers';

export const SHORTCUT_SCOPES: readonly { id: ShortcutScope; label: string }[] =
  [
    { id: 'app', label: 'Anywhere' },
    { id: 'files', label: 'Files list' },
    { id: 'trash', label: 'Trash list' },
    { id: 'tree', label: 'Folder tree' },
    { id: 'chat', label: 'Chat' },
    { id: 'chatChannels', label: 'Chat channel list' },
    { id: 'chatMessages', label: 'Chat messages' },
    { id: 'chatComposer', label: 'Chat composer' },
    { id: 'chatMentions', label: 'Chat @name suggestions' },
    { id: 'chatFind', label: 'Chat find bar' },
    { id: 'search', label: 'Search palette' },
    { id: 'servers', label: 'Settings › Servers' },
  ];

/**
 * One key combination. `key` is a `KeyboardEvent.key` value, lowercase for a
 * letter. `mod` is written ⌘ on macOS and Ctrl elsewhere, and matches ⌘ or
 * Ctrl on every platform; `ctrl` and `meta` name one physical key. Every
 * other modifier the chord does not name must be up.
 */
export interface Chord {
  key: string;
  mod?: boolean;
  ctrl?: boolean;
  meta?: boolean;
  alt?: boolean;
  shift?: boolean;
  /**
   * Shift is not checked: the key is a character that some layouts type
   * with Shift and others without, such as `?`.
   */
  anyShift?: boolean;
  /** Listed on this platform only. Matching does not depend on it. */
  only?: 'mac' | 'other';
}

export interface Shortcut {
  /** What the keys do, as the sheet lists it. */
  label: string;
  scope: ShortcutScope;
  /** The combinations that run it, the first one being the hint's. */
  chords: readonly Chord[];
  /**
   * The macOS application menu carries the command with the first chord as
   * its accelerator (`src-tauri/src/lib.rs`).
   */
  menu?: true;
}

const MOVE_KEYS: readonly Chord[] = [
  { key: 'ArrowUp' },
  { key: 'ArrowDown' },
  { key: 'Home' },
  { key: 'End' },
];

const ROW_MOVE_KEYS: readonly Chord[] = [
  ...MOVE_KEYS,
  { key: 'PageUp' },
  { key: 'PageDown' },
];

const MENU_KEYS: readonly Chord[] = [
  { key: 'F10', shift: true },
  { key: 'ContextMenu' },
];

/**
 * A letter with the platform's command key: ⌘ on macOS and Ctrl elsewhere,
 * not either, as the chat composer and Find read it (`applePlatform` in
 * `chat/composer-format.ts`).
 */
function platformCommand(key: string, shift = false): readonly Chord[] {
  return [
    { key, meta: true, shift, only: 'mac' },
    { key, ctrl: true, shift, only: 'other' },
  ];
}

export const SHORTCUTS = {
  search: {
    label: 'Search everything',
    scope: 'app',
    chords: [{ key: 'k', mod: true }],
  },
  newPassword: {
    label: 'New password on a Files page',
    scope: 'app',
    chords: [{ key: 'n', mod: true }],
    menu: true,
  },
  newDocument: {
    label: 'Add a document on a Files page',
    scope: 'app',
    chords: [{ key: 'n', mod: true, shift: true }],
    menu: true,
  },
  lock: {
    label: 'Lock',
    scope: 'app',
    chords: [{ key: 'l', mod: true }],
    menu: true,
  },
  settings: {
    label: 'Settings',
    scope: 'app',
    chords: [{ key: ',', mod: true, only: 'mac' }],
    menu: true,
  },
  shortcuts: {
    label: 'Keyboard shortcuts',
    scope: 'app',
    chords: [
      { key: '/', mod: true },
      { key: '?', anyShift: true },
    ],
  },
  nextSection: {
    label: 'Next section',
    scope: 'app',
    chords: [{ key: 'Tab', ctrl: true }],
  },
  previousSection: {
    label: 'Previous section',
    scope: 'app',
    chords: [{ key: 'Tab', ctrl: true, shift: true }],
  },
  back: {
    label: 'Back',
    scope: 'app',
    chords: [
      { key: '[', meta: true, only: 'mac' },
      { key: 'ArrowLeft', alt: true },
    ],
  },
  forward: {
    label: 'Forward',
    scope: 'app',
    chords: [
      { key: ']', meta: true, only: 'mac' },
      { key: 'ArrowRight', alt: true },
    ],
  },
  dismiss: {
    label: 'Clear the search, then close the selected item',
    scope: 'app',
    chords: [{ key: 'Escape' }],
  },
  rowMove: {
    label: 'Move between rows',
    scope: 'files',
    chords: ROW_MOVE_KEYS,
  },
  rowActions: {
    label: 'Move to a row’s buttons, or between cards',
    scope: 'files',
    chords: [{ key: 'ArrowRight' }, { key: 'ArrowLeft' }],
  },
  rowOpen: {
    label: 'Show details, or open a folder',
    scope: 'files',
    chords: [{ key: 'Enter' }, { key: ' ' }],
  },
  rowCopy: {
    label: 'Copy the password or value',
    scope: 'files',
    chords: [{ key: 'c', mod: true }],
  },
  rowCopyUsername: {
    label: 'Copy the user name',
    scope: 'files',
    chords: [{ key: 'c', mod: true, shift: true }],
  },
  rowRename: {
    label: 'Rename or move',
    scope: 'files',
    chords: [{ key: 'F2' }],
  },
  rowTrash: {
    label: 'Move to Trash',
    scope: 'files',
    chords: [{ key: 'Delete' }, { key: 'Backspace', mod: true }],
  },
  undoTrash: {
    label: 'Undo a move to Trash while its notice shows',
    scope: 'files',
    chords: [{ key: 'z', mod: true }],
  },
  rowMenu: {
    label: 'Open the row menu',
    scope: 'files',
    chords: MENU_KEYS,
  },
  rowToggle: {
    label: 'Add the row to the chosen rows, or remove it',
    scope: 'files',
    chords: [
      { key: 'Enter', mod: true },
      { key: ' ', mod: true },
    ],
  },
  rowExtend: {
    label: 'Choose the rows up to this one',
    scope: 'files',
    chords: [
      { key: 'Enter', shift: true },
      { key: ' ', shift: true },
    ],
  },
  rowsAll: {
    label: 'Choose every row',
    scope: 'files',
    chords: [{ key: 'a', mod: true }],
  },
  rowsClear: {
    label: 'Clear the chosen rows',
    scope: 'files',
    chords: [{ key: 'Escape' }],
  },
  trashMove: {
    label: 'Move between rows',
    scope: 'trash',
    chords: ROW_MOVE_KEYS,
  },
  trashActions: {
    label: 'Move to a row’s Restore and Delete permanently buttons',
    scope: 'trash',
    chords: [{ key: 'ArrowRight' }, { key: 'ArrowLeft' }],
  },
  trashSelect: {
    label: 'Show where the entry is stored',
    scope: 'trash',
    chords: [{ key: 'Enter' }, { key: ' ' }],
  },
  trashDelete: {
    label: 'Delete permanently',
    scope: 'trash',
    chords: [{ key: 'Delete' }, { key: 'Backspace', mod: true }],
  },
  treeMove: {
    label: 'Move between folders',
    scope: 'tree',
    chords: MOVE_KEYS,
  },
  treeOpen: {
    label: 'Open a folder, or move into it',
    scope: 'tree',
    chords: [{ key: 'ArrowRight' }],
  },
  treeClose: {
    label: 'Close a folder, or move to its parent',
    scope: 'tree',
    chords: [{ key: 'ArrowLeft' }],
  },
  treeSelect: {
    label: 'Show the folder',
    scope: 'tree',
    chords: [{ key: 'Enter' }, { key: ' ' }],
  },
  treeMenu: {
    label: 'Open the folder menu',
    scope: 'tree',
    chords: MENU_KEYS,
  },
  chatChannelStep: {
    label: 'Previous or next channel',
    scope: 'chat',
    chords: [
      { key: 'ArrowUp', alt: true },
      { key: 'ArrowDown', alt: true },
    ],
  },
  chatUnreadStep: {
    label: 'Previous or next channel with unread messages',
    scope: 'chat',
    chords: [
      { key: 'ArrowUp', alt: true, shift: true },
      { key: 'ArrowDown', alt: true, shift: true },
    ],
  },
  chatFind: {
    label: 'Find in the conversation',
    scope: 'chat',
    chords: platformCommand('f'),
  },
  chatInfoClose: {
    label: 'Close Channel info, from inside it',
    scope: 'chat',
    chords: [{ key: 'Escape' }],
  },
  chatChannelMenu: {
    label: 'Open the channel or team menu',
    scope: 'chatChannels',
    chords: MENU_KEYS,
  },
  chatRowMove: {
    label: 'Move between messages',
    scope: 'chatMessages',
    chords: MOVE_KEYS,
  },
  chatRowMenu: {
    label: 'Open the message’s actions menu',
    scope: 'chatMessages',
    chords: [{ key: 'Enter' }, ...MENU_KEYS],
  },
  chatRowActions: {
    label: 'Move between the message’s buttons',
    scope: 'chatMessages',
    chords: [{ key: 'ArrowRight' }, { key: 'ArrowLeft' }],
  },
  chatRowLeave: {
    label: 'Move to the composer',
    scope: 'chatMessages',
    chords: [{ key: 'Escape' }],
  },
  chatSend: {
    label: 'Send',
    scope: 'chatComposer',
    chords: [{ key: 'Enter' }],
  },
  chatNewLine: {
    label: 'New line',
    scope: 'chatComposer',
    chords: [
      { key: 'Enter', shift: true },
      { key: 'Enter', alt: true },
    ],
  },
  chatBold: {
    label: 'Bold',
    scope: 'chatComposer',
    chords: platformCommand('b'),
  },
  chatItalic: {
    label: 'Italic',
    scope: 'chatComposer',
    chords: platformCommand('i'),
  },
  chatCode: {
    label: 'Code',
    scope: 'chatComposer',
    chords: platformCommand('e'),
  },
  chatCodeBlock: {
    label: 'Code block',
    scope: 'chatComposer',
    chords: platformCommand('e', true),
  },
  chatRestore: {
    label: 'Edit your unsent message, in an empty composer',
    scope: 'chatComposer',
    chords: [{ key: 'ArrowUp' }],
  },
  chatMarkRead: {
    label: 'Mark the channel read, with the composer empty',
    scope: 'chatComposer',
    chords: [{ key: 'Escape' }],
  },
  chatMentionMove: {
    label: 'Move between suggestions',
    scope: 'chatMentions',
    chords: [{ key: 'ArrowUp' }, { key: 'ArrowDown' }],
  },
  chatMentionPick: {
    label: 'Insert the @name',
    scope: 'chatMentions',
    chords: [{ key: 'Enter' }, { key: 'Tab' }],
  },
  chatMentionClose: {
    label: 'Close the suggestions',
    scope: 'chatMentions',
    chords: [{ key: 'Escape' }],
  },
  chatFindOlder: {
    label: 'Older match',
    scope: 'chatFind',
    chords: [{ key: 'Enter' }],
  },
  chatFindNewer: {
    label: 'Newer match',
    scope: 'chatFind',
    chords: [{ key: 'Enter', shift: true }],
  },
  chatFindClose: {
    label: 'Close the find bar',
    scope: 'chatFind',
    chords: [{ key: 'Escape' }],
  },
  paletteMove: {
    label: 'Move between results',
    scope: 'search',
    chords: [{ key: 'ArrowUp' }, { key: 'ArrowDown' }],
  },
  paletteOpen: {
    label: 'Open the result, or run the action',
    scope: 'search',
    chords: [{ key: 'Enter' }],
  },
  paletteKind: {
    label: 'Next kind',
    scope: 'search',
    chords: [{ key: 'Tab' }],
  },
  palettePreviousKind: {
    label: 'Previous kind',
    scope: 'search',
    chords: [{ key: 'Tab', shift: true }],
  },
  paletteCopy: {
    label: 'Copy the password or value',
    scope: 'search',
    chords: [{ key: 'c', mod: true }],
  },
  paletteCopyUsername: {
    label: 'Copy the user name',
    scope: 'search',
    chords: [{ key: 'c', mod: true, shift: true }],
  },
  paletteOpenWebsite: {
    label: "Open a login's website",
    scope: 'search',
    chords: [{ key: 'o', mod: true }],
  },
  paletteFavorite: {
    label: 'Add to Favorites, or remove',
    scope: 'search',
    chords: [{ key: 'd', mod: true }],
  },
  serverCheck: {
    label: 'Check the selected server',
    scope: 'servers',
    chords: [{ key: 'r', mod: true }],
  },
} as const satisfies Record<string, Shortcut>;

export type ShortcutId = keyof typeof SHORTCUTS;

/** Whether the window runs on macOS, where shortcuts use ⌘ instead of Ctrl. */
export function isMac(): boolean {
  if (typeof navigator === 'undefined') return true;
  return /Mac|iPhone|iPad/i.test(
    `${navigator.platform} ${navigator.userAgent}`,
  );
}

/** The parts of a key event a chord is matched against. */
export type KeyPress = Pick<
  KeyboardEvent,
  'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'
>;

/** `KeyboardEvent.key` with a letter folded to lowercase, as chords write it. */
function keyOf(key: string): string {
  return key.length === 1 ? key.toLowerCase() : key;
}

export function matchesChord(event: KeyPress, chord: Chord): boolean {
  if (keyOf(event.key) !== chord.key) return false;
  if (chord.mod) {
    if (!event.metaKey && !event.ctrlKey) return false;
  } else if (
    event.metaKey !== Boolean(chord.meta) ||
    event.ctrlKey !== Boolean(chord.ctrl)
  )
    return false;
  if (event.altKey !== Boolean(chord.alt)) return false;
  return chord.anyShift || event.shiftKey === Boolean(chord.shift);
}

/** Whether `event` is one of the combinations that run shortcut `id`. */
export function matchesShortcut(event: KeyPress, id: ShortcutId): boolean {
  return SHORTCUTS[id].chords.some((chord) => matchesChord(event, chord));
}

const MAC_KEYS: Readonly<Record<string, string>> = {
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
  Enter: '↩',
  Backspace: '⌫',
  Delete: '⌦',
  Escape: 'Esc',
  Tab: 'Tab',
  ' ': 'Space',
  ContextMenu: 'Menu',
  PageUp: 'Page Up',
  PageDown: 'Page Down',
};

const OTHER_KEYS: Readonly<Record<string, string>> = {
  ...MAC_KEYS,
  Enter: 'Enter',
  Backspace: 'Backspace',
  Delete: 'Delete',
};

/**
 * A chord written the way the platform writes it: `⇧⌘N` on macOS and
 * `Ctrl Shift N` elsewhere.
 */
export function chordLabel(chord: Chord, mac: boolean = isMac()): string {
  const key =
    (mac ? MAC_KEYS : OTHER_KEYS)[chord.key] ?? chord.key.toUpperCase();
  if (mac)
    return [
      chord.ctrl ? '⌃' : '',
      chord.alt ? '⌥' : '',
      chord.shift ? '⇧' : '',
      chord.mod || chord.meta ? '⌘' : '',
      key,
    ].join('');
  return [
    chord.mod || chord.ctrl ? 'Ctrl' : '',
    chord.meta ? 'Super' : '',
    chord.alt ? 'Alt' : '',
    chord.shift ? 'Shift' : '',
    key,
  ]
    .filter(Boolean)
    .join(' ');
}

/** The chords of shortcut `id` the platform lists. */
export function listedChords(
  id: ShortcutId,
  mac: boolean = isMac(),
): readonly Chord[] {
  return (SHORTCUTS[id].chords as readonly Chord[]).filter(
    (chord) => !chord.only || chord.only === (mac ? 'mac' : 'other'),
  );
}

/** The hint for shortcut `id`: its first listed chord, as `chordLabel` writes it. */
export function shortcutLabel(id: ShortcutId, mac: boolean = isMac()): string {
  const [chord] = listedChords(id, mac);
  return chord ? chordLabel(chord, mac) : '';
}

/**
 * The accelerator Tauri's menu builder takes for a chord, such as
 * `CmdOrCtrl+Shift+N`.
 */
export function menuAccelerator(chord: Chord): string {
  return [
    chord.mod ? 'CmdOrCtrl' : '',
    chord.ctrl ? 'Ctrl' : '',
    chord.meta ? 'Cmd' : '',
    chord.alt ? 'Alt' : '',
    chord.shift ? 'Shift' : '',
    chord.key.length === 1 ? chord.key.toUpperCase() : chord.key,
  ]
    .filter(Boolean)
    .join('+');
}

/** Whether a key press is typing into a text field, which keeps its own keys. */
export function typingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName) ||
    target.closest('[role="textbox"]') !== null
  );
}
