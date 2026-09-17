/**
 * The Files browser: a permanent folder tree on the left and the selected
 * folder's contents in the middle. The details panel is a third, permanent
 * column that the shell mounts beside this screen; this file owns the tree
 * and the list only.
 */

import { Fragment, useLayoutEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import { virtualListWindow } from '/kit/virtual-list';
import { Band, Button, Icon, KindIcon } from '../components';
import type { FilterKind } from '../components';
import type { FoksIconName } from '../icons';
import { PageHeader } from '../shell/page-header';
import { NewItemButton, Toolbar } from '../shell/toolbar';
import {
  KINDS,
  defaultCreateStore,
  kindOf,
  nameOf,
  partiesOf,
  peopleGroups,
  prefixOf,
  serverName,
  storeDescription,
  storeDescriptionState,
  storeDisplayOrder,
  storeAvailability,
  storeOf,
  storeReadable,
} from '../model';
import type { Item, Store, AgentSnapshot } from '../model';
import type { KindFilter, LocationStore, LocationState } from '../location';
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
    ? `Items saved in ${store.name} are accessible to all team members according to their permissions. Use New to add passwords or documents.`
    : `Save logins, secure notes, and credentials in ${store.name}.`;
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

interface RowProps {
  snapshot: AgentSnapshot;
  item: Item;
  selected: boolean;
  /** The full path is shown beside the store while a search is running. */
  searching: boolean;
  /** Store pages and grouped rows omit their redundant store subtitle. */
  subtitle?: boolean;
  /** Folder view already supplies the path, so it suppresses the chip. */
  chip?: boolean;
  onSelect: () => void;
}

function Row({
  snapshot,
  item,
  selected,
  searching,
  subtitle = true,
  chip = true,
  onSelect,
}: RowProps): ReactNode {
  const sub = searching
    ? `${whereOf(snapshot, item)} · ${item.path}`
    : subtitle
      ? whereOf(snapshot, item)
      : '';
  return (
    <div
      className={['row', selected ? 'sel' : '', sub ? '' : 'one']
        .filter(Boolean)
        .join(' ')}
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
      <KindIcon kind={kindOf(item) as FilterKind} />
      <span className="name">
        <span className="tt">
          <span>{nameOf(item.path)}</span>
          {chip && !searching ? <PathChip path={item.path} /> : null}
        </span>
        {sub ? <small>{sub}</small> : null}
      </span>
    </div>
  );
}

/** The section header a grouped list sorts items under. */
function TileSection({
  snapshot,
  store,
}: {
  snapshot: AgentSnapshot;
  store: Store;
}): ReactNode {
  if (store.kind === 'team') {
    return (
      <div className="gsec">
        <span>{store.name}</span>
        <span className="n">
          · {peopleGroups(partiesOf(snapshot, store.id))}
        </span>
      </div>
    );
  }
  return (
    <div className="gsec">
      {store.name}
      <span className="n">· {serverName(snapshot, store)}</span>
    </div>
  );
}

interface StoreTree {
  store: Store;
  root: FolderNode;
}

function FolderRow({
  icon,
  name,
  count,
  onSelect,
}: {
  icon: 'folder' | 'vault' | 'people';
  name: string;
  count: number;
  onSelect: () => void;
}): ReactNode {
  return (
    <div
      className="row one folder"
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        onSelect();
      }}
    >
      <span className={`kic ${icon === 'folder' ? 'Folder' : 'Store'}`}>
        <Icon name={icon} />
      </span>
      <span className="name">
        <span className="tt">
          <span>{name}</span>
          <span className="cnt">
            {count} {count === 1 ? 'item' : 'items'}
          </span>
        </span>
      </span>
    </div>
  );
}

