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
 */

import { Fragment, useEffect, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { Icon, StoreMark } from '../components';
import {
  catalog,
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
  onSelect: () => void;
  onToggle: () => void;
}): ReactNode {
  return (
    <div
      className={['fn', depth === 0 ? 'root' : '', active ? 'on' : '']
        .filter(Boolean)
        .join(' ')}
      data-folder-store={storeId}
      data-folder-path={path}
      style={{ '--d': depth } as CSSProperties}
    >
      {expandable ? (
        <button
          type="button"
          className={open ? 'twist open' : 'twist'}
          aria-expanded={open}
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
        aria-haspopup="menu"
        aria-current={active ? 'location' : undefined}
        aria-expanded={expandable ? open : undefined}
        disabled={disabled}
        onClick={onSelect}
        onKeyDown={(event) => {
          // The row is the disclosure's keyboard home: Right opens, Left
          // closes, as in a Finder sidebar.
          if (!expandable) return;
          if (event.key === 'ArrowRight' && !open) {
            event.preventDefault();
            onToggle();
          } else if (event.key === 'ArrowLeft' && open) {
            event.preventDefault();
            onToggle();
          }
        }}
      >
        {mark ?? <Icon name="folder" />}
        <span className="t nm">{name}</span>
        {tail}
      </button>
    </div>
  );
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
  const trees: StoreTree[] = storeDisplayOrder(snapshot).map((store) => ({
    store,
    root: folderTree(items.filter((item) => item.store === store.id)),
  }));
  const vaultTrees = trees.filter((tree) => tree.store.kind === 'account');
  const teamTrees = trees.filter((tree) => tree.store.kind === 'team');

  const drawFolders = (
    folders: readonly FolderNode[],
    store: Store,
    depth: number,
  ): ReactNode =>
    folders.map((node) => {
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
            onSelect={() => onOpen(`${store.id}|${node.path}`)}
            onToggle={() => toggle(key)}
          />
          {open ? drawFolders(node.folders, store, depth + 1) : null}
        </Fragment>
      );
    });

  const storeRow = (tree: StoreTree): ReactNode => {
    const { store, root } = tree;
    const key = `${store.id}|/`;
    const open = expanded.has(key);
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
          expandable={root.folders.length > 0}
          disabled={disabled}
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
      <div className="nav-group" role="group" aria-labelledby="side-vaults">
        <h6
          className="nav-h"
          id="side-vaults"
          // With one vault the heading stands for it, so its menu is there too.
          tabIndex={single ? 0 : undefined}
          aria-haspopup={single ? 'menu' : undefined}
          data-folder-store={single?.id}
          data-folder-path={single ? '/' : undefined}
        >
          {single ? 'Your vault' : 'Vaults'}
        </h6>
        {vaultTrees.map(storeRow)}
      </div>
      <div className="nav-group" role="group" aria-labelledby="side-teams">
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
        {teamTrees.map(storeRow)}
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
