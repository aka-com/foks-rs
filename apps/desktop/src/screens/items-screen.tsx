/**
 * The Files browser: a folder tree on the left and the selected folder's
 * contents in the middle. The details panel is a third column that the shell
 * mounts beside this screen; this file owns the tree and the list/grid views.
 *
 * The kind filter is located at the top of the sidebar above the item counts.
 * Search and the New button are located in the window header. The browser
 * displays "All items" by default and filters to a specific store or folder
 * when selected in the tree. Table columns include Name, Kind, Location
 * (shown when viewing multiple stores), and Size.
 */

import { Fragment, useLayoutEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import { anyDialogOpen, ContextMenu, Menu } from '/kit/overlay-primitives';
import { useDismissedOnboardingTip } from '../onboarding-tips';
import { virtualListWindow } from '/kit/virtual-list';
import { rememberFilesView, storedFilesView } from '../files-view-pref';
import type { FilesView } from '../files-view-pref';
import { filesGridWindow } from './files-grid';
import {
  Band,
  Button,
  Icon,
  MenuItem,
  Notice,
  SegmentedControl,
} from '../components';
import type { FilterKind } from '../components';
import type { FoksIconName } from '../icons';
import { PageHeader } from '../shell/page-header';
import {
  KINDS,
  KIND_LIST,
  catalog,
  defaultCreateStore,
  displayPathComponent,
  fmtSize,
  initials,
  kindLabel,
  kindOf,
  nameOf,
  partiesOf,
  prefixOf,
  storeDescription,
  storeDescriptionState,
  storeDisplayOrder,
  storeAvailability,
  storeHues,
  storeNavigationOrder,
  storeOf,
  storeReadable,
  usernameOf,
} from '../model';
import type { Item, Store, AgentSnapshot } from '../model';
import type {
  KindFilter,
  LocationStore,
  LocationState,
  SortKey,
  SortDirection,
} from '../location';
import { normalizeCommandError } from '../bridge';
import type { Bridge } from '../bridge';
import { useFileDrop } from '../file-drop';
import { fileDropPath, writeBlockReason } from './write-workflows';
import type { NewKind } from './write-workflows';
import {
  ALL_ITEMS,
  folderAt,
  folderKey,
  folderSelection,
  folderTree,
  scopedItems,
  whereOf,
} from './scope';
import type { FolderNode } from './scope';
import { AccountMark } from './account-switcher';
import {
  itemActionProblem,
  StoreAccessTakeover,
  storeAccessBands,
} from './store-access';

/* --------------------------------------------------------------- pieces -- */

function emptyStoreCopy(store: Store): string {
  return store.kind === 'team'
    ? 'Items stored in this team vault are accessible to team members according to their assigned roles.'
    : 'Passwords and documents you add are private to this vault.';
}

function itemCount(count: number): string {
  return `${count} ${count === 1 ? 'item' : 'items'}`;
}

/**
 * Renders a store's badge (team initials with its theme hue, or account avatar
 * with its hashed color). Used consistently across the tree, title bar, and Location column.
 */
function StoreMark({
  snapshot,
  store,
  hue,
}: {
  snapshot: AgentSnapshot;
  store: Store;
  hue: string | undefined;
}): ReactNode {
  return store.kind === 'team' ? (
    <span className="av team" style={{ background: hue }} aria-hidden="true">
      {initials(store.name)}
    </span>
  ) : (
    <AccountMark name={usernameOf(snapshot, store) ?? store.account} />
  );
}

/** The table's column set: Location is drawn only while rows span stores. */
interface Columns {
  location: boolean;
}

function columnClass(columns: Columns, ...more: string[]): string {
  return ['cols', columns.location ? 'loc' : '', ...more]
    .filter(Boolean)
    .join(' ');
}

function ListHeader({
  columns,
  sort,
  direction,
  onSort,
}: {
  columns: Columns;
  sort: SortKey;
  direction: SortDirection;
  onSort: (sort: SortKey, direction: SortDirection) => void;
}): ReactNode {
  // Within one store, ordering by store is ordering by name; the Name header
  // says so while the Location header is not drawn.
  const shown = !columns.location && sort === 'group' ? 'name' : sort;
  const head = (key: SortKey, label: string, num = false): ReactNode => (
    <button
      type="button"
      className={[shown === key ? 'on' : '', num ? 'num' : '']
        .filter(Boolean)
        .join(' ')}
      aria-pressed={shown === key}
      title={`Sort by ${label.toLowerCase()} ${shown === key && direction === 'asc' ? 'descending' : 'ascending'}`}
      onClick={() =>
        onSort(key, shown === key && direction === 'asc' ? 'desc' : 'asc')
      }
    >
      {label}
      {shown === key ? (direction === 'asc' ? ' ↑' : ' ↓') : ''}
    </button>
  );
  return (
    <div className={`hdr ${columnClass(columns)}`}>
      {head('name', 'Name')}
      {head('kind', 'Kind')}
      {columns.location ? head('group', 'Location') : null}
      <span className="num">Size</span>
    </div>
  );
}

interface RowProps {
  snapshot: AgentSnapshot;
  hues: ReadonlyMap<string, string>;
  item: Item;
  columns: Columns;
  selected: boolean;
  /** A store that can be read but not written: rows are drawn back. */
  readOnly: boolean;
  onSelect: () => void;
  onCopy: (item: Item) => void;
  onReveal: (item: Item) => void;
  onDownload: (item: Item) => void;
  onDelete: (item: Item) => void;
  accessNow: () => number;
}

function Row({
  snapshot,
  hues,
  item,
  columns,
  selected,
  readOnly,
  onSelect,
  onCopy,
  onReveal,
  onDownload,
  onDelete,
  accessNow,
}: RowProps): ReactNode {
  const rowRef = useRef<HTMLDivElement>(null);
  const [menuPoint, setMenuPoint] = useState<{ x: number; y: number } | null>(
    null,
  );
  const closeMenu = (): void => setMenuPoint(null);
  const dismissMenu = (): void => {
    closeMenu();
    if (!anyDialogOpen()) rowRef.current?.focus();
  };
  const kind = kindOf(item) as FilterKind;
  const store = storeOf(snapshot, item.store);
  const name = nameOf(item.path);
  const prefix = prefixOf(item.path);
  const password = kind === 'Password';
  const readProblem = menuPoint
    ? itemActionProblem(snapshot, item, false, accessNow())
    : undefined;
  const writeProblem = menuPoint
    ? itemActionProblem(snapshot, item, true, accessNow())
    : undefined;
  const choose =
    (run: () => void): (() => void) =>
    () => {
      closeMenu();
      run();
    };
  const action = (
    label: string,
    icon: FoksIconName,
    run: (item: Item) => void,
  ): ReactNode => (
    <button
      type="button"
      title={label}
      aria-label={`${label} ${name}`}
      onClick={(event) => {
        event.stopPropagation();
        run(item);
      }}
    >
      <Icon name={icon} />
    </button>
  );
  return (
    <>
      <div
        ref={rowRef}
        className={columnClass(
          columns,
          'row',
          'one',
          selected ? 'sel' : '',
          readOnly ? 'ro' : '',
        )}
        role="button"
        tabIndex={0}
        aria-pressed={selected}
        aria-haspopup="menu"
        onClick={onSelect}
        onContextMenu={(event) => {
          event.preventDefault();
          rowRef.current?.focus();
          setMenuPoint({ x: event.clientX, y: event.clientY });
        }}
        onKeyDown={(event) => {
          if (
            event.key === 'ContextMenu' ||
            (event.key === 'F10' && event.shiftKey)
          ) {
            event.preventDefault();
            const bounds = event.currentTarget.getBoundingClientRect();
            setMenuPoint({ x: bounds.left, y: bounds.bottom });
            return;
          }
          if (event.target !== event.currentTarget) return;
          if (event.key !== 'Enter' && event.key !== ' ') return;
          event.preventDefault();
          onSelect();
        }}
      >
        <span className="name">
          <Icon
            name={password ? 'key' : 'file'}
            className={password ? 'k' : 'f'}
          />
          <span className="nm" title={name}>
            {name}
          </span>
          {/* Inside a folder every row shares the location, so the path is
            drawn only where rows come from more than one of them. */}
          {columns.location && prefix ? (
            <span className="fpath">/{prefix}</span>
          ) : null}
        </span>
        <span className="cell kind">{kindLabel(kind)}</span>
        {columns.location ? (
          <span className="cell mark">
            {store ? (
              <StoreMark
                snapshot={snapshot}
                store={store}
                hue={hues.get(store.id)}
              />
            ) : null}
            <span>{whereOf(snapshot, item)}</span>
          </span>
        ) : null}
        <span className="cell num">{fmtSize(item.size)}</span>
        <span className="acts">
          {item.kind !== 'File' ? (
            <>
              {action('Copy', 'copy', onCopy)}
              {action('Reveal', 'eye', onReveal)}
            </>
          ) : (
            action('Download', 'download', onDownload)
          )}
        </span>
      </div>
      {menuPoint ? (
        <ContextMenu
          point={menuPoint}
          className="menu-portal"
          onClose={dismissMenu}
        >
          <Menu
            className="menu"
            aria-label={`${name} actions`}
            anchorRef={rowRef}
            initialFocus="first"
            onClose={closeMenu}
          >
            <MenuItem icon="file" onClick={choose(onSelect)}>
              Show details
            </MenuItem>
            {item.kind === 'File' ? (
              <MenuItem
                icon="download"
                reason={readProblem}
                onClick={choose(() => onDownload(item))}
              >
                Download
              </MenuItem>
            ) : (
              <>
                <MenuItem
                  icon="copy"
                  reason={readProblem}
                  onClick={choose(() => onCopy(item))}
                >
                  Copy
                </MenuItem>
                <MenuItem
                  icon="eye"
                  reason={readProblem}
                  onClick={choose(() => onReveal(item))}
                >
                  Reveal
                </MenuItem>
              </>
            )}
            <MenuItem
              icon="trash"
              danger
              reason={writeProblem}
              onClick={choose(() => onDelete(item))}
            >
              Delete
            </MenuItem>
          </Menu>
        </ContextMenu>
      ) : null}
    </>
  );
}

interface StoreTree {
  store: Store;
  root: FolderNode;
}

/** A subfolder of the selected folder, listed above its items. */
function FolderRow({
  name,
  count,
  columns,
  onSelect,
  storeId,
  path,
}: {
  storeId: string;
  path: string;
  name: string;
  count: number;
  columns: Columns;
  onSelect: () => void;
}): ReactNode {
  return (
    <div
      className={columnClass(columns, 'row', 'one', 'folder')}
      data-folder-store={storeId}
      data-folder-path={path}
      aria-haspopup="menu"
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        onSelect();
      }}
    >
      <span className="name">
        <Icon name="folder" className="f" />
        <span className="nm" title={name}>
          {name}
        </span>
      </span>
      <span className="cell kind">Folder</span>
      {columns.location ? <span className="cell" /> : null}
      <span className="cell num">{itemCount(count)}</span>
    </div>
  );
}

