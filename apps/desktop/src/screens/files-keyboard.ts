/**
 * Keyboard movement for the Files list, its card grid and the folder tree.
 *
 * Each of the three is one tab stop: a single entry carries tabIndex 0 and
 * the arrow keys move it (a roving tabindex). These functions map a key to
 * the entry that takes focus next and leave focusing and scrolling to the
 * screen, so the movement rules are testable without a DOM.
 */

import type { FolderNode } from '../model/scope';

export interface GridPosition {
  /** The focused entry's position in list order. */
  index: number;
  count: number;
  /** Entries per line: 1 in the list view, cards per row in the grid view. */
  columns: number;
  /** Lines one viewport holds, the distance PageUp and PageDown move. */
  page: number;
}

/**
 * The entry that `key` moves focus to, or null when the key does not move
 * focus. Left and Right move between cards only when a line holds more than
 * one entry; in the list view they belong to the row's own actions.
 */
export function gridFocusTarget(
  key: string,
  { index, count, columns, page }: GridPosition,
): number | null {
  if (count <= 0) return null;
  const last = count - 1;
  const step = Math.max(1, Math.floor(columns));
  const pageStep = step * Math.max(1, Math.floor(page));
  const clamp = (target: number): number => Math.min(Math.max(target, 0), last);
  switch (key) {
    case 'ArrowDown':
      if (index + step <= last) return index + step;
      // The last line can be short: Down from above its end lands on the
      // final entry rather than going nowhere.
      return Math.floor(index / step) < Math.floor(last / step) ? last : null;
    case 'ArrowUp':
      return index - step >= 0 ? index - step : null;
    case 'ArrowRight':
      return step > 1 && index < last ? index + 1 : null;
    case 'ArrowLeft':
      return step > 1 && index > 0 ? index - 1 : null;
    case 'Home':
      return 0;
    case 'End':
      return last;
    case 'PageDown':
      return clamp(index + pageStep);
    case 'PageUp':
      return clamp(index - pageStep);
    default:
      return null;
  }
}

/**
 * The scroll offset that brings a line spanning `top`..`top + height` (in
 * the scroller's content coordinates) into view, or `scrollTop` unchanged
 * when it is already visible. `stickyTop` is the height of a header that
 * stays pinned over the top of the viewport.
 */
export function revealOffset({
  top,
  height,
  scrollTop,
  viewport,
  stickyTop = 0,
}: {
  top: number;
  height: number;
  scrollTop: number;
  viewport: number;
  stickyTop?: number;
}): number {
  if (top - stickyTop < scrollTop) return Math.max(0, top - stickyTop);
  if (top + height > scrollTop + viewport)
    return Math.max(0, top + height - viewport);
  return scrollTop;
}

/** The first of `preferred` that names one of `keys`, else the first key. */
export function rovingKey(
  keys: readonly string[],
  ...preferred: (string | null | undefined)[]
): string | undefined {
  for (const key of preferred)
    if (key !== null && key !== undefined && keys.includes(key)) return key;
  return keys[0];
}

/** One drawn row of the folder tree, in drawing order. */
export interface TreeEntry {
  /** `store|path`, the key the tree's fold state uses. */
  key: string;
  storeId: string;
  path: string;
  /** 1 for a store, one more for each folder below it. */
  level: number;
  expandable: boolean;
  open: boolean;
}

/**
 * The rows a set of store trees draws, in order: each store, then while it is
 * open its folders depth first and, when `trash` names the store, its Trash.
 * A store that draws Trash is a parent even when it holds no folders.
 */
export function treeEntries(
  roots: readonly { storeId: string; root: FolderNode; trash: boolean }[],
  closed: ReadonlySet<string>,
  trashPath: string,
): TreeEntry[] {
  const entries: TreeEntry[] = [];
  const visit = (
    storeId: string,
    node: FolderNode,
    level: number,
    trash = false,
  ): boolean => {
    const key = `${storeId}|${node.path}`;
    const open = !closed.has(key);
    entries.push({
      key,
      storeId,
      path: node.path,
      level,
      expandable: node.folders.length > 0 || trash,
      open,
    });
    if (open)
      for (const folder of node.folders) visit(storeId, folder, level + 1);
    return open;
  };
  for (const { storeId, root, trash } of roots) {
    const open = visit(storeId, root, 1, trash);
    if (open && trash)
      entries.push({
        key: `${storeId}|${trashPath}`,
        storeId,
        path: trashPath,
        level: 2,
        expandable: false,
        open: false,
      });
  }
  return entries;
}

export type TreeKeyAction =
  | { kind: 'focus'; index: number }
  | { kind: 'toggle'; index: number }
  | { kind: 'select'; index: number };

/**
 * What `key` does on the tree row at `index`: Up and Down move between drawn
 * rows, Right opens a closed folder or moves into an open one, Left closes an
 * open folder or moves to its parent, and Enter or Space selects the row.
 */
export function treeKeyAction(
  key: string,
  entries: readonly TreeEntry[],
  index: number,
): TreeKeyAction | null {
  const entry = entries[index];
  if (!entry) return null;
  switch (key) {
    case 'ArrowDown':
      return index < entries.length - 1
        ? { kind: 'focus', index: index + 1 }
        : null;
    case 'ArrowUp':
      return index > 0 ? { kind: 'focus', index: index - 1 } : null;
    case 'Home':
      return { kind: 'focus', index: 0 };
    case 'End':
      return { kind: 'focus', index: entries.length - 1 };
    case 'ArrowRight':
      if (!entry.expandable) return null;
      if (!entry.open) return { kind: 'toggle', index };
      return (entries[index + 1]?.level ?? 0) > entry.level
        ? { kind: 'focus', index: index + 1 }
        : null;
    case 'ArrowLeft': {
      if (entry.expandable && entry.open) return { kind: 'toggle', index };
      for (let parent = index - 1; parent >= 0; parent -= 1)
        if (entries[parent].level < entry.level)
          return { kind: 'focus', index: parent };
      return null;
    }
    case 'Enter':
    case ' ':
      return { kind: 'select', index };
    default:
      return null;
  }
}
