/**
 * The destination tree of the move sheet: the folders of one store, read
 * from the catalog's items and folder entries, plus folders created while the
 * sheet is open. It is a single-select tree whose selection is the
 * destination, so the arrow keys move the selection with the focus.
 */

import { useRef } from 'react';
import type { CSSProperties, KeyboardEvent, ReactNode } from 'react';
import { Icon } from '../../components';
import { displayPathComponent, isTrashPath } from '../../model';
import type { AgentSnapshot, Item } from '../../model';
import { within } from './item-moves';
import { folderTree } from '../../model/scope';
import type { FolderNode } from '../../model/scope';

export interface PickerFolder {
  path: string;
  /** The decoded last segment; empty for the store's root. */
  name: string;
  depth: number;
}

/**
 * The folders of `store` a move can target, in tree order and starting with
 * the root. Trash is left out: entries go there through Move to Trash.
 * `known` adds folders created since the catalog was read.
 */
export function pickerFolders(
  snapshot: AgentSnapshot,
  store: string,
  known: readonly string[] = [],
): PickerFolder[] {
  const entries: Item[] = snapshot.items.filter(
    (item) => item.store === store && !isTrashPath(item.path),
  );
  for (const path of known)
    entries.push({
      store,
      path,
      direntId: '',
      kind: 'Folder',
      size: 0,
      version: 0,
      read: { role: 'Owner' },
      write: { role: 'Owner' },
    });
  const folders: PickerFolder[] = [];
  const walk = (node: FolderNode, depth: number): void => {
    folders.push({
      path: node.path,
      name: displayPathComponent(node.name),
      depth,
    });
    for (const child of node.folders) walk(child, depth + 1);
  };
  walk(folderTree(entries), 0);
  return folders;
}

export function FolderPicker({
  label,
  storeName,
  folders,
  value,
  current,
  moving,
  disabled = false,
  onChange,
}: {
  label: string;
  storeName: string;
  folders: readonly PickerFolder[];
  /** The selected folder, or null when the typed path names none listed. */
  value: string | null;
  /** The folder the items are in now, when they share one. */
  current: string | null;
  /** Folders being moved, which cannot hold themselves. */
  moving: readonly string[];
  disabled?: boolean;
  onChange: (path: string) => void;
}): ReactNode {
  const treeRef = useRef<HTMLDivElement>(null);
  const off = (path: string): boolean =>
    moving.some((folder) => within(path, folder));
  const enabled = folders.filter((folder) => !off(folder.path));
  const focusable =
    value !== null && enabled.some((folder) => folder.path === value)
      ? value
      : (enabled[0]?.path ?? null);
  const choose = (path: string): void => {
    onChange(path);
    const node = [
      ...(treeRef.current?.querySelectorAll<HTMLElement>('[role="treeitem"]') ??
        []),
    ].find((element) => element.dataset.pickerPath === path);
    node?.focus();
  };
  const onKeyDown = (
    event: KeyboardEvent<HTMLDivElement>,
    path: string,
  ): void => {
    if (disabled) return;
    const at = enabled.findIndex((folder) => folder.path === path);
    const next =
      event.key === 'ArrowDown'
        ? enabled[at + 1]
        : event.key === 'ArrowUp'
          ? enabled[at - 1]
          : event.key === 'Home'
            ? enabled[0]
            : event.key === 'End'
              ? enabled.at(-1)
              : undefined;
    if (next) {
      event.preventDefault();
      choose(next.path);
    } else if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      if (!off(path)) onChange(path);
    }
  };
  return (
    <div
      ref={treeRef}
      className="folder-picker"
      role="tree"
      aria-label={label}
      aria-disabled={disabled || undefined}
    >
      {folders.map((folder) => {
        const blocked = off(folder.path);
        const selected = folder.path === value;
        return (
          <div
            key={folder.path}
            role="treeitem"
            aria-level={folder.depth + 1}
            aria-selected={selected}
            aria-disabled={blocked || undefined}
            tabIndex={folder.path === focusable ? 0 : -1}
            data-picker-path={folder.path}
            className={['fp-node', selected ? 'on' : '', blocked ? 'off' : '']
              .filter(Boolean)
              .join(' ')}
            style={{ '--d': folder.depth } as CSSProperties}
            onClick={() => {
              if (!blocked && !disabled) onChange(folder.path);
            }}
            onKeyDown={(event) => onKeyDown(event, folder.path)}
          >
            <Icon name={folder.depth === 0 ? 'vault' : 'folder'} />
            <span className="nm">
              {folder.depth === 0 ? storeName : folder.name}
            </span>
            {blocked ? (
              <span className="tag">Being moved</span>
            ) : folder.path === current ? (
              <span className="tag">Current folder</span>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
