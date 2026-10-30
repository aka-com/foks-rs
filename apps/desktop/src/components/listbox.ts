/**
 * Keyboard and focus model for a single-select `listbox` with roving
 * `tabIndex` across `option` rows, providing a single Tab stop and arrow-key
 * navigation.
 *
 * The hook does not own the selection. Up, Down, Home, End and type-select
 * call `onMove` with the row they reach; the caller selects it (selection
 * follows focus) or only lets focus move there (a folder row), and the hook
 * focuses that row once it has rendered. Rows are found by
 * `data-option-key` anywhere under the container, so section labels and
 * `role="group"` wrappers can sit between them.
 *
 * Interactive content inside a row (hover actions) should carry
 * `tabIndex={-1}`: it is reached with the pointer or through the row's
 * context menu, not with Tab.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { FocusEvent, KeyboardEvent, RefObject } from 'react';

/** How the keyboard reached a row. */
export type ListboxMove = 'arrow' | 'home' | 'end' | 'type';

export interface ListboxOptions {
  /** Every row's key, in the order the list draws them. */
  keys: readonly string[];
  /** Currently selected row key, or null. */
  selectedKey: string | null;
  /** Invoked when keyboard navigation moves to `key`. */
  onMove: (key: string, how: ListboxMove) => void;
  /** Returns a row's label for type-select; omit to disable type-select. */
  labelOf?: (key: string) => string;
  /**
   * The rows actually in the DOM, for a windowed list. When the row that
   * should take Tab is not rendered, the first rendered row takes it so the
   * list stays reachable.
   */
  renderedKeys?: readonly string[];
}

export interface ListboxProps<E extends HTMLElement> {
  ref: RefObject<E | null>;
  role: 'listbox';
  onKeyDown: (event: KeyboardEvent<E>) => void;
  onFocus: (event: FocusEvent<E>) => void;
}

export interface ListboxOptionProps {
  role: 'option';
  'aria-selected': boolean;
  tabIndex: 0 | -1;
  'data-option-key': string;
}

export interface Listbox<E extends HTMLElement> {
  /** Props for the listbox container; the caller must also supply `aria-label`. */
  listProps: ListboxProps<E>;
  /** Returns option props for the row with `key`. */
  optionProps: (key: string) => ListboxOptionProps;
  /** Focuses the row for `key` immediately if mounted, or on the next render. */
  focusKey: (key: string) => void;
}

/** Maximum interval in milliseconds between keystrokes in a single type-select query. */
const TYPE_SELECT_MS = 700;

function optionKey(
  container: HTMLElement | null,
  target: EventTarget | null,
): string | null {
  if (!container || !(target instanceof Element)) return null;
  const option = target.closest<HTMLElement>('[data-option-key]');
  if (!option || !container.contains(option)) return null;
  return option.dataset.optionKey ?? null;
}

function findOption(
  container: HTMLElement | null,
  key: string,
): HTMLElement | undefined {
  return [
    ...(container?.querySelectorAll<HTMLElement>('[data-option-key]') ?? []),
  ].find((candidate) => candidate.dataset.optionKey === key);
}

export function useListbox<E extends HTMLElement = HTMLDivElement>({
  keys,
  selectedKey,
  onMove,
  labelOf,
  renderedKeys,
}: ListboxOptions): Listbox<E> {
  const ref = useRef<E | null>(null);
  // The row focus last rested on, when it is not the selected one (a folder
  // row reached with the arrows). A new selection takes precedence.
  const [active, setActive] = useState<string | null>(null);
  useEffect(() => setActive(null), [selectedKey]);
  const pending = useRef<string | null>(null);
  const typed = useRef({ text: '', at: 0 });

  useEffect(() => {
    const key = pending.current;
    if (!key) return;
    if (!keys.includes(key)) {
      pending.current = null;
      return;
    }
    const option = findOption(ref.current, key);
    if (!option) return;
    pending.current = null;
    option.focus({ preventScroll: true });
    option.scrollIntoView?.({ block: 'nearest' });
  });

  const focusKey = useCallback((key: string) => {
    setActive(key);
    const option = findOption(ref.current, key);
    if (option && option !== document.activeElement) {
      option.focus({ preventScroll: true });
      option.scrollIntoView?.({ block: 'nearest' });
    } else if (!option) pending.current = key;
  }, []);

  const preferred =
    active && keys.includes(active)
      ? active
      : selectedKey && keys.includes(selectedKey)
        ? selectedKey
        : (keys[0] ?? null);
  const tabStop =
    renderedKeys && preferred && !renderedKeys.includes(preferred)
      ? (renderedKeys[0] ?? null)
      : preferred;

  const move = (key: string, how: ListboxMove): void => {
    pending.current = key;
    setActive(key);
    onMove(key, how);
  };

  const onKeyDown = (event: KeyboardEvent<E>): void => {
    if (event.defaultPrevented || event.altKey || event.metaKey) return;
    if (
      event.target instanceof Element &&
      event.target.closest('input, textarea, select, [contenteditable="true"]')
    )
      return;
    if (!keys.length) return;
    const current =
      optionKey(ref.current, event.target) ?? active ?? selectedKey;
    const index = current ? keys.indexOf(current) : -1;
    const step = (to: number, how: ListboxMove): void => {
      const key = keys[Math.min(Math.max(to, 0), keys.length - 1)];
      if (key === undefined) return;
      event.preventDefault();
      move(key, how);
    };
    if (event.ctrlKey) return;
    switch (event.key) {
      case 'ArrowDown':
        step(index < 0 ? 0 : index + 1, 'arrow');
        return;
      case 'ArrowUp':
        step(index < 0 ? 0 : index - 1, 'arrow');
        return;
      case 'Home':
        step(0, 'home');
        return;
      case 'End':
        step(keys.length - 1, 'end');
        return;
    }
    if (!labelOf || event.key.length !== 1) return;
    const now = Date.now();
    const continuing = now - typed.current.at < TYPE_SELECT_MS;
    // Space only extends an active type-select query; otherwise it activates the focused row.
    if (event.key === ' ' && !continuing) return;
    const text = (continuing ? typed.current.text : '') + event.key;
    typed.current = { text, at: now };
    const needle = text.toLocaleLowerCase();
    // Single-character queries start after the current row so repeated presses
    // cycle through matching rows; multi-character queries include the current row.
    const start = index < 0 ? 0 : text.length === 1 ? index + 1 : index;
    for (let offset = 0; offset < keys.length; offset += 1) {
      const key = keys[(start + offset) % keys.length];
      if (key === undefined) continue;
      if (labelOf(key).toLocaleLowerCase().startsWith(needle)) {
        event.preventDefault();
        if (key !== current) move(key, 'type');
        return;
      }
    }
  };

  const onFocus = (event: FocusEvent<E>): void => {
    const key = optionKey(ref.current, event.target);
    if (key !== null && key !== active) setActive(key);
  };

  const optionProps = (key: string): ListboxOptionProps => ({
    role: 'option',
    'aria-selected': key === selectedKey,
    tabIndex: key === tabStop ? 0 : -1,
    'data-option-key': key,
  });

  return {
    listProps: { ref, role: 'listbox', onKeyDown, onFocus },
    optionProps,
    focusKey,
  };
}