function TreeRow({
  depth,
  active,
  root,
  quiet,
  icon,
  mark,
  name,
  count,
  tag,
  open,
  expandable,
  action,
  onSelect,
  onToggle,
  storeId,
  path,
}: {
  storeId?: string;
  path?: string;
  depth: number;
  active: boolean;
  root?: boolean;
  /** The muted invitation row a section draws in place of no rows at all. */
  quiet?: boolean;
  icon: 'folder' | 'vault' | 'grid' | 'plus';
  /** Drawn in place of `icon` when the row names a store. */
  mark?: ReactNode;
  name: string;
  /** Omitted, not drawn as 0, when there is nothing to count. */
  count?: number;
  /** Replaces the count when the row has a condition to state instead. */
  tag?: ReactNode;
  open: boolean;
  expandable: boolean;
  /** A control that belongs to the row, drawn after its name. */
  action?: ReactNode;
  onSelect: () => void;
  onToggle: () => void;
}): ReactNode {
  return (
    <div
      className={[
        'fn',
        active ? 'on' : '',
        root ? 'root' : '',
        quiet ? 'quiet' : '',
      ]
        .filter(Boolean)
        .join(' ')}
      data-folder-store={storeId}
      data-folder-path={path}
      style={{ '--d': depth } as React.CSSProperties}
    >
      {/* Every row keeps the twist cell so names align down the column; a row
          with nothing to disclose leaves it empty. */}
      {expandable ? (
        <button
          type="button"
          className={open ? 'twist open' : 'twist'}
          aria-expanded={open}
          aria-label={open ? `Collapse ${name}` : `Expand ${name}`}
          onClick={(event) => {
            event.stopPropagation();
            onToggle();
          }}
        >
          <Icon name="chevronDown" />
        </button>
      ) : (
        <span className="twist none" aria-hidden="true" />
      )}
      <button
        type="button"
        className="fselect"
        aria-haspopup={storeId ? 'menu' : undefined}
        aria-current={active ? 'location' : undefined}
        onClick={onSelect}
      >
        {mark ?? <Icon name={icon} />}
        <span className="nm">{name}</span>
      </button>
      {action}
      {tag ?? (count === undefined ? null : <span className="c">{count}</span>)}
    </div>
  );
}

