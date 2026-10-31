/**
 * Sidebar store tree rendering the Vaults and Teams groups with expandable
 * folder hierarchies.
 *
 * Every store renders a row even when empty so the store can be selected from
 * the sidebar. Store trees start collapsed; selecting a store does not expand
 * it because the Files list already groups root items by top-level folder.
 * Selecting a subfolder from the list expands its ancestor folders so the
 * selection remains visible in the tree.
 *
 * Rows set `data-folder-store` and `data-folder-path` for the shared folder
 * context menu (`screens/folder-menu.tsx`), triggered via right-click,
 * ContextMenu, or Shift-F10.
 *
 * A store with entries in Trash, or whose Trash is open, lists a Trash row
 * after its folders; it opens the store's Trash view and has no folder menu.
 */

import { Fragment, useEffect, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { Icon, MenuButton, MenuItem, StoreMark } from '../components';
import {
  catalog,
  catalogFolders,
  displayPathComponent,
  storeAvailability,
  storeDescription,
  storeDescriptionState,
  storeDisplayOrder,
  storeReadable,
  TRASH_ROOT,
  trashEntries,
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
  /** `LocationState.folder`: selected store or folder in a Files view. */
  folder: string;
  /** Opens a store or folder using the `all`-route `store|/path` format. */
  onOpen: (folder: string) => void;
  onNavigate: (location: Location) => void;
  /** Folder context menu action callbacks; when omitted, menu items are disabled. */
  folderActions?: Omit<FolderMenuActions, 'onNavigate'>;
  /** Trailing rows rendered at the end of the Teams group (e.g. All Teams). */
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
  /** 1-based index among siblings and total sibling count. */
  position: { index: number; of: number };
  /** Whether the store is an inactive team (renders the label and mark in muted style). */
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
        // Pointer-only disclosure toggle. The `treeitem` row exposes
        // `aria-expanded` and handles ArrowRight/ArrowLeft, so this button is
        // hidden from assistive technology and excluded from the Tab order.
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
          // Handle tree disclosure keys on the row: ArrowRight expands a
          // collapsed folder or moves focus to its first child; ArrowLeft
          // collapses an expanded folder or moves focus to its parent.
          // The sidebar source list handles ArrowUp and ArrowDown.
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
        {/* Separate the name and count for VoiceOver ("Personal, 3" instead of "Personal3"). */}
        {tail ? <span className="offscreen">, </span> : null}
        {tail}
      </button>
    </div>
  );
}

/**
 * Returns the first child of an expanded tree row or the parent of a tree row
 * (the next item one level deeper or the nearest preceding item one level
 * shallower within the same tree).
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

/** Returns the tree node keys for `store` root and every ancestor folder of `path`. */
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
  const trashCount = (store: Store): number =>
    storeReadable(snapshot, store.id)
      ? trashEntries(snapshot, store.id).length
      : 0;
  const vaultTrees = trees.filter((tree) => tree.store.kind === 'account');
  const teamTrees = trees.filter((tree) => tree.store.kind === 'team');

  const drawFolders = (
    folders: readonly FolderNode[],
    store: Store,
    depth: number,
    /** Rows drawn after these folders at the same level, such as Trash. */
    trailing = 0,
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
            position={{ index: index + 1, of: folders.length + trailing }}
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
    const trashOpen =
      selected.store === store.id && selected.path === TRASH_ROOT;
    const inTrash = trashCount(store);
    const trashRow = inTrash > 0 || trashOpen;
    const access = storeAvailability(snapshot, store, {
      nowSeconds: accessNow(),
    });
    const state = storeDescriptionState(snapshot, store);
    // For unavailable stores, replace the item count with a lock icon (for
    // expired check-ins) or a warning dot (for other attention states).
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
          expandable={root.folders.length > 0 || trashRow}
          disabled={disabled}
          position={{ index: index + 1, of: group.length }}
          inactive={store.kind === 'team' && store.active === false}
          onSelect={() => onOpen(`${store.id}|/`)}
          onToggle={() => toggle(key)}
        />
        {open ? drawFolders(root.folders, store, 1, trashRow ? 1 : 0) : null}
        {open && trashRow ? (
          <TreeRow
            storeId={store.id}
            path={TRASH_ROOT}
            depth={1}
            active={trashOpen}
            mark={<Icon name="trash" />}
            name="Trash"
            tail={inTrash ? <span className="n c">{inTrash}</span> : undefined}
            open={false}
            expandable={false}
            disabled={disabled}
            position={{
              index: root.folders.length + 1,
              of: root.folders.length + 1,
            }}
            onSelect={() => onOpen(`${store.id}|${TRASH_ROOT}`)}
            onToggle={() => {}}
          />
        ) : null}
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
          // When there is a single vault, attach its context menu metadata to
          // the heading as well; keyboard users access the menu from the vault row.
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
          <MenuButton
            label=""
            className="plus"
            menuLabel="Create or join a team"
            aria-label="Create or join a team"
            title="Create or join a team"
            variant="quiet"
            icon="plus"
            trailingIcon={null}
            align="start"
            disabled={disabled}
          >
            {() => (
              <>
                <MenuItem
                  onClick={() => onNavigate({ kind: 'teams', open: 'create' })}
                >
                  Create a Team…
                </MenuItem>
                <MenuItem
                  onClick={() => onNavigate({ kind: 'teams', open: 'join' })}
                >
                  Join a Team…
                </MenuItem>
                <MenuItem onClick={() => onNavigate({ kind: 'teams' })}>
                  Show All Teams
                </MenuItem>
              </>
            )}
          </MenuButton>
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
