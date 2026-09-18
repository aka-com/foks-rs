/**
 * The Files browser: a permanent folder tree on the left and the selected
 * folder's contents in the middle. The details panel is a third, permanent
 * column that the shell mounts beside this screen; this file owns the tree
 * and the list only.
 *
 * The item browser defaults to the "All items" view across all stores,
 * filtering results when a store or folder is selected. Items are presented in
 * a table with Name, Kind, Location (omitted when filtered to a single store),
 * and Size columns, with sortable column headers. The topbar breadcrumb
 * displays the active folder path, omitting a redundant page header.
 */

import { Fragment, useLayoutEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import { virtualListWindow } from '/kit/virtual-list';
import { Band, Button, Icon, KindIcon } from '../components';
import type { FilterKind } from '../components';
import type { FoksIconName } from '../icons';
import { NewItemButton, Toolbar } from '../shell/toolbar';
import {
  KINDS,
  defaultCreateStore,
  fmtSize,
  initials,
  kindLabel,
  kindOf,
  nameOf,
  prefixOf,
  storeDescription,
  storeDescriptionState,
  storeDisplayOrder,
  storeAvailability,
  storeHues,
  storeNavigationOrder,
  storeOf,
  storeReadable,
} from '../model';
import type { Item, Store, AgentSnapshot } from '../model';
import type {
  KindFilter,
  LocationStore,
  LocationState,
  SortKey,
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
import { StoreAccessTakeover, storeAccessBands } from './store-access';

/* --------------------------------------------------------------- pieces -- */

function emptyStoreCopy(store: Store): string {
  return store.kind === 'team'
    ? 'Items stored in this team vault are accessible to team members according to their assigned roles.'
    : 'Passwords and documents you add here are private to this vault.';
}

function PathChip({ path }: { path: string }): ReactNode {
  const prefix = prefixOf(path);
  return prefix ? (
    <span className="pchip" title={`In /${prefix}`}>
      <Icon name="folder" />
      {prefix}
    </span>
  ) : null;
}

/**
 * Store badge displayed in item table rows. Teams display their initials with
 * their assigned theme color; personal vaults display a standard vault icon.
 */
function StoreMark({
  store,
  hue,
}: {
  store: Store;
  hue: string | undefined;
}): ReactNode {
  return store.kind === 'team' ? (
    <span className="av team" style={{ background: hue }} aria-hidden="true">
      {initials(store.name)}
    </span>
  ) : (
    <Icon name="vault" />
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
  onSort,
}: {
  columns: Columns;
  sort: SortKey;
  onSort: (sort: SortKey) => void;
}): ReactNode {
  // Within one store, ordering by store is ordering by name; the Name header
  // says so while the Location header is not drawn.
  const shown = !columns.location && sort === 'group' ? 'name' : sort;
  const head = (key: SortKey, label: string): ReactNode => (
    <button
      type="button"
      className={shown === key ? 'on' : ''}
      aria-pressed={shown === key}
      onClick={() => onSort(key)}
    >
      {label}
      {shown === key ? ' ↓' : ''}
    </button>
  );
  return (
    <div className={`hdr ${columnClass(columns)}`}>
      <span />
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
  /** The folder view already names the path, so it suppresses the chip. */
  chip: boolean;
  onSelect: () => void;
}

function Row({
  snapshot,
  hues,
  item,
  columns,
  selected,
  chip,
  onSelect,
}: RowProps): ReactNode {
  const kind = kindOf(item) as FilterKind;
  const store = storeOf(snapshot, item.store);
  return (
    <div
      className={columnClass(columns, 'row', 'one', selected ? 'sel' : '')}
      role="button"
      tabIndex={0}
      aria-pressed={selected}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        onSelect();
      }}
    >
      <KindIcon kind={kind} />
      <span className="name">
        <span className="tt">
          <span>{nameOf(item.path)}</span>
          {chip ? <PathChip path={item.path} /> : null}
        </span>
      </span>
      <span className="cell kind">{kindLabel(kind)}</span>
      {columns.location ? (
        <span className="cell mark">
          {store ? <StoreMark store={store} hue={hues.get(store.id)} /> : null}
          <span>{whereOf(snapshot, item)}</span>
        </span>
      ) : null}
      <span className="cell num">
        {item.size === null ? '' : fmtSize(item.size)}
      </span>
    </div>
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
}: {
  name: string;
  count: number;
  columns: Columns;
  onSelect: () => void;
}): ReactNode {
  return (
    <div
      className={columnClass(columns, 'row', 'one', 'folder')}
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        onSelect();
      }}
    >
      <span className="kic Folder">
        <Icon name="folder" />
      </span>
      <span className="name">
        <span className="tt">
          <span>{name}</span>
        </span>
      </span>
      <span className="cell kind">Folder</span>
      {columns.location ? <span className="cell" /> : null}
      <span className="cell num">
        {count} {count === 1 ? 'item' : 'items'}
      </span>
    </div>
  );
}

