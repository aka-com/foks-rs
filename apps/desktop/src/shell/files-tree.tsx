/**
 * The sidebar's store tree: the Vaults and Teams groups, each store with a
 * disclosure triangle over its folders.
 *
 * Every store gets a row, including one with nothing in it yet, because the
 * tree is the way into a store. Trees start collapsed and opening one is the
 * reader's choice: selecting a store does not expand it, since the list
 * already groups the store's items by folder. A folder selected from the
 * list opens its ancestors so the selection stays visible.
 *
 * Rows carry `data-folder-store` and `data-folder-path`, which the shared
 * folder menu (`screens/folder-menu.tsx`) reads on a right click, the
 * ContextMenu key or Shift-F10.
 */

import { Fragment, useEffect, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { Icon, StoreMark } from '../components';
import {
  catalog,
  catalogFolders,
  displayPathComponent,
  storeAvailability,
  storeDescription,
  storeDescriptionState,
  storeDisplayOrder,
} from '../model';
import type { AgentSnapshot, Store } from '../model';
import type { Location } from '../location';
import {
  ALL_ITEMS,
  folderSelection,
  folderTree,
  listsItems,
} from '../screens/scope';
import type { FolderNode } from '../screens/scope';
import { useFolderMenu, type FolderMenuActions } from '../screens/folder-menu';

export interface FilesTreeProps {
  snapshot: AgentSnapshot;
  location: Location;
  /** `LocationState.folder`: the selected store or folder on a Files page. */
  folder: string;
  /** Opens a store or folder, as the `all` location's `store|/path` value. */
  onOpen: (folder: string) => void;
  onNavigate: (location: Location) => void;
  /** The folder menu's commands; without them the menu offers no actions. */
  folderActions?: Omit<FolderMenuActions, 'onNavigate'>;
  /** Rows drawn at the end of the Teams group (All Teams). */
  teamsTail?: ReactNode;
  disabled?: boolean;
  accessNow?: () => number;
}

interface StoreTree {
  store: Store;
  root: FolderNode;
}

function TreeRow({
  storeId,
  path,
  depth,
  active,
  mark,
  name,
  tail,
  open,
  expandable,
  disabled,
  position,
  inactive = false,
  onSelect,
  onToggle,
}: {
  storeId: string;
  path: string;
  depth: number;
  active: boolean;
  mark?: ReactNode;
  name: string;
  tail?: ReactNode;
  open: boolean;
  expandable: boolean;
  disabled: boolean;
  /** 1-based position among its siblings, and their number. */
  position: { index: number; of: number };
  /** An inactive team: its name is muted, as its mark is. */
  inactive?: boolean;
  onSelect: () => void;
  onToggle: () => void;
}): ReactNode {
  return (
    <div
      className={[
        'fn',
        depth === 0 ? 'root' : '',
        active ? 'on' : '',
        inactive ? 'off' : '',
      ]
        .filter(Boolean)
        .join(' ')}
      data-folder-store={storeId}
      data-folder-path={path}
      style={{ '--d': depth } as CSSProperties}
    >
      {expandable ? (
        // The pointer's disclosure. The row is a treeitem that carries
        // aria-expanded and answers Right and Left, so the triangle stays
        // out of the tree for VoiceOver and the keyboard.
        <button
          type="button"
          className={open ? 'twist open' : 'twist'}
          aria-hidden="true"
          aria-label={open ? `Collapse ${name}` : `Expand ${name}`}
          tabIndex={-1}
          disabled={disabled}
          onClick={(event) => {
            event.stopPropagation();
            onToggle();
          }}
        >
          <Icon name="chevronRight" />
        </button>
      ) : null}
      <button
        type="button"
        className={active ? 'nav tree fselect on' : 'nav tree fselect'}
        role="treeitem"
        aria-level={depth + 1}
        aria-posinset={position.index}
        aria-setsize={position.of}
        aria-selected={active}
        aria-haspopup="menu"
        aria-current={active ? 'location' : undefined}
        aria-expanded={expandable ? open : undefined}
        data-depth={depth}
        disabled={disabled}
        onClick={onSelect}
        onKeyDown={(event) => {
          // The row is the disclosure's keyboard home, as in a Finder
          // sidebar: Right opens a closed folder and then steps into it;
          // Left closes an open one and then steps out to its parent.
          // Up and Down belong to the sidebar's source list.
          if (event.altKey || event.metaKey || event.ctrlKey) return;
          if (event.key === 'ArrowRight') {
            event.preventDefault();
            if (!expandable) return;
            if (!open) onToggle();
            else treeNeighbour(event.currentTarget, 'child')?.focus();
          } else if (event.key === 'ArrowLeft') {
            event.preventDefault();
            if (expandable && open) onToggle();
            else treeNeighbour(event.currentTarget, 'parent')?.focus();
          }
        }}
      >
        {mark ?? <Icon name="folder" />}
        <span className="t nm">{name}</span>
        {/* "Personal, 3" for VoiceOver rather than "Personal3". */}
        {tail ? <span className="offscreen">, </span> : null}
        {tail}
      </button>
    </div>
  );
}

/**
 * The first child of an open tree row, or the row's parent: the nearest row
 * after it one level deeper, or before it one level shallower, in the same
 * tree.
 */
function treeNeighbour(
  row: HTMLElement,
  direction: 'child' | 'parent',
): HTMLElement | null {
  const tree = row.closest('[role="tree"]');
  if (!tree) return null;
  const rows = Array.from(
    tree.querySelectorAll<HTMLElement>('[role="treeitem"]'),
  );
  const at = rows.indexOf(row);
  const depth = Number(row.dataset.depth ?? 0);
  if (direction === 'child') {
    const next = rows[at + 1];
    return next && Number(next.dataset.depth) === depth + 1 ? next : null;
  }
  for (let index = at - 1; index >= 0; index -= 1)
    if (Number(rows[index].dataset.depth) < depth) return rows[index];
  return null;
}

/** The keys of `path` and every folder above it, under `store`. */
function ancestors(store: string, path: string): string[] {
  const parts = path.split('/').filter(Boolean);
  const keys = [`${store}|/`];
  for (let index = 1; index < parts.length; index += 1)
    keys.push(`${store}|/${parts.slice(0, index).join('/')}`);
  return keys;
}

export function FilesTree({
  snapshot,
  location,
  folder,
  onOpen,
  onNavigate,
  folderActions,
  teamsTail,
  disabled = false,
  accessNow = () => Date.now() / 1000,
}: FilesTreeProps): ReactNode {
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const filesHere = listsItems(location);
  const selected = filesHere
    ? folderSelection(location, folder)
    : { store: '', path: '/' };
  useEffect(() => {
    if (!selected.store || selected.store === ALL_ITEMS) return;
    if (selected.path === '/') return;
    setExpanded((current) => {
      const wanted = ancestors(selected.store, selected.path);
      if (wanted.every((key) => current.has(key))) return current;
      return new Set([...current, ...wanted]);
    });
  }, [selected.store, selected.path]);
  const menu = useFolderMenu(snapshot, {
    ...folderActions,
    onNavigate,
    accessNow,
  });
  const toggle = (key: string): void =>
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  const items = catalog(snapshot);
  const folderEntries = catalogFolders(snapshot);
  const trees: StoreTree[] = storeDisplayOrder(snapshot).map((store) => ({
    store,
    root: folderTree(
      items.filter((item) => item.store === store.id),
      folderEntries.filter((item) => item.store === store.id),
    ),
  }));
  const vaultTrees = trees.filter((tree) => tree.store.kind === 'account');
  const teamTrees = trees.filter((tree) => tree.store.kind === 'team');

  const drawFolders = (
    folders: readonly FolderNode[],
    store: Store,
    depth: number,
  ): ReactNode =>
    folders.map((node, index) => {
      const key = `${store.id}|${node.path}`;
      const open = expanded.has(key);
      return (
        <Fragment key={key}>
          <TreeRow
            storeId={store.id}
            path={node.path}
            depth={depth}
            active={selected.store === store.id && selected.path === node.path}
            name={displayPathComponent(node.name)}
            tail={<span className="n c">{node.count}</span>}
            open={open}
            expandable={node.folders.length > 0}
            disabled={disabled}
            position={{ index: index + 1, of: folders.length }}
            onSelect={() => onOpen(`${store.id}|${node.path}`)}
            onToggle={() => toggle(key)}
          />
          {open ? drawFolders(node.folders, store, depth + 1) : null}
        </Fragment>
      );
    });

  const storeRow = (
    tree: StoreTree,
    index: number,
    group: readonly StoreTree[],
  ): ReactNode => {
    const { store, root } = tree;
    const key = `${store.id}|/`;
    const open = expanded.has(key);
    const access = storeAvailability(snapshot, store, {
      nowSeconds: accessNow(),
    });
    const state = storeDescriptionState(snapshot, store);
    // A store that cannot be used says so in the count's place: a lock for
    // a lapsed check-in, a dot for anything else that needs attention.
    const tail =
      !access.available && access.reason === 'check-in-expired' ? (
        <span
          className="n lock"
          role="img"
          aria-label={storeDescription(snapshot, store)}
          title={storeDescription(snapshot, store)}
        >
          <Icon name="lock" />
        </span>
      ) : state !== 'normal' ? (
        <span
          className="dot warn"
          role="img"
          aria-label={storeDescription(snapshot, store)}
          title={storeDescription(snapshot, store)}
        />
      ) : (
        <span className="n c">{root.count}</span>
      );
    return (
      <Fragment key={store.id}>
        <TreeRow
          storeId={store.id}
          path="/"
          depth={0}
          active={selected.store === store.id && selected.path === '/'}
          mark={<StoreMark snapshot={snapshot} store={store} />}
          name={store.name}
          tail={tail}
          open={open}
          expandable={root.folders.length > 0}
          disabled={disabled}
          position={{ index: index + 1, of: group.length }}
          inactive={store.kind === 'team' && store.active === false}
          onSelect={() => onOpen(`${store.id}|/`)}
          onToggle={() => toggle(key)}
        />
        {open ? drawFolders(root.folders, store, 1) : null}
      </Fragment>
    );
  };

  const single = vaultTrees.length === 1 ? vaultTrees[0].store : null;
  return (
    <div
      className="side-tree"
      onContextMenu={menu.handlers.onContextMenu}
      onKeyDown={menu.handlers.onKeyDown}
    >
      {menu.menu}
      <div className="nav-group">
        <h6
          className="nav-h"
          id="side-vaults"
          // With one vault the heading stands for it, so a right click there
          // opens its menu too. The vault's own row is the keyboard's way in.
          aria-haspopup={single ? 'menu' : undefined}
          data-folder-store={single?.id}
          data-folder-path={single ? '/' : undefined}
        >
          {single ? 'Your vault' : 'Vaults'}
        </h6>
        <div role="tree" aria-labelledby="side-vaults">
          {vaultTrees.map(storeRow)}
        </div>
      </div>
      <div className="nav-group">
        <h6 className="nav-h">
          <span id="side-teams">Teams</span>
          <button
            type="button"
            className="tbtn plus"
            title="New team"
            aria-label="New team"
            disabled={disabled}
            onClick={() => onNavigate({ kind: 'teams', open: 'create' })}
          >
            <Icon name="plus" />
          </button>
        </h6>
        {teamTrees.length ? (
          <div role="tree" aria-labelledby="side-teams">
            {teamTrees.map(storeRow)}
          </div>
        ) : null}
        {!teamTrees.length ? (
          <button
            type="button"
            className="nav dim"
            disabled={disabled}
            onClick={() => onNavigate({ kind: 'teams' })}
          >
            <Icon name="plus" />
            <span className="t">Join or create a team</span>
          </button>
        ) : null}
        {teamsTail}
      </div>
    </div>
  );
}