function TreeRow({
  depth,
  active,
  root,
  icon,
  name,
  count,
  open,
  expandable,
  onSelect,
  onToggle,
}: {
  depth: number;
  active: boolean;
  root?: boolean;
  icon: 'folder' | 'vault' | 'people' | 'grid';
  name: string;
  count?: number;
  open: boolean;
  expandable: boolean;
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
      {/* The twist sits in the row's flow, in a gutter the stylesheet reserves
          on every row, so a folder with children and one without put their
          icons on the same left edge. Depth indents the row itself. */}
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
        <Icon name={icon} />
        <span className="nm">{name}</span>
        {count === undefined ? null : <span className="c">{count}</span>}
      </button>
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
  const items = scopedItems(snapshot, state);
  // Search flattens the browser to matching results; it must not invalidate
  // the selected folder or change where New saves.
  const folderItems = scopedItems(snapshot, { ...state, query: '' });
  // Every store gets a tree entry, including one with nothing in it yet: the
  // tree is the only way into a store now, so none may be left unreachable.
  const trees: StoreTree[] = storeDisplayOrder(snapshot).map((candidate) => ({
    store: candidate,
    root: folderTree(folderItems.filter((item) => item.store === candidate.id)),
  }));
  const vaultTrees = trees.filter((tree) => tree.store.kind === 'account');
  const teamTrees = trees.filter((tree) => tree.store.kind === 'team');

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
  const storePage = location.kind === 'store';
  const selected = folderSelection(location, state.folder);
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

  const storeRoot = (tree: StoreTree): ReactNode => {
    const foldKey = `${tree.store.id}|/`;
    const open = !closed.has(foldKey);
    return (
      <Fragment key={tree.store.id}>
        <TreeRow
          depth={0}
          root
          active={selected.store === tree.store.id && selected.path === '/'}
          icon={tree.store.kind === 'team' ? 'people' : 'vault'}
          name={tree.store.name}
          open={open}
          expandable={tree.root.folders.length > 0}
          onSelect={() => selectFolder(tree.store, '/')}
          onToggle={() => locations.toggleFolder(foldKey)}
        />
        {open ? drawFolders(tree.root.folders, tree.store, 1) : null}
      </Fragment>
    );
  };

  // The page header names only the open folder; the topbar carries its path.
  const headerTitle =
    selected.store === ALL_ITEMS
      ? 'All items'
      : selectedTree && selectedNode
        ? (selectedNode.path.split('/').filter(Boolean).at(-1) ??
          selectedTree.store.name)
        : 'Files';
  const head = (
    <PageHeader
      ruled
      title={headerTitle}
      query={state.query}
      onQuery={(query) => {
        locations.search(query);
      }}
    />
  );

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
  const rowHeight = state.query || location.kind === 'all' ? 50 : 40;
  const rowWindow = virtualListWindow({
    heights: items.map(() => rowHeight),
    listTop: listMetrics.top,
    scrollTop,
    viewport: listMetrics.viewport,
    overscan: 3,
  });
  const visibleRows = items.slice(rowWindow.start, rowWindow.end);

  const folders = selectedTree ? (selectedNode?.folders ?? []) : [];
  const folderScopedItems = selectedTree ? (selectedNode?.items ?? []) : [];
  const showEmptyFolder =
    Boolean(selectedTree) && !folders.length && !folderScopedItems.length;
  const paneRows = !selectedTree
    ? trees.map((tree) => (
        <FolderRow
          key={tree.store.id}
          icon={tree.store.kind === 'team' ? 'people' : 'vault'}
          name={tree.store.name}
          count={tree.root.count}
          onSelect={() => selectFolder(tree.store, '/')}
        />
      ))
    : [
        ...folders.map((folder) => (
          <FolderRow
            key={folder.path}
            icon="folder"
            name={folder.name}
            count={folder.count}
            onSelect={() => selectFolder(selectedTree.store, folder.path)}
          />
        )),
        ...folderScopedItems.map((item) => (
          <Row
            key={`${item.store}|${item.path}`}
            snapshot={snapshot}
            item={item}
            selected={
              state.selection?.store === item.store &&
              state.selection.path === item.path
            }
            searching={false}
            subtitle={false}
            chip={false}
            onSelect={() =>
              locations.select({ store: item.store, path: item.path })
            }
          />
        )),
      ];

  const vaultHeading = vaultTrees.length === 1 ? 'Your vault' : 'Vaults';

  return (
    <>
      {head}
      <div className="drop-area">
        {dropZone}
        <div className="folder-layout">
          <div className="folder-split">
            <aside className="tpane" aria-label="Folders">
              {!storePage ? <h6>{vaultHeading}</h6> : null}
              {vaultTrees.map(storeRoot)}
              {!storePage && teamTrees.length ? <h6>Teams</h6> : null}
              {!storePage ? teamTrees.map(storeRoot) : null}
              {!storePage ? (
                <>
                  <h6>Everything</h6>
                  <TreeRow
                    depth={0}
                    root
                    active={selected.store === ALL_ITEMS}
                    icon="grid"
                    name="All items"
                    open={false}
                    expandable={false}
                    onSelect={() => locations.setFolder(ALL_ITEMS)}
                    onToggle={() => {}}
                  />
                </>
              ) : null}
            </aside>
            <section className="lpane" aria-label="Folder contents">
              <Toolbar
                onNew={createNew}
                kind={state.kind}
                onKind={(kind) => {
                  locations.setKind(kind);
                }}
                sort={state.sort}
                onSort={(sort) => {
                  locations.setSort(sort);
                }}
                onSettings={
                  selectedTree?.store.kind === 'team'
                    ? () => onSettings(selectedTree.store.id)
                    : undefined
                }
              />
              <div
                className="body folder-body"
                ref={bodyRef}
                onScroll={(event) =>
                  setScrollTop(event.currentTarget.scrollTop)
                }
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
                          No items {store ? `in ${store.name}` : 'here'} match “
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
                            : 'Save logins, secure notes, and credentials to get started.'}
                        </p>
                        <NewItemButton onNew={createNew} />
                      </div>
                    )
                  ) : (
                    <div className="list-window">
                      <div className="hdr">
                        <span />
                        <button
                          type="button"
                          className={state.sort === 'name' ? 'on' : ''}
                          onClick={() => {
                            locations.setSort('name');
                          }}
                        >
                          Name{state.sort === 'name' ? ' ↓' : ''}
                        </button>
                      </div>
                      {location.kind === 'all' &&
                      state.sort === 'group' &&
                      !state.query ? (
                        storeDisplayOrder(snapshot).map((sectionStore) => {
                          const section = items.filter(
                            (item) => item.store === sectionStore.id,
                          );
                          if (!section.length) return null;
                          return (
                            <Fragment key={sectionStore.id}>
                              <TileSection
                                snapshot={snapshot}
                                store={sectionStore}
                              />
                              {section.map((item) => (
                                <Row
                                  key={`${item.store}|${item.path}`}
                                  snapshot={snapshot}
                                  item={item}
                                  searching={false}
                                  subtitle={false}
                                  selected={
                                    state.selection?.store === item.store &&
                                    state.selection.path === item.path
                                  }
                                  onSelect={() =>
                                    locations.select({
                                      store: item.store,
                                      path: item.path,
                                    })
                                  }
                                />
                              ))}
                            </Fragment>
                          );
                        })
                      ) : (
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
                              item={item}
                              searching={Boolean(state.query)}
                              subtitle={location.kind === 'all'}
                              selected={
                                state.selection?.store === item.store &&
                                state.selection.path === item.path
                              }
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
                      )}
                    </div>
                  )
                ) : showEmptyFolder ? (
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
                      {selected.path === '/' && selectedTree
                        ? emptyStoreCopy(selectedTree.store)
                        : 'This folder is empty.'}
                    </p>
                    <NewItemButton onNew={createNew} />
                  </div>
                ) : (
                  <div className="list-window">
                    <div className="hdr">
                      <span />
                      <span>Name</span>
                    </div>
                    <div className="virtual-rows">{paneRows}</div>
                  </div>
                )}
              </div>
            </section>
          </div>
        </div>
      </div>
    </>
  );
}
