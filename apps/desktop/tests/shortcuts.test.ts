/**
 * The shortcut registry: how a chord matches a key press, how it is written
 * on each platform, and that the keys it lists for moving focus and for the
 * macOS menu are the keys those handlers actually take.
 */

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  SHORTCUTS,
  SHORTCUT_SCOPES,
  chordLabel,
  listedChords,
  matchesShortcut,
  menuAccelerator,
  shortcutLabel,
  type Chord,
  type KeyPress,
  type ShortcutId,
} from '../src/shortcuts';
import {
  gridFocusTarget,
  treeEntries,
  treeKeyAction,
} from '../src/features/files/files-keyboard';
import type { FolderNode } from '../src/model/scope';

function press(key: string, modifiers: Partial<KeyPress> = {}): KeyPress {
  return {
    key,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    ...modifiers,
  };
}

const IDS = Object.keys(SHORTCUTS) as ShortcutId[];

test('a command chord takes ⌘ or Ctrl on every platform, in either letter case', () => {
  assert.ok(matchesShortcut(press('k', { metaKey: true }), 'search'));
  assert.ok(matchesShortcut(press('K', { ctrlKey: true }), 'search'));
  assert.ok(!matchesShortcut(press('k'), 'search'));
  assert.ok(
    !matchesShortcut(press('k', { metaKey: true, altKey: true }), 'search'),
  );
  // Shift is part of the chord: ⇧⌘N is Add a document, not New password.
  const shifted = press('N', { metaKey: true, shiftKey: true });
  assert.ok(matchesShortcut(shifted, 'newDocument'));
  assert.ok(!matchesShortcut(shifted, 'newPassword'));
  assert.ok(matchesShortcut(press('n', { ctrlKey: true }), 'newPassword'));
});

test('a physical modifier matches only that key', () => {
  assert.ok(matchesShortcut(press('Tab', { ctrlKey: true }), 'nextSection'));
  assert.ok(!matchesShortcut(press('Tab', { metaKey: true }), 'nextSection'));
  assert.ok(
    matchesShortcut(
      press('Tab', { ctrlKey: true, shiftKey: true }),
      'previousSection',
    ),
  );
  assert.ok(matchesShortcut(press('[', { metaKey: true }), 'back'));
  assert.ok(!matchesShortcut(press('[', { ctrlKey: true }), 'back'));
  assert.ok(
    !matchesShortcut(press('[', { metaKey: true, shiftKey: true }), 'back'),
  );
  assert.ok(matchesShortcut(press('ArrowRight', { altKey: true }), 'forward'));
  assert.ok(!matchesShortcut(press('ArrowRight'), 'forward'));
});

test('? opens the sheet whether or not the layout types it with Shift', () => {
  assert.ok(matchesShortcut(press('?', { shiftKey: true }), 'shortcuts'));
  assert.ok(matchesShortcut(press('?'), 'shortcuts'));
  assert.ok(matchesShortcut(press('/', { ctrlKey: true }), 'shortcuts'));
  assert.ok(!matchesShortcut(press('/'), 'shortcuts'));
});

test('chords are written the way each platform writes them', () => {
  assert.equal(shortcutLabel('newDocument', true), '⇧⌘N');
  assert.equal(shortcutLabel('newDocument', false), 'Ctrl Shift N');
  assert.equal(shortcutLabel('nextSection', true), '⌃Tab');
  assert.equal(shortcutLabel('rowTrash', true), '⌦');
  assert.equal(chordLabel(SHORTCUTS.rowTrash.chords[1], true), '⌘⌫');
  assert.equal(
    chordLabel(SHORTCUTS.rowTrash.chords[1], false),
    'Ctrl Backspace',
  );
  assert.equal(shortcutLabel('rowMenu', false), 'Shift F10');
  assert.equal(shortcutLabel('rowOpen', false), 'Enter');
  // ⌘[ is listed on macOS only, where Alt+Left follows it.
  assert.deepEqual(
    listedChords('back', true).map((chord) => chordLabel(chord, true)),
    ['⌘[', '⌥←'],
  );
  assert.deepEqual(
    listedChords('back', false).map((chord) => chordLabel(chord, false)),
    ['Alt ←'],
  );
  // Settings has only the macOS menu's ⌘, and no hint elsewhere.
  assert.equal(shortcutLabel('settings', false), '');
});

