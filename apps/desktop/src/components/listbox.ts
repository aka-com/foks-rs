/**
 * Keyboard and focus model for a single-select `listbox` with roving
 * `tabIndex` across `option` rows, providing a single Tab stop and arrow-key
 * navigation.
 *
 * The caller owns selection state. Up, Down, Home, End, and type-select invoke
 * `onMove` with the target row key; the caller can update selection (for item
 * rows) or leave selection unchanged while moving focus (for folder rows), and
 * the hook focuses the target row after render. The hook matches rows by
 * `data-option-key` anywhere inside the container, allowing section headers
 * and `role="group"` wrappers between options.
 *
 * PageUp and PageDown move by one viewport page (visible row count minus one)
 * and scroll the target row into view. When `columns` is greater than 1,
 * options render in a grid: Left and Right move by one card, and Up and Down
 * move by one grid row (to the vertically aligned card, or by `columns`
 * indices when the target row is unmounted).
 *
 * Interactive controls inside a row (such as hover actions) must set
 * `tabIndex={-1}` to exclude them from the Tab order; users access them via
 * pointer or the row context menu.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { FocusEvent, KeyboardEvent, RefObject } from 'react';

/** Navigation input that moved focus to a row. */
export type ListboxMove = 'arrow' | 'page' | 'home' | 'end' | 'type';

export interface ListboxOptions {
  /** Row keys in render order. */
  keys: readonly string[];
  /** Currently selected row key, or null. */
  selectedKey: string | null;
  /** Invoked when keyboard navigation moves to `key`. */
  onMove: (key: string, how: ListboxMove) => void;
  /** Returns a row's label for type-select; omit to disable type-select. */
  labelOf?: (key: string) => string;
  /**
   * Keys currently mounted in the DOM for a virtualized list. When the
   * preferred roving-tabindex row is unmounted, the first rendered row
   * receives `tabIndex={0}` so the listbox remains focusable via Tab.
   */
  renderedKeys?: readonly string[];
  /**
   * Number of columns when rendering options as a grid; omit or pass 1 for a
   * single-column list.
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

/** Returns the nearest vertically scrollable ancestor, or null. */
function scroller(element: HTMLElement | null): HTMLElement | null {
  for (let node = element; node; node = node.parentElement) {
    const overflow = getComputedStyle(node).overflowY;
    if (overflow === 'auto' || overflow === 'scroll') return node;
  }
  return null;
}

/**
 * Finds the option on the adjacent grid row in `direction` (1 for down, -1
 * for up) closest to the current option's horizontal center, or returns null
 * if no rendered option matches or layout geometry is unavailable.
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
    // Prefer the nearest row, then the horizontally closest card; measure
    // full-width section headers from their left edge.
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
  // Last focused row key when focus differs from `selectedKey` (such as a
  // folder row focused via arrow keys); resets when `selectedKey` changes.
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
    /** Moves one row in a list or one row of cards in a grid. */
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
      // Do not wrap or clamp across row boundaries at the first or last grid row.
      if (to < 0 || to >= keys.length) {
        event.preventDefault();
        return;
      }
      step(to, 'arrow');
    };
    /** Number of options in one viewport page (visible rows minus one). */
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