/* ------------------------------------------------------------- uploads -- */

/** Where a file dropped on the current view is saved. */
export interface DropTarget {
  store: Store;
  /** The selected folder, when one narrows the destination. */
  folder?: string;
  /** Why the vault cannot take an upload, or null when it can. */
  blocked: string | null;
}

/** The request a dropped file turns into. */
export interface DropUpload {
  storeId: string;
  path: string;
  sourcePath: string;
}

/**
 * A drop zone covering the vault's content area.
 *
 * The runtime reports drops for the whole window rather than for an element,
 * so this renders a veil naming the destination instead of relying on the
 * pointer position, and stays out of the pointer's way while it is shown.
 */
function VaultDropZone({
  bridge,
  target,
  onUpload,
}: {
  bridge: Bridge;
  target: DropTarget;
  onUpload: (upload: DropUpload) => Promise<void>;
}): ReactNode {
  const toasts = useToast();
  const [hovering, setHovering] = useState(false);
  const [uploading, setUploading] = useState<string | null>(null);
  // Guards against a second drop landing while the first upload is in flight:
  // the agent serializes mutations and would refuse the overlapping write.
  const inFlight = useRef(false);
  const warn = (message: string): void => {
    toasts.show(message, { tone: 'warning' });
  };
  useFileDrop({
    bridge,
    active: true,
    priority: 'background',
    onHover: setHovering,
    onPaths: (paths) => {
      setHovering(false);
      if (target.blocked) {
        warn(`${target.store.name}: ${target.blocked}`);
        return;
      }
      if (paths.length !== 1) {
        warn('Drop one file at a time to upload it.');
        return;
      }
      const sourcePath = paths[0];
      const name = sourcePath?.split(/[\\/]/).at(-1);
      if (!sourcePath || !name || inFlight.current) return;
      inFlight.current = true;
      setUploading(name);
      void onUpload({
        storeId: target.store.id,
        path: fileDropPath(name, target.folder),
        sourcePath,
      })
        .catch((error: unknown) => {
          warn(normalizeCommandError(error).message);
        })
        .finally(() => {
          inFlight.current = false;
          setUploading(null);
        });
    },
    onError: (error) => {
      warn(normalizeCommandError(error).message);
    },
  });
  if (!uploading && !hovering) return null;
  const folder = target.folder && target.folder !== '/' ? target.folder : null;
  return (
    <div
      className={target.blocked && !uploading ? 'drop-veil off' : 'drop-veil'}
      role="status"
      aria-live="polite"
    >
      <div className="drop-card">
        <span className="glyph">
          <Icon name="file" />
        </span>
        <b>
          {uploading
            ? `Uploading ${uploading}…`
            : target.blocked
              ? `Cannot upload to ${target.store.name}`
              : `Drop to upload to ${target.store.name}`}
        </b>
        <small>
          {uploading
            ? 'Encrypting and saving this file'
            : (target.blocked ??
              (folder ? `Saved in ${folder}` : 'Saved at the top level'))}
        </small>
      </div>
    </div>
  );
}

/* --------------------------------------------------------------- screen -- */

export interface ItemsScreenProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  state: LocationState;
  locations: LocationStore;
  /** Opens the selected item's value in the details panel, already revealed. */
  onReveal: (item: Item) => void;
  onNew: (kind: NewKind, storeId: string, folder?: string) => void;
  onResume: (storeId: string) => Promise<void>;
  onDelete: (item: Item) => void;
  onSettings: (storeId: string) => void;
  onTeamInfo?: (storeId: string, trigger?: HTMLElement) => void;
  onNewFolder?: (storeId: string, path: string) => void;
  onCommandError: (error: unknown, item?: Item) => void;
  /** Saves a file dropped on the vault's content area. */
  onUploadDroppedFile?: (upload: DropUpload) => Promise<void>;
  /** Cleared while a modal workflow owns the drop, such as the new-item sheet. */
  dropEnabled?: boolean;
  accessNow?: () => number;
}