test('every shortcut belongs to a listed scope and has a chord to list', () => {
  const scopes = new Set(SHORTCUT_SCOPES.map((scope) => scope.id));
  for (const id of IDS) {
    assert.ok(scopes.has(SHORTCUTS[id].scope), id);
    assert.ok(listedChords(id, true).length > 0, id);
  }
});

test('the macOS menu carries each menu shortcut with the registry’s accelerator', () => {
  const source = readFileSync(
    new URL('../src-tauri/src/lib.rs', import.meta.url),
    'utf8',
  );
  const native = [...source.matchAll(/\.accelerator\("([^"]+)"\)/g)]
    .map((match) => match[1])
    .sort();
  const registry = IDS.flatMap((id) => {
    const shortcut: { menu?: true; chords: readonly unknown[] } = SHORTCUTS[id];
    return shortcut.menu ? [menuAccelerator(SHORTCUTS[id].chords[0])] : [];
  }).sort();
  assert.deepEqual(native, registry);
  assert.deepEqual(registry, [
    'CmdOrCtrl+,',
    'CmdOrCtrl+L',
    'CmdOrCtrl+N',
    'CmdOrCtrl+Shift+N',
  ]);
});

test('each key listed for moving through the Files list moves focus', () => {
  const list = { index: 5, count: 20, columns: 1, page: 4 };
  for (const chord of SHORTCUTS.rowMove.chords)
    assert.notEqual(gridFocusTarget(chord.key, list), null, chord.key);
  // Left and Right move between cards; in the list they reach the row's
  // buttons, which the screen handles.
  const cards = { ...list, columns: 4 };
  for (const chord of SHORTCUTS.rowActions.chords)
    assert.notEqual(gridFocusTarget(chord.key, cards), null, chord.key);
});

test('each key listed for moving through the Trash list moves focus', () => {
  // The Trash list is one column; its Right and Left reach the row's
  // buttons, which the Trash view handles and its render test walks.
  const list = { index: 5, count: 20, columns: 1, page: 4 };
  for (const chord of SHORTCUTS.trashMove.chords)
    assert.notEqual(gridFocusTarget(chord.key, list), null, chord.key);
  for (const chord of SHORTCUTS.trashActions.chords)
    assert.equal(gridFocusTarget(chord.key, list), null, chord.key);
});

test('a key that runs a command takes no modifier the chord does not name', () => {
  assert.ok(matchesShortcut(press('Delete'), 'trashDelete'));
  assert.ok(
    matchesShortcut(press('Backspace', { metaKey: true }), 'trashDelete'),
  );
  assert.ok(
    !matchesShortcut(press('Delete', { shiftKey: true }), 'trashDelete'),
  );
  assert.ok(!matchesShortcut(press('Backspace'), 'trashDelete'));
  assert.ok(matchesShortcut(press('F10', { shiftKey: true }), 'treeMenu'));
  assert.ok(!matchesShortcut(press('F10'), 'treeMenu'));
  assert.ok(matchesShortcut(press('ArrowDown'), 'paletteMove'));
  assert.ok(
    !matchesShortcut(press('ArrowDown', { shiftKey: true }), 'paletteMove'),
  );
});

test('each key listed for the folder tree acts on it', () => {
  const node = (path: string, folders: FolderNode[] = []): FolderNode => ({
    name: path.split('/').pop() ?? '',
    path,
    folders,
    items: [],
    count: 0,
  });
  const root = node('/', [node('/a', [node('/a/b')]), node('/c')]);
  const entries = treeEntries(
    [{ storeId: 's', root, trash: false }],
    new Set(),
    '/.trash',
  );
  const at = entries.findIndex((entry) => entry.path === '/a');
  const ids: ShortcutId[] = ['treeMove', 'treeOpen', 'treeClose', 'treeSelect'];
  for (const id of ids)
    for (const chord of SHORTCUTS[id].chords)
      assert.notEqual(treeKeyAction(chord.key, entries, at), null, chord.key);
});

/** Key presses that type `chord`: a `mod` chord with ⌘ and with Ctrl. */
function pressesOf(chord: Chord): KeyPress[] {
  const base = {
    key: chord.key,
    altKey: Boolean(chord.alt),
    shiftKey: Boolean(chord.shift),
  };
  if (chord.mod)
    return [
      { ...base, metaKey: true, ctrlKey: false },
      { ...base, metaKey: false, ctrlKey: true },
    ];
  return [
    { ...base, metaKey: Boolean(chord.meta), ctrlKey: Boolean(chord.ctrl) },
  ];
}

test('no key press runs two shortcuts of one scope', () => {
  for (const { id: scope } of SHORTCUT_SCOPES) {
    const ids = IDS.filter((id) => SHORTCUTS[id].scope === scope);
    for (const id of ids)
      for (const chord of SHORTCUTS[id].chords as readonly Chord[])
        for (const event of pressesOf(chord))
          assert.deepEqual(
            ids.filter((other) => matchesShortcut(event, other)),
            [id],
            `${scope}: ${chordLabel(chord, false)}`,
          );
  }
});

test('a chat key runs no shortcut of the whole app but Escape, which chat takes first', () => {
  const app = IDS.filter((id) => SHORTCUTS[id].scope === 'app');
  for (const id of IDS.filter((id) => SHORTCUTS[id].scope.startsWith('chat')))
    for (const chord of SHORTCUTS[id].chords as readonly Chord[])
      for (const event of pressesOf(chord))
        assert.deepEqual(
          app.filter((other) => matchesShortcut(event, other)),
          event.key === 'Escape' ? ['dismiss'] : [],
          `${id}: ${chordLabel(chord, false)}`,
        );
});

test('the chat keys are written in each platform’s notation', () => {
  assert.deepEqual(
    listedChords('chatChannelStep', true).map((chord) =>
      chordLabel(chord, true),
    ),
    ['⌥↑', '⌥↓'],
  );
  assert.equal(shortcutLabel('chatUnreadStep', false), 'Alt Shift ↑');
  // The composer's formatting keys and Find take ⌘ on macOS and Ctrl
  // elsewhere, and each platform lists only its own.
  assert.equal(shortcutLabel('chatBold', true), '⌘B');
  assert.equal(shortcutLabel('chatBold', false), 'Ctrl B');
  assert.equal(shortcutLabel('chatCodeBlock', true), '⇧⌘E');
  assert.equal(shortcutLabel('chatCodeBlock', false), 'Ctrl Shift E');
  assert.equal(listedChords('chatFind', false).length, 1);
  assert.equal(shortcutLabel('chatFind', false), 'Ctrl F');
  assert.equal(shortcutLabel('chatRowMenu', true), '↩');
  assert.deepEqual(
    listedChords('chatRowMenu', false).map((chord) => chordLabel(chord, false)),
    ['Enter', 'Shift F10', 'Menu'],
  );
});

test('the chat channel and message steps take only the keys they list', () => {
  const up = press('ArrowUp', { altKey: true });
  assert.ok(matchesShortcut(up, 'chatChannelStep'));
  assert.ok(!matchesShortcut(up, 'chatUnreadStep'));
  const unread = press('ArrowDown', { altKey: true, shiftKey: true });
  assert.ok(matchesShortcut(unread, 'chatUnreadStep'));
  assert.ok(!matchesShortcut(unread, 'chatChannelStep'));
  for (const modifiers of [{ ctrlKey: true }, { metaKey: true }]) {
    const chord = press('ArrowUp', { altKey: true, ...modifiers });
    assert.ok(!matchesShortcut(chord, 'chatChannelStep'));
    assert.ok(!matchesShortcut(chord, 'chatUnreadStep'));
  }
  // Alt+Left and Alt+Right stay Back and Forward.
  assert.ok(
    !matchesShortcut(press('ArrowLeft', { altKey: true }), 'chatChannelStep'),
  );
  for (const key of ['ArrowUp', 'ArrowDown', 'Home', 'End'])
    assert.ok(matchesShortcut(press(key), 'chatRowMove'), key);
  assert.ok(!matchesShortcut(press('Home', { shiftKey: true }), 'chatRowMove'));
  assert.ok(matchesShortcut(press('Escape'), 'chatRowLeave'));
  assert.ok(
    !matchesShortcut(press('Escape', { altKey: true }), 'chatRowLeave'),
  );
});