function TreeRow({
  depth,
  active,
  root,
  icon,
  mark,
  name,
  count,
  open,
  expandable,
  action,
  onSelect,
  onToggle,
}: {
  depth: number;
  active: boolean;
  root?: boolean;
  icon: 'folder' | 'vault' | 'grid';
  /** Drawn in place of `icon` when the row names a team. */
  mark?: ReactNode;
  name: string;
  /** Omitted, not drawn as 0, when there is nothing to count. */
  count?: number;
  open: boolean;
  expandable: boolean;
  /** A control that belongs to the row, drawn after its name. */
  action?: ReactNode;
  onSelect: () => void;
  onToggle: () => void;
}): ReactNode {
  return (
    <div
      className={['fn', active ? 'on' : '', root ? 'root' : '']
        .filter(Boolean)
        .join(' ')}
      style={{ '--d': depth } as React.CSSProperties}
    >
      {/* Store root rows cannot be collapsed and have no expand toggle, so
          their icons align flush with the left edge. Nested expandable folders
          place the disclosure toggle within the indent column. */}
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
          <Icon name="chev" />
        </button>
      ) : null}
      <button
        type="button"
        className="fselect"
        aria-current={active ? 'location' : undefined}
        onClick={onSelect}
      >
        {mark ?? <Icon name={icon} />}
        <span className="nm">{name}</span>
        {count ? <span className="c">{count}</span> : null}
      </button>
      {action}
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
            ? 'Encrypting and saving the file.'
            : (target.blocked ??
              `${folder ? `Saved in ${folder}` : 'Saved at the top level'} · one file at a time`)}
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
  /** Unused now that reveal is details-panel-only; kept for the caller's wiring. */
  onReveal: (item: Item) => void;
  onNew: (kind: NewKind, storeId: string, folder?: string) => void;
  onResume: (storeId: string) => Promise<void>;
  /** Unused now that delete is details-panel-only; kept for the caller's wiring. */
  onDelete: (item: Item) => void;
  onSettings: (storeId: string) => void;
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
  onNew,
  onResume,
  onSettings,
  onUploadDroppedFile,
  dropEnabled = true,
  accessNow = () => Date.now() / 1000,
}: ItemsScreenProps): ReactNode {
  const bodyRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [listMetrics, setListMetrics] = useState({ top: 0, viewport: 600 });
  const { location } = state;
  const store =
    location.kind === 'store' ? storeOf(snapshot, location.ref) : undefined;
  // The account the rail header currently names, so a store's own heading
  // does not repeat a server the rail already states.
  const activeAccountRef = locations.getAccount();
  const activeAccount = activeAccountRef
    ? storeOf(snapshot, activeAccountRef)
    : undefined;
  const storePage = location.kind === 'store';
  const selected = folderSelection(location, state.folder);
  // Search flattens the browser to matching results; it must not invalidate
  // the selected folder or change where New saves.
  const folderItems = scopedItems(snapshot, { ...state, query: '' });
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
    root: folderTree(folderItems.filter((item) => item.store === candidate.id)),
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
      });
    };
    measure();
    const observer =
      typeof ResizeObserver === 'undefined'
        ? null
        : new ResizeObserver(measure);
    observer?.observe(body);
    return () => observer?.disconnect();
  }, [items.length, state.kind, state.query, state.folder, location.kind]);

  if (store && storeDescriptionState(snapshot, store) !== 'normal') {
    return (
      <StoreAccessTakeover
        snapshot={snapshot}
        store={store}
        activeAccount={activeAccount}
        onOpenServer={(profile) =>
          locations.navigate({ kind: 'settings', section: 'servers', profile })
        }
        onFinishSetup={() => void onResume(store.id)}
        headerAction={
          store.kind === 'team' ? (
            <Button
              variant="quiet"
              icon="gear"
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

  /**
   * Moves the browser to `path` in `target`. A `store` location is pinned to
   * one store; clicking a *different* store while pinned leaves that page for
   * the broad tree first, so every store the tree lists stays reachable from
   * it, even from a deep link that opened on just one of them.
   */
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
            depth={depth}
            active={
              selected.store === treeStore.id && selected.path === folder.path
            }
            icon="folder"
            name={folder.name}
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

  // Store root rows cannot be collapsed and have no expand toggle; their icons
  // align flush with the left container edge.
  const storeRoot = (tree: StoreTree): ReactNode => {
    const active = selected.store === tree.store.id && selected.path === '/';
    return (
      <Fragment key={tree.store.id}>
        <TreeRow
          depth={0}
          root
          active={active}
          icon="vault"
          mark={
            tree.store.kind === 'team' ? (
              <StoreMark store={tree.store} hue={hues.get(tree.store.id)} />
            ) : undefined
          }
          name={tree.store.name}
          count={tree.root.count}
          open
          expandable={false}
          action={
            active && tree.store.kind === 'team' ? (
              <button
                type="button"
                className="fact"
                title="Team settings"
                aria-label="Team settings"
                onClick={(event) => {
                  event.stopPropagation();
                  onSettings(tree.store.id);
                }}
              >
                <Icon name="gear" />
              </button>
            ) : undefined
          }
          onSelect={() => selectFolder(tree.store, '/')}
          onToggle={() => {}}
        />
        {drawFolders(tree.root.folders, tree.store, 1)}
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
    return Boolean(
      candidate &&
      storeAvailability(snapshot, candidate, {
        nowSeconds: accessNow(),
      }).available,
    );
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

  // Search and the All items view use a flat virtualized list; folder views
  // display direct children.
  const flatMode = Boolean(state.query) || selected.store === ALL_ITEMS;
  // Location is a column only when rows span multiple stores.
  const columns: Columns = { location: selected.store === ALL_ITEMS };
  const rowWindow = virtualListWindow({
    heights: items.map(() => ROW_HEIGHT),
    listTop: listMetrics.top,
    scrollTop,
    viewport: listMetrics.viewport,
    overscan: 3,
  });
  const visibleRows = items.slice(rowWindow.start, rowWindow.end);

  const folders = selectedNode?.folders ?? [];
  const folderScopedItems = selectedNode?.items ?? [];
  const showEmptyFolder =
    Boolean(selectedTree) && !folders.length && !folderScopedItems.length;
  const paneRows = selectedTree
    ? [
        ...folders.map((folder) => (
          <FolderRow
            key={folder.path}
            name={folder.name}
            count={folder.count}
            columns={columns}
            onSelect={() => selectFolder(selectedTree.store, folder.path)}
          />
        )),
        ...folderScopedItems.map((item) => (
          <Row
            key={`${item.store}|${item.path}`}
            snapshot={snapshot}
            hues={hues}
            item={item}
            columns={columns}
            selected={
              state.selection?.store === item.store &&
              state.selection.path === item.path
            }
            chip={false}
            onSelect={() =>
              locations.select({ store: item.store, path: item.path })
            }
          />
        )),
      ]
    : [];

  const vaultHeading = vaultTrees.length === 1 ? 'Your vault' : 'Vaults';
  const searchPlaceholder =
    selected.store === ALL_ITEMS
      ? 'Search all items'
      : selected.path !== '/'
        ? 'Search this folder'
        : selectedTree?.store.kind === 'team'
          ? 'Search this team'
          : 'Search this vault';
  const searchScope = selectedTree?.store.name ?? store?.name;
  const allCount = folderItems.length;

  return (
    <div className="drop-area">
      {dropZone}
      <div className="folder-layout">
        {/* The toolbar spans both columns: the kind filter sits over the tree
            whose counts it changes, search and New over the list. */}
        <Toolbar
          onNew={createNew}
          kind={state.kind}
          onKind={(kind) => {
            locations.setKind(kind);
          }}
          query={state.query}
          onQuery={(query) => {
            locations.search(query);
          }}
          searchPlaceholder={searchPlaceholder}
        />
        <div className="folder-split">
          <aside className="tpane" aria-label="Folders">
            {!storePage ? (
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
                  onSelect={() => locations.setFolder(ALL_ITEMS)}
                  onToggle={() => {}}
                />
              </div>
            ) : null}
            {!storePage ? <h6>{vaultHeading}</h6> : null}
            {vaultTrees.map(storeRoot)}
            {!storePage && teamTrees.length ? <h6>Teams</h6> : null}
            {!storePage ? teamTrees.map(storeRoot) : null}
          </aside>
          <section className="lpane" aria-label="Folder contents">
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
              {flatMode ? (
                !items.length ? (
                  state.query ? (
                    <div className="empty">
                      <h2>
                        No items{' '}
                        {searchScope ? `in ${searchScope}` : 'anywhere'} match “
                        {state.query}”
                      </h2>
                      <p>
                        Search by item name, path, or vault. Item contents are
                        encrypted and cannot be searched.
                      </p>
                    </div>
                  ) : (
                    <div className="empty">
                      <div className="big">
                        <Icon
                          name={
                            kindMeta ? (kindMeta.icon as FoksIconName) : 'key'
                          }
                        />
                      </div>
                      <h2>No items yet</h2>
                      <p>
                        {store
                          ? emptyStoreCopy(store)
                          : 'Save passwords and documents to get started.'}
                      </p>
                      <NewItemButton onNew={createNew} />
                    </div>
                  )
                ) : (
                  <div className="list-window">
                    <ListHeader
                      columns={columns}
                      sort={state.sort}
                      onSort={(sort) => {
                        locations.setSort(sort);
                      }}
                    />
                    <div className="virtual-rows" ref={listRef}>
                      {rowWindow.padTop ? (
                        <div
                          className="virtual-spacer"
                          style={{ height: rowWindow.padTop }}
                        />
                      ) : null}
                      {visibleRows.map((item) => (
                        <Row
                          key={`${item.store}|${item.path}`}
                          snapshot={snapshot}
                          hues={hues}
                          item={item}
                          columns={columns}
                          selected={
                            state.selection?.store === item.store &&
                            state.selection.path === item.path
                          }
                          chip
                          onSelect={() => {
                            locations.select({
                              store: item.store,
                              path: item.path,
                            });
                          }}
                        />
                      ))}
                      {rowWindow.padBottom ? (
                        <div
                          className="virtual-spacer"
                          style={{ height: rowWindow.padBottom }}
                        />
                      ) : null}
                    </div>
                  </div>
                )
              ) : showEmptyFolder ? (
                <div className="empty">
                  <div className="big">
                    <Icon
                      name={kindMeta ? (kindMeta.icon as FoksIconName) : 'key'}
                    />
                  </div>
                  <h2>No items yet</h2>
                  <p>
                    {selected.path === '/' && selectedTree
                      ? emptyStoreCopy(selectedTree.store)
                      : 'This folder is empty.'}
                  </p>
                  <NewItemButton onNew={createNew} />
                </div>
              ) : (
                <div className="list-window">
                  <ListHeader
                    columns={columns}
                    sort={state.sort}
                    onSort={(sort) => {
                      locations.setSort(sort);
                    }}
                  />
                  <div className="virtual-rows">{paneRows}</div>
                </div>
              )}
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}