/** Single-line row height now that metadata is displayed in table columns. */
const ROW_HEIGHT = 38;

/** Whether `path` is `folder` itself or lies under it. */
function underFolder(path: string, folder: string): boolean {
  return folder === '/' || path === folder || path.startsWith(`${folder}/`);
}

export function ItemsScreen({
  snapshot,
  bridge,
  state,
  locations,
  onReveal,
  onNew,
  onResume,
  onDelete,
  onSettings,
  onTeamInfo,
  onNewFolder,
  onCommandError,
  onUploadDroppedFile,
  dropEnabled = true,
  accessNow = () => Date.now() / 1000,
}: ItemsScreenProps): ReactNode {
  const toasts = useToast();
  const [view, setView] = useState<FilesView>(storedFilesView);
  const [folderMenu, setFolderMenu] = useState<{
    x: number;
    y: number;
    storeId: string;
    path: string;
    trigger: HTMLElement;
  } | null>(null);
  const folderMenuAnchor = useRef<HTMLElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [listMetrics, setListMetrics] = useState({
    top: 0,
    viewport: 600,
    width: 600,
  });
  const changeView = (next: FilesView): void => {
    if (next === view) return;
    setView(next);
    rememberFilesView(next);
    // The layouts have different heights; start at the top without changing
    // the folder, filters, sorting, or selected item's details.
    if (bodyRef.current) bodyRef.current.scrollTop = 0;
    setScrollTop(0);
  };
  const [nextUpDismissed, setNextUpDismissed] =
    useDismissedOnboardingTip('add-password');
  const { location } = state;
  const store =
    location.kind === 'store' ? storeOf(snapshot, location.ref) : undefined;
  const storePage = location.kind === 'store';
  const selected = folderSelection(location, state.folder);
  // Search flattens the browser to matching results; it must not invalidate
  // the selected folder or change where New saves.
  const folderItems = scopedItems(snapshot, { ...state, query: '' });
  // The tree draws every store whatever page it is on: a store page pins the
  // list to one store, not the map beside it, so its counts come from the
  // whole catalog rather than the page's own scope.
  const treeItems = storePage
    ? scopedItems(snapshot, { ...state, query: '', location: { kind: 'all' } })
    : folderItems;
  // A search runs over the tree's selection — one store and folder, or every
  // item from the "All items" leaf — which is what the placeholder promises.
  const items = scopedItems(snapshot, state).filter(
    (item) =>
      selected.store === ALL_ITEMS ||
      (item.store === selected.store && underFolder(item.path, selected.path)),
  );
  // Every store gets a tree entry, including one with nothing in it yet: the
  // tree is the only way into a store now, so none may be left unreachable.
  const trees: StoreTree[] = storeDisplayOrder(snapshot).map((candidate) => ({
    store: candidate,
    root: folderTree(treeItems.filter((item) => item.store === candidate.id)),
  }));
  const vaultTrees = trees.filter((tree) => tree.store.kind === 'account');
  const teamTrees = trees.filter((tree) => tree.store.kind === 'team');
  // Teams keep the same hue here as they do in the account menu and Teams tab.
  const hues = storeHues(storeNavigationOrder(snapshot));

  useLayoutEffect(() => {
    const body = bodyRef.current;
    const list = listRef.current;
    if (!body || !list) return;
    const measure = (): void => {
      setListMetrics({
        top: Math.max(
          0,
          list.getBoundingClientRect().top -
            body.getBoundingClientRect().top +
            body.scrollTop,
        ),
        viewport: body.clientHeight || 600,
        width: list.clientWidth || body.clientWidth || 600,
      });
    };
    measure();
    const observer =
      typeof ResizeObserver === 'undefined'
        ? null
        : new ResizeObserver(measure);
    observer?.observe(body);
    observer?.observe(list);
    return () => observer?.disconnect();
  }, [
    items.length,
    state.kind,
    state.query,
    state.folder,
    location.kind,
    view,
  ]);

  // Show the unavailable state when a stale link or catalog refresh refers to
  // a store that is no longer present.
  if (location.kind === 'store' && !store) {
    return (
      <>
        <PageHeader title="Vault unavailable" subtitle="" />
        <div className="body">
          <Notice severity="crit" title="Vault no longer available">
            <p>Refresh, or choose another vault from Files.</p>
          </Notice>
        </div>
      </>
    );
  }

  if (store && storeDescriptionState(snapshot, store) !== 'normal') {
    return (
      <StoreAccessTakeover
        snapshot={snapshot}
        store={store}
        onOpenServer={(profile) =>
          locations.navigate({ kind: 'settings', section: 'account', profile })
        }
        onFinishSetup={() => void onResume(store.id)}
        headerAction={
          store.kind === 'team' ? (
            <Button
              variant="quiet"
              icon="settings"
              title="Team settings"
              aria-label="Team settings"
              onClick={() => onSettings(store.id)}
            />
          ) : undefined
        }
      />
    );
  }

  const accessBands = storeAccessBands(snapshot);
  const kindMeta = state.kind === 'All' ? null : KINDS[state.kind];
  const selectedTree = trees.find((tree) => tree.store.id === selected.store);
  const selectedNode = selectedTree
    ? (folderAt(selectedTree.root, selected.path) ?? selectedTree.root)
    : undefined;
  const closed = new Set(state.closedFolders);
  const availability = (candidate: Store) =>
    storeAvailability(snapshot, candidate, { nowSeconds: accessNow() });
  /** True if the store's check-in lease has expired (read-only). */
  const leaseExpired = (candidate: Store): boolean => {
    const access = availability(candidate);
    return !access.available && access.reason === 'check-in-expired';
  };

  /**
   * Moves the browser to `path` in `target`. A `store` location is pinned to
   * one store; clicking a *different* store while pinned leaves that page for
   * the broad tree first, so every store the tree lists stays reachable from
   * it, even from a deep link that opened on just one of them.
   */
  /** Navigates to the global "All items" view, unpinning from any single store page. */
  const goAllItems = (): void => {
    if (storePage) locations.navigate({ kind: 'all' });
    locations.setFolder(ALL_ITEMS);
  };
  const selectFolder = (target: Store, path: string): void => {
    const samePage = storePage && target.id === location.ref;
    if (storePage && !samePage) locations.navigate({ kind: 'all' });
    locations.setFolder(folderKey(samePage, target.id, path));
  };

  const drawFolders = (
    folders: readonly FolderNode[],
    treeStore: Store,
    depth: number,
  ): ReactNode =>
    folders.map((folder) => {
      const foldKey = `${treeStore.id}|${folder.path}`;
      const open = !closed.has(foldKey);
      return (
        <Fragment key={foldKey}>
          <TreeRow
            storeId={treeStore.id}
            path={folder.path}
            depth={depth}
            active={
              selected.store === treeStore.id && selected.path === folder.path
            }
            icon="folder"
            name={displayPathComponent(folder.name)}
            count={folder.count}
            open={open}
            expandable={folder.folders.length > 0}
            onSelect={() => selectFolder(treeStore, folder.path)}
            onToggle={() => locations.toggleFolder(foldKey)}
          />
          {open ? drawFolders(folder.folders, treeStore, depth + 1) : null}
        </Fragment>
      );
    });

  // Render store root row and its expandable folder sub-tree.
  const storeRoot = (tree: StoreTree): ReactNode => {
    const active = selected.store === tree.store.id && selected.path === '/';
    const foldKey = `${tree.store.id}|/`;
    const open = !closed.has(foldKey);
    const expired = leaseExpired(tree.store);
    return (
      <Fragment key={tree.store.id}>
        <TreeRow
          storeId={tree.store.id}
          path="/"
          depth={0}
          root
          active={active}
          icon="vault"
          mark={
            <StoreMark
              snapshot={snapshot}
              store={tree.store}
              hue={hues.get(tree.store.id)}
            />
          }
          name={tree.store.name}
          count={tree.root.count}
          tag={
            expired ? (
              <span className="warn" title="This server check-in has expired">
                <Icon name="alert" />
                expired
              </span>
            ) : undefined
          }
          open={open}
          expandable={tree.root.folders.length > 0}
          action={
            tree.store.kind === 'team' ? (
              <button
                type="button"
                className="fact"
                title="Team settings"
                aria-label={`${tree.store.name} settings`}
                onClick={(event) => {
                  event.stopPropagation();
                  onSettings(tree.store.id);
                }}
              >
                <Icon name="settings" />
              </button>
            ) : undefined
          }
          onSelect={() => selectFolder(tree.store, '/')}
          onToggle={() => locations.toggleFolder(foldKey)}
        />
        {open ? drawFolders(tree.root.folders, tree.store, 1) : null}
      </Fragment>
    );
  };

  const createStore =
    selectedTree && storeReadable(snapshot, selectedTree.store.id)
      ? selectedTree.store.id
      : store && storeReadable(snapshot, store.id)
        ? store.id
        : (defaultCreateStore(snapshot) ?? '');
  const createFolder =
    selectedTree && selectedNode && selected.path !== '/'
      ? selected.path
      : undefined;
  const currentStoreAvailable = (storeId: string): boolean => {
    const candidate = storeOf(snapshot, storeId);
    return Boolean(candidate && availability(candidate).available);
  };
  const createNew = (itemKind: Exclude<KindFilter, 'All'>): void => {
    if (createStore && currentStoreAvailable(createStore))
      onNew(itemKind, createStore, createFolder);
  };
  // Dropped files are uploaded to the active store or selected folder.
  // Multi-store views disable drops because no target store is selected.
  const dropStore =
    location.kind === 'store'
      ? store
      : !state.query
        ? selectedTree?.store
        : undefined;
  const dropTarget: DropTarget | null = dropStore
    ? {
        store: dropStore,
        folder: createFolder,
        blocked: !currentStoreAvailable(dropStore.id)
          ? storeDescription(snapshot, dropStore)
          : writeBlockReason(snapshot, dropStore),
      }
    : null;
  const dropZone =
    dropEnabled && dropTarget && onUploadDroppedFile ? (
      <VaultDropZone
        bridge={bridge}
        target={dropTarget}
        onUpload={onUploadDroppedFile}
      />
    ) : null;

  // Search and All items window a flat result set in either layout; folder
  // views display direct children.
  const flatMode = Boolean(state.query) || selected.store === ALL_ITEMS;
  // Location is a column only when rows span multiple stores.
  const columns: Columns = { location: selected.store === ALL_ITEMS };
  const viewport = {
    listTop: listMetrics.top,
    scrollTop,
    viewport: listMetrics.viewport,
    overscan: 3,
  };
  const gridWindow = filesGridWindow({
    ...viewport,
    count: items.length,
    width: listMetrics.width,
    location: columns.location,
    overscan: 1,
  });
  const rowWindow =
    view === 'grid'
      ? gridWindow
      : virtualListWindow({
          ...viewport,
          heights: items.map(() => ROW_HEIGHT),
        });
  const visibleRows = items.slice(rowWindow.start, rowWindow.end);

  const folders =
    state.sortDirection === 'desc'
      ? [...(selectedNode?.folders ?? [])].reverse()
      : (selectedNode?.folders ?? []);
  const folderScopedItems = selectedNode?.items ?? [];
  const showEmptyFolder =
    Boolean(selectedTree) && !folders.length && !folderScopedItems.length;

  /* ------------------------------------------------------- row actions -- */

  const request = (item: Item) => ({
    storeId: item.store,
    path: item.path,
    version: item.version,
  });
  const allowAction = (item: Item, write = false): boolean => {
    const problem = itemActionProblem(snapshot, item, write, accessNow());
    if (!problem) return true;
    toasts.show(problem, { tone: 'warning' });
    return false;
  };
  const copyItem = (item: Item): void => {
    if (!allowAction(item)) return;
    void bridge.copyItemValue(request(item)).then(
      () =>
        toasts.show(
          `${kindOf(item) === 'Password' ? 'Password' : 'Value'} copied`,
        ),
      (error: unknown) => onCommandError(error, item),
    );
  };
  const revealItem = (item: Item): void => {
    if (!allowAction(item)) return;
    // Reveal happens in the details panel, where the value is masked again on
    // a selection change or a lost window: select the item, then ask for it.
    locations.select({ store: item.store, path: item.path });
    onReveal(item);
  };
  const downloadItem = (item: Item): void => {
    if (!allowAction(item)) return;
    void bridge.downloadFile(request(item)).then(
      ({ saved }) =>
        toasts.show(
          saved ? `Downloaded ${nameOf(item.path)}` : 'Download cancelled',
        ),
      (error: unknown) => onCommandError(error, item),
    );
  };

  const selectedStore = selectedTree?.store ?? store;
  // Reads work from the local copy while writes do not: the list says so once,
  // above the rows, and draws them back rather than hiding them.
  const readOnlyReason = selectedStore
    ? !availability(selectedStore).available
      ? storeDescription(snapshot, selectedStore)
      : writeBlockReason(snapshot, selectedStore)
    : null;
  const readOnly = Boolean(readOnlyReason);
  const expiredHere = Boolean(selectedStore && leaseExpired(selectedStore));

  const row = (item: Item, chipColumns: Columns): ReactNode => (
    <Row
      key={`${item.store}|${item.path}`}
      snapshot={snapshot}
      hues={hues}
      item={item}
      columns={chipColumns}
      selected={
        state.selection?.store === item.store &&
        state.selection.path === item.path
      }
      readOnly={readOnly}
      onSelect={() => locations.select({ store: item.store, path: item.path })}
      onCopy={copyItem}
      onReveal={revealItem}
      onDownload={downloadItem}
      onDelete={(item) => {
        if (allowAction(item, true)) onDelete(item);
      }}
      accessNow={accessNow}
    />
  );

  const paneRows = selectedTree
    ? [
        ...folders.map((folder) => (
          <FolderRow
            storeId={selectedTree.store.id}
            path={folder.path}
            key={folder.path}
            name={displayPathComponent(folder.name)}
            count={folder.count}
            columns={columns}
            onSelect={() => selectFolder(selectedTree.store, folder.path)}
          />
        )),
        ...folderScopedItems.map((item) => row(item, columns)),
      ]
    : [];

  const contextStore = folderMenu
    ? storeOf(snapshot, folderMenu.storeId)
    : undefined;
  const newFolderReason = !contextStore
    ? 'This vault is no longer available.'
    : !availability(contextStore).available
      ? storeDescription(snapshot, contextStore)
      : (writeBlockReason(snapshot, contextStore) ??
        (onNewFolder
          ? undefined
          : 'Folder creation is unavailable in this window.'));
  const showFolderMenu = (
    target: EventTarget,
    x: number,
    y: number,
  ): boolean => {
    if (!(target instanceof Element) || target.closest('[role="menu"]'))
      return false;
    const row = target.closest<HTMLElement>('[data-folder-store]');
    if (!row?.dataset.folderStore || !row.dataset.folderPath) return false;
    const trigger = row.matches('[role="button"]')
      ? row
      : (row.querySelector<HTMLElement>('.fselect') ?? row);
    trigger.focus({ preventScroll: true });
    folderMenuAnchor.current = trigger;
    setFolderMenu({
      x,
      y,
      storeId: row.dataset.folderStore,
      path: row.dataset.folderPath,
      trigger,
    });
    return true;
  };
  const vaultHeading = vaultTrees.length === 1 ? 'Your vault' : 'Vaults';
  const allCount = treeItems.length;
  /** The current view's name in the title row. */
  const scopeLabel =
    selected.store === ALL_ITEMS
      ? 'All items'
      : selected.path !== '/'
        ? nameOf(selected.path)
        : (selectedStore?.name ?? 'All items');
  const shownCount =
    selected.store === ALL_ITEMS ? items.length : (selectedNode?.count ?? 0);
  const people =
    selectedStore?.kind === 'team'
      ? partiesOf(snapshot, selectedStore.id).length
      : 0;

  /* ------------------------------------------------------- empty states -- */

  const vaultEmpty = catalog(snapshot).length === 0;
  const starter = (
    <div className="starter">
      <div>
        <h2>Your vault is empty</h2>
        <p>Start by adding any of these items to your vault.</p>
      </div>
      <div className="cards">
        <button
          type="button"
          className="card"
          onClick={() => createNew('Password')}
        >
          <span className="ci">
            <Icon name="key" />
          </span>
          <b>New password</b>
          <small>A login, API key or any secret value.</small>
          <kbd>⌘N</kbd>
        </button>
        <button
          type="button"
          className="card"
          onClick={() => createNew('Document')}
        >
          <span className="ci">
            <Icon name="upload" />
          </span>
          <b>Add a document</b>
          <small>Drop a file anywhere in this window, or pick one.</small>
          <kbd>⇧⌘N</kbd>
        </button>
        {/* The app has no CLI import command, so this card states the one the
            CLI already has rather than offering an action it cannot run. */}
        <div className="card static">
          <span className="ci">
            <Icon name="terminal" />
          </span>
          <b>Import from the CLI</b>
          <small>Bring items from an existing FOKS install.</small>
          <span className="cli">foks kv export …</span>
        </div>
      </div>
    </div>
  );

  const searchMiss = (
    <div className="empty">
      <div className="big">
        <Icon name="search" />
      </div>
      <h2>Nothing matches “{state.query}”</h2>
      <p>Try a shorter word, or press ⌘K to search every vault and team.</p>
    </div>
  );

  const emptyHere = (scopeStore: Store | undefined): ReactNode => (
    <div className="empty">
      <div className="big">
        <Icon name={kindMeta?.icon ?? 'key'} />
      </div>
      <h2>No items yet</h2>
      <p>
        {scopeStore && selected.path === '/'
          ? emptyStoreCopy(scopeStore)
          : scopeStore
            ? 'This folder is empty.'
            : 'Save passwords and documents to get started.'}
      </p>
    </div>
  );

  const dropStrip =
    state.query || readOnly || !dropTarget || dropTarget.blocked ? null : (
      <div className="dropzone" aria-hidden="true">
        <Icon name="upload" />
        Drop files to add to vault
      </div>
    );

  // The suggestion applies once the vault holds something but no password:
  // the agent can sign in from one without the value living in a file.
  const nextUp =
    state.query ||
    nextUpDismissed ||
    vaultEmpty ||
    catalog(snapshot).some((item) => kindOf(item) === 'Password') ? null : (
      <div className="nextup">
        <Icon name="key" />
        <span>Next: Try adding a password.</span>
        <span className="sp" />
        <button
          type="button"
          className="lnk"
          onClick={() => createNew('Password')}
        >
          New password
        </button>
        <button
          type="button"
          className="x"
          title="Not now"
          aria-label="Not now"
          onClick={() => setNextUpDismissed(true)}
        >
          <Icon name="close" />
        </button>
      </div>
    );

  const sortSelect = (
    <label className="files-sort-control">
      <span>Sort</span>
      <select
        aria-label="Sort items"
        value={`${!columns.location && state.sort === 'group' ? 'name' : state.sort}:${state.sortDirection}`}
        onChange={(event) => {
          const [sort, direction] = event.target.value.split(':') as [
            SortKey,
            SortDirection,
          ];
          locations.setSort(sort, direction);
        }}
      >
        <option value="name:asc">Name (A–Z)</option>
        <option value="name:desc">Name (Z–A)</option>
        <option value="kind:asc">Kind (ascending)</option>
        <option value="kind:desc">Kind (descending)</option>
        {columns.location ? (
          <>
            <option value="group:asc">Location (A–Z)</option>
            <option value="group:desc">Location (Z–A)</option>
          </>
        ) : null}
      </select>
    </label>
  );
  const contentsHeader =
    view === 'list' ? (
      <ListHeader
        columns={columns}
        sort={state.sort}
        direction={state.sortDirection}
        onSort={(sort, direction) => locations.setSort(sort, direction)}
      />
    ) : null;

  const listBody = flatMode ? (
    !items.length ? (
      state.query ? (
        searchMiss
      ) : vaultEmpty ? (
        starter
      ) : (
        emptyHere(selectedStore)
      )
    ) : (
      <div className="list-window">
        {contentsHeader}
        <div className="virtual-rows" ref={listRef}>
          {rowWindow.padTop ? (
            <div
              className="virtual-spacer"
              style={{ height: rowWindow.padTop }}
            />
          ) : null}
          {view === 'grid' ? (
            <div
              className="files-grid"
              style={{
                gridTemplateColumns: `repeat(${gridWindow.columns}, minmax(0, 1fr))`,
                paddingBottom: gridWindow.trailingGap,
              }}
            >
              {visibleRows.map((item) => row(item, columns))}
            </div>
          ) : (
            visibleRows.map((item) => row(item, columns))
          )}
          {rowWindow.padBottom ? (
            <div
              className="virtual-spacer"
              style={{ height: rowWindow.padBottom }}
            />
          ) : null}
        </div>
        {nextUp}
      </div>
    )
  ) : showEmptyFolder ? (
    vaultEmpty ? (
      starter
    ) : (
      emptyHere(selectedTree?.store)
    )
  ) : (
    <div className="list-window">
      {contentsHeader}
      <div className={view === 'grid' ? 'files-grid' : 'virtual-rows'}>
        {paneRows}
      </div>
      {nextUp}
    </div>
  );
  const listShown = flatMode ? items.length > 0 : !showEmptyFolder;

  return (
    <div className="drop-area">
      {dropZone}
      <div
        className="folder-layout"
        onContextMenu={(event) => {
          if (showFolderMenu(event.target, event.clientX, event.clientY))
            event.preventDefault();
        }}
        onKeyDown={(event) => {
          if (
            event.key !== 'ContextMenu' &&
            !(event.key === 'F10' && event.shiftKey)
          )
            return;
          const bounds = (event.target as HTMLElement).getBoundingClientRect();
          if (showFolderMenu(event.target, bounds.left, bounds.bottom)) {
            event.preventDefault();
            event.stopPropagation();
          }
        }}
      >
        {folderMenu ? (
          <ContextMenu
            point={folderMenu}
            className="menu-portal"
            onClose={() => {
              setFolderMenu(null);
              if (!anyDialogOpen())
                folderMenuAnchor.current?.focus({ preventScroll: true });
            }}
          >
            <Menu
              className="menu"
              aria-label="Folder actions"
              anchorRef={folderMenuAnchor}
              initialFocus="first"
              onClose={() => setFolderMenu(null)}
            >
              <MenuItem
                icon="plus"
                reason={newFolderReason}
                onClick={() => {
                  setFolderMenu(null);
                  onNewFolder?.(folderMenu.storeId, folderMenu.path);
                }}
              >
                New folder
              </MenuItem>
              {folderMenu.path !== '/' ? (
                <MenuItem
                  icon="trash"
                  danger
                  reason="Deleting folders is not supported by the desktop agent."
                >
                  Delete folder
                </MenuItem>
              ) : null}
              {contextStore?.kind === 'team' ? (
                <>
                  <div role="separator" className="menu-separator" />
                  <MenuItem
                    icon="info"
                    reason={
                      onTeamInfo
                        ? undefined
                        : 'Team info is unavailable in this window.'
                    }
                    onClick={() => {
                      setFolderMenu(null);
                      onTeamInfo?.(contextStore.id, folderMenu.trigger);
                    }}
                  >
                    Team info
                  </MenuItem>
                </>
              ) : null}
            </Menu>
          </ContextMenu>
        ) : null}
        <div className="folder-split">
          <aside className="tpane" aria-label="Folders">
            {/* Filter control for item kind (passwords, documents, etc.). */}
            <div className="filt">
              <SegmentedControl<KindFilter>
                label="Filter items by kind"
                value={state.kind}
                onChange={(kind) => {
                  locations.setKind(kind);
                }}
                items={[
                  { id: 'All', label: 'All' },
                  ...KIND_LIST.map((name) => ({
                    id: name,
                    label: KINDS[name].plural,
                    title: KINDS[name].blurb,
                  })),
                ]}
              />
            </div>
            <div className="tscroll">
              <div className="tree-all">
                <TreeRow
                  depth={0}
                  root
                  active={selected.store === ALL_ITEMS}
                  icon="grid"
                  name="All items"
                  count={allCount}
                  open={false}
                  expandable={false}
                  onSelect={() => goAllItems()}
                  onToggle={() => {}}
                />
              </div>
              <h6
                tabIndex={vaultTrees.length === 1 ? 0 : undefined}
                aria-haspopup={vaultTrees.length === 1 ? 'menu' : undefined}
                data-folder-store={
                  vaultTrees.length === 1 ? vaultTrees[0].store.id : undefined
                }
                data-folder-path="/"
              >
                {vaultHeading}
              </h6>
              {vaultTrees.map(storeRoot)}
              {
                <h6>
                  Teams
                  <button
                    type="button"
                    className="plus"
                    title="New team"
                    aria-label="New team"
                    onClick={() =>
                      locations.navigate({ kind: 'teams', open: 'create' })
                    }
                  >
                    <Icon name="plus" />
                  </button>
                </h6>
              }
              {teamTrees.map(storeRoot)}
              {!teamTrees.length ? (
                <TreeRow
                  depth={0}
                  quiet
                  active={false}
                  icon="plus"
                  name="Join or create a team"
                  open={false}
                  expandable={false}
                  onSelect={() => locations.navigate({ kind: 'teams' })}
                  onToggle={() => {}}
                />
              ) : null}
            </div>
          </aside>
          <section className="lpane" aria-label="Folder contents">
            <div className="lt">
              <span className="where">
                {state.query ? (
                  <>
                    Results for “{state.query}”
                    <span className="sub search-scope">in {scopeLabel}</span>
                  </>
                ) : (
                  <>
                    {selectedStore && selected.path === '/' ? (
                      <StoreMark
                        snapshot={snapshot}
                        store={selectedStore}
                        hue={hues.get(selectedStore.id)}
                      />
                    ) : null}
                    {scopeLabel}
                    <span className="sub">
                      {itemCount(shownCount)}
                      {people
                        ? ` · ${people} ${people === 1 ? 'member' : 'members'}`
                        : ''}
                    </span>
                  </>
                )}
              </span>
              <span className="sp" />
              {view === 'grid' ? sortSelect : null}
              <span className="seg" role="group" aria-label="View">
                <button
                  type="button"
                  className={view === 'list' ? 'on' : ''}
                  aria-pressed={view === 'list'}
                  aria-label="List view"
                  title="List view"
                  onClick={() => changeView('list')}
                >
                  <Icon name="list" />
                </button>
                <button
                  type="button"
                  className={view === 'grid' ? 'on' : ''}
                  aria-pressed={view === 'grid'}
                  aria-label="Grid view"
                  title="Grid view"
                  onClick={() => changeView('grid')}
                >
                  <Icon name="grid" />
                </button>
              </span>
            </div>
            <div
              className="body folder-body"
              ref={bodyRef}
              onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
            >
              {location.kind === 'all' && accessBands.length ? (
                <div className="bandstrip">
                  {accessBands.map((band) => (
                    <Band key={band.key}>{band.text}</Band>
                  ))}
                </div>
              ) : null}
              {expiredHere && selectedStore ? (
                <div className="lnotice" role="status">
                  <Icon name="alert" />
                  <span className="t">
                    <b>Your server session has expired.</b>
                    <p>
                      Items in {selectedStore.name} are unavailable until it
                      checks in again.
                    </p>
                  </span>
                  <Button
                    size="sm"
                    onClick={() => void onResume(selectedStore.id)}
                  >
                    Check in
                  </Button>
                </div>
              ) : null}
              {listBody}
            </div>
            {listShown ? dropStrip : null}
          </section>
        </div>
      </div>
    </div>
  );
}
