/**
 * Keyboard and focus model for a single-select list drawn as rows: a
 * `listbox` whose rows are `option`s with a roving tabindex, so the list is
 * one Tab stop and the arrow keys move between rows.
 *
 * The hook does not own the selection. Up, Down, Home, End and type-select
 * call `onMove` with the row they reach; the caller selects it (selection
 * follows focus) or only lets focus move there (a folder row), and the hook
 * focuses that row once it has rendered. Rows are found by
 * `data-option-key` anywhere under the container, so section labels and
 * `role="group"` wrappers can sit between them.
 *
 * PageUp and PageDown move a page of rows (as many as the scrolling area
 * shows, less one) and the row reached scrolls to the edge. With `columns`
 * the rows are cards in a grid: Left and Right step one card, Up and Down
 * one line of cards (the card drawn above or below, or `columns` cards
 * along when nothing is drawn there).
 *
 * Interactive content inside a row (hover actions) should carry
 * `tabIndex={-1}`: it is reached with the pointer or through the row's
 * context menu, not with Tab.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { FocusEvent, KeyboardEvent, RefObject } from 'react';

/** How the keyboard reached a row. */
export type ListboxMove = 'arrow' | 'page' | 'home' | 'end' | 'type';

export interface ListboxOptions {
  /** Every row's key, in the order the list draws them. */
  keys: readonly string[];
  /** The selected row, or null. */
  selectedKey: string | null;
  /** The keyboard reached `key`. */
  onMove: (key: string, how: ListboxMove) => void;
  /** A row's text for type-select; type-select is off without it. */
  labelOf?: (key: string) => string;
  /**
   * The rows actually in the DOM, for a windowed list. When the row that
   * should take Tab is not rendered, the first rendered row takes it so the
   * list stays reachable.
   */
  renderedKeys?: readonly string[];
  /**
   * Cards per line, for rows drawn as a grid; leave out (or 1) for a list.
   */
  columns?: number;
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
  /** Spread on the container; add an `aria-label`. */
  listProps: ListboxProps<E>;
  /** Spread on each row. */
  optionProps: (key: string) => ListboxOptionProps;
  /** Focuses `key`'s row after the next render (or now, if it is drawn). */
  focusKey: (key: string) => void;
}

/** Keys typed within this long of each other extend one type-select search. */
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

/** The nearest ancestor that scrolls vertically, or null. */
function scroller(element: HTMLElement | null): HTMLElement | null {
  for (let node = element; node; node = node.parentElement) {
    const overflow = getComputedStyle(node).overflowY;
    if (overflow === 'auto' || overflow === 'scroll') return node;
  }
  return null;
}

/**
 * The option drawn on the next line of a grid in `direction` (1 down, -1
 * up), nearest the current option's horizontal centre, or null when the
 * layout gives no answer (nothing drawn there, or no layout at all).
 */
function gridNeighbour(
  container: HTMLElement | null,
  current: HTMLElement | undefined,
  direction: 1 | -1,
): string | null {
  if (!container || !current) return null;
  const from = current.getBoundingClientRect();
  if (!from.height) return null;
  const centre = from.left + from.width / 2;
  let best: { key: string; line: number; distance: number } | null = null;
  for (const option of container.querySelectorAll<HTMLElement>(
    '[data-option-key]',
  )) {
    const rect = option.getBoundingClientRect();
    const line = direction * (rect.top - from.top);
    if (line < from.height / 2 || !option.dataset.optionKey) continue;
    // Closest line first, then the closest card on it; a full-width section
    // label measures from its own left edge.
    const distance =
      rect.width > from.width * 2
        ? Math.abs(rect.left - from.left)
        : Math.abs(rect.left + rect.width / 2 - centre);
    if (
      !best ||
      line < best.line - 1 ||
      (Math.abs(line - best.line) <= 1 && distance < best.distance)
    )
      best = { key: option.dataset.optionKey, line, distance };
  }
  return best?.key ?? null;
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
  columns = 1,
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
    const grid = columns > 1;
    /** Up or Down: the next row, or the next line of cards. */
    const line = (direction: 1 | -1): void => {
      if (index < 0) return step(0, 'arrow');
      if (!grid) return step(index + direction, 'arrow');
      const drawn = gridNeighbour(
        ref.current,
        current ? findOption(ref.current, current) : undefined,
        direction,
      );
      const to =
        drawn === null ? index + direction * columns : keys.indexOf(drawn);
      // Down from the last line stays put rather than wrapping.
      if (to < 0 || to >= keys.length) {
        event.preventDefault();
        return;
      }
      step(to, 'arrow');
    };
    /** Rows (or cards) in one page of the scrolling area, less one line. */
    const page = (): number => {
      const option = current ? findOption(ref.current, current) : undefined;
      const height = option?.getBoundingClientRect().height ?? 0;
      const area = scroller(ref.current)?.clientHeight ?? 0;
      const lines = height && area ? Math.floor(area / height) - 1 : 9;
      return Math.max(1, lines) * columns;
    };
    switch (event.key) {
      case 'ArrowDown':
        line(1);
        return;
      case 'ArrowUp':
        line(-1);
        return;
      case 'ArrowRight':
        if (grid) step(index < 0 ? 0 : index + 1, 'arrow');
        return;
      case 'ArrowLeft':
        if (grid) step(index < 0 ? 0 : index - 1, 'arrow');
        return;
      case 'PageDown':
        step(index < 0 ? 0 : index + page(), 'page');
        return;
      case 'PageUp':
        step(index < 0 ? 0 : index - page(), 'page');
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
    // A space only extends a search already under way; alone it activates.
    if (event.key === ' ' && !continuing) return;
    const text = (continuing ? typed.current.text : '') + event.key;
    typed.current = { text, at: now };
    const needle = text.toLocaleLowerCase();
    // A fresh search starts after the current row so repeating a letter
    // walks the rows that start with it; a longer one may stay put.
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
