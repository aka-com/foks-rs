/**
 * Main vault items view, supporting list and grid card layouts with filtering and search.
 */

import { Fragment, useLayoutEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import { virtualListWindow } from '/kit/virtual-list';
import { Band, Button, Icon, KindGlyph, KindIcon } from '../components';
import type { FilterKind } from '../components';
import type { FoksIconName } from '../icons';
import { PageHeader, headerFor } from '../shell/page-header';
import { NewItemButton, Toolbar } from '../shell/toolbar';
import {
  KINDS,
  canChangeItem,
  catalog,
  defaultCreateStore,
  fmtSize,
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
import type { LocationStore, LocationState } from '../location';
import { normalizeCommandError } from '../bridge';
import type { Bridge, ItemRequest } from '../bridge';
import { useFileDrop } from '../file-drop';
import { fileDropPath, writeBlockReason } from './write-workflows';
import type { NewKind } from './write-workflows';
import { folderAt, folderTree, scopedItems, whereOf } from './scope';
import type { FolderNode } from './scope';
import { StoreAccessTakeover, storeAccessBands } from './store-access';

/** Maximum number of cards rendered in grid view before filtering is required. */
const GRID_CAP = 200;

/* --------------------------------------------------------------- pieces -- */

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

interface ItemActionProps {
  item: Item;
  onReveal: () => void;
  onCopyValue: () => void;
  onDownload: () => void;
  onOpen: () => void;
  onDelete: () => void;
  deleteDisabled: boolean;
}

function ItemActions({
  item,
  onReveal,
  onCopyValue,
  onDownload,
  onOpen,
  onDelete,
  deleteDisabled,
}: ItemActionProps): ReactNode {
  const kind = kindOf(item);
  const action = (
    label: string,
    icon: FoksIconName,
    run: () => void,
    danger = false,
    disabled = false,
  ) => (
    <button
      type="button"
      className={danger ? 'danger' : undefined}
      title={label}
      aria-label={label}
      disabled={disabled}
      onClick={(event) => {
        event.stopPropagation();
        run();
      }}
    >
      <Icon name={icon} />
    </button>
  );
  return (
    <span className="acts">
      {kind === 'Password' || kind === 'Resource' ? (
        <>
          {action(
            kind === 'Password' ? 'Show password' : 'Show value',
            'eye',
            onReveal,
          )}
          {action(
            kind === 'Password' ? 'Copy password' : 'Copy value',
            'copy',
            onCopyValue,
          )}
        </>
      ) : kind === 'File' ? (
        action('Download', 'download', onDownload)
      ) : (
        action('Open linked item', 'arrow', onOpen)
      )}
      {action(
        deleteDisabled
          ? 'You do not have permission to delete this item'
          : 'Delete this item',
        'trash',
        onDelete,
        true,
        deleteDisabled,
      )}
    </span>
  );
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

function Tile({
  item,
  selected,
  onSelect,
  onReveal,
  onCopyValue,
  onDownload,
  onOpen,
  onDelete,
  deleteDisabled,
}: Omit<RowProps, 'searching' | 'snapshot'> & ItemActionProps): ReactNode {
  const kind = kindOf(item);
  return (
    <div
      className={selected ? 'tile sel' : 'tile'}
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
      <div className="glyph">
        <KindGlyph item={item} size="big" />
        <span className="qa">
          <ItemActions
            {...{
              item,
              onReveal,
              onCopyValue,
              onDownload,
              onOpen,
              onDelete,
              deleteDisabled,
            }}
          />
        </span>
      </div>
      <div className="cap">
        <div className="nm">{nameOf(item.path)}</div>
        <div className="sub">
          {kind === 'Link' ? (
            <>
              <PathChip path={item.path} /> Link
            </>
          ) : kind === 'File' ? (
            <>
              <PathChip path={item.path} /> {fmtSize(item.size)}
            </>
          ) : prefixOf(item.path) ? (
            <PathChip path={item.path} />
          ) : (
            <span className="dim">Root</span>
          )}
        </div>
      </div>
    </div>
  );
}

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

function folderKey(storePage: boolean, store: string, path: string): string {
  if (storePage) return path === '/' ? '' : path;
  return `${store}|${path}`;
}

function folderSelection(
  location: LocationState['location'],
  value: string,
): { store: string | null; path: string } {
  if (location.kind === 'store') {
    return { store: location.ref, path: value || '/' };
  }
  if (!value) return { store: null, path: '/' };
  const cut = value.indexOf('|');
  return cut > 0
    ? { store: value.slice(0, cut), path: value.slice(cut + 1) || '/' }
    : { store: null, path: '/' };
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
  icon: 'folder' | 'vault' | 'people';
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
      {expandable ? (
        <button
          type="button"
          className={open ? 'twist open' : 'twist'}
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

function FolderBrowser({
  snapshot,
  state,
  trees,
  locations,
}: {
  snapshot: AgentSnapshot;
  state: LocationState;
  trees: readonly StoreTree[];
  locations: LocationStore;
}): ReactNode {
  const storePage = state.location.kind === 'store';
  const selected = folderSelection(state.location, state.folder);
  const selectedTree = trees.find((tree) => tree.store.id === selected.store);
  const selectedNode = selectedTree
    ? (folderAt(selectedTree.root, selected.path) ?? selectedTree.root)
    : null;
  const closed = new Set(state.closedFolders);

  const drawFolders = (
    folders: readonly FolderNode[],
    store: Store,
    depth: number,
  ): ReactNode =>
    folders.map((folder) => {
      const foldKey = `${store.id}|${folder.path}`;
      const open = !closed.has(foldKey);
      return (
        <Fragment key={foldKey}>
          <TreeRow
            depth={depth}
            active={
              selected.store === store.id && selected.path === folder.path
            }
            icon="folder"
            name={folder.name}
            count={folder.count}
            open={open}
            expandable={folder.folders.length > 0}
            onSelect={() =>
              locations.setFolder(folderKey(storePage, store.id, folder.path))
            }
            onToggle={() => locations.toggleFolder(foldKey)}
          />
          {open ? drawFolders(folder.folders, store, depth + 1) : null}
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
          onSelect={() =>
            locations.setFolder(folderKey(storePage, tree.store.id, '/'))
          }
          onToggle={() => locations.toggleFolder(foldKey)}
        />
        {open ? drawFolders(tree.root.folders, tree.store, 1) : null}
      </Fragment>
    );
  };

  const selectFolder = (store: Store, path: string): void =>
    locations.setFolder(folderKey(storePage, store.id, path));
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
        ...selectedNode!.folders.map((folder) => (
          <FolderRow
            key={folder.path}
            icon="folder"
            name={folder.name}
            count={folder.count}
            onSelect={() => selectFolder(selectedTree.store, folder.path)}
          />
        )),
        ...selectedNode!.items.map((item) => (
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

  const crumbs: { label: string; path: string; store: Store }[] = [];
  if (selectedTree) {
    crumbs.push({
      label: selectedTree.store.name,
      path: '/',
      store: selectedTree.store,
    });
    let path = '';
    for (const part of selectedNode!.path.split('/').filter(Boolean)) {
      path += `/${part}`;
      crumbs.push({ label: part, path, store: selectedTree.store });
    }
  }

  return (
    <div className="folder-split">
      <aside className="tpane" aria-label="Folders">
        {!storePage ? <h6>Vaults</h6> : null}
        {trees.map(storeRoot)}
      </aside>
      <section className="lpane" aria-label="Folder contents">
        <div className="lhead">
          <nav className="crumbs" aria-label="Current folder">
            {!storePage ? (
              <button type="button" onClick={() => locations.setFolder('')}>
                All items
              </button>
            ) : null}
            {crumbs.map((crumb, index) => (
              <Fragment key={`${crumb.store.id}|${crumb.path}`}>
                {index || !storePage ? (
                  <span className="sep">
                    <Icon name="chev" />
                  </span>
                ) : null}
                {index === crumbs.length - 1 ? (
                  <span className="cur">{crumb.label}</span>
                ) : (
                  <button
                    type="button"
                    onClick={() => selectFolder(crumb.store, crumb.path)}
                  >
                    {crumb.label}
                  </button>
                )}
              </Fragment>
            ))}
          </nav>
        </div>
        <div className="body folder-body">
          <div className="list-window">
            <div className="hdr">
              <span />
              <span>Name</span>
            </div>
            <div className="virtual-rows">{paneRows}</div>
          </div>
        </div>
      </section>
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
              `Saved in ${folder ?? '/documents'} · one file at a time`)}
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
  onReveal: (item: Item) => void;
  onNew: (kind: NewKind, storeId: string, folder?: string) => void;
  onResume: (storeId: string) => Promise<void>;
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
  onReveal,
  onNew,
  onResume,
  onDelete,
  onSettings,
  onCommandError,
  onUploadDroppedFile,
  dropEnabled = true,
  accessNow = () => Date.now() / 1000,
}: ItemsScreenProps): ReactNode {
  const toasts = useToast();
  const bodyRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [listMetrics, setListMetrics] = useState({ top: 0, viewport: 600 });
  const { location } = state;
  const store =
    location.kind === 'store' ? storeOf(snapshot, location.ref) : undefined;
  const header = headerFor(snapshot, location);
  const head = (
    <PageHeader
      {...header}
      back={{
        label: 'Files home',
        onBack: () => {
          locations.navigate({ kind: 'files' });
        },
      }}
      query={state.query}
      onQuery={(query) => {
        locations.search(query);
      }}
    />
  );
  const items = scopedItems(snapshot, state);
  // Search temporarily replaces the browser with flat results; it must not
  // invalidate the selected folder or change where New saves.
  const folderItems = scopedItems(snapshot, { ...state, query: '' });
  const trees: StoreTree[] = storeDisplayOrder(snapshot)
    .map((candidate) => ({
      store: candidate,
      root: folderTree(
        folderItems.filter((item) => item.store === candidate.id),
      ),
    }))
    .filter((tree) => tree.root.count > 0);

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
  }, [items.length, state.view, state.kind, state.query, location.kind]);

  if (store && storeDescriptionState(snapshot, store) !== 'normal') {
    return (
      <StoreAccessTakeover
        back={{
          label: 'Files home',
          onBack: () => locations.navigate({ kind: 'files' }),
        }}
        snapshot={snapshot}
        store={store}
        onOpenServer={(profile) =>
          locations.navigate({ kind: 'settings', section: 'servers', profile })
        }
        onFinishSetup={() => void onResume(store.id)}
        headerAction={
          store.kind === 'team' ? (
            <Button
              variant="quiet"
              icon="gear"
              title="Group settings"
              aria-label="Group settings"
              onClick={() => onSettings(store.id)}
            />
          ) : undefined
        }
      />
    );
  }

  const accessBands = storeAccessBands(snapshot);
  const kindMeta = state.kind === 'All' ? null : KINDS[state.kind];
  // Default to the current store if readable; otherwise, fall back to the first available vault.
  const selectedFolder = folderSelection(location, state.folder);
  const selectedTree = trees.find(
    (candidate) => candidate.store.id === selectedFolder.store,
  );
  const selectedNode = selectedTree
    ? folderAt(selectedTree.root, selectedFolder.path)
    : undefined;
  const createStore =
    selectedTree && storeReadable(snapshot, selectedTree.store.id)
      ? selectedTree.store.id
      : store && storeReadable(snapshot, store.id)
        ? store.id
        : (defaultCreateStore(snapshot) ?? '');
  const createFolder =
    state.view === 'folders' &&
    selectedTree &&
    selectedNode &&
    selectedFolder.path !== '/'
      ? selectedFolder.path
      : undefined;
  const requestOf = (item: Item): ItemRequest => ({
    storeId: item.store,
    path: item.path,
    version: item.version,
  });
  const report = (error: unknown): void => {
    if (normalizeCommandError(error).code === 'agent-lost')
      onCommandError(error);
    else toasts.show(normalizeCommandError(error).message);
  };
  const currentStoreAvailable = (storeId: string): boolean => {
    const candidate = storeOf(snapshot, storeId);
    return Boolean(
      candidate &&
      storeAvailability(snapshot, candidate, {
        nowSeconds: accessNow(),
      }).available,
    );
  };
  const accessAvailable = (item: Item): boolean =>
    currentStoreAvailable(item.store);
  const createNew = (itemKind: Exclude<KindFilter, 'All'>): void => {
    if (createStore && currentStoreAvailable(createStore))
      onNew(itemKind, createStore, createFolder);
  };
  // Dropped files are uploaded to the active store or selected folder.
  // Multi-store views disable drops because no target store is selected.
  const dropStore =
    location.kind === 'store'
      ? store
      : state.view === 'folders' && !state.query
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
  const copyValue = (item: Item): void => {
    if (!accessAvailable(item)) return;
    void bridge
      .copyItemValue(requestOf(item))
      .then(
        () =>
          toasts.show(
            `${kindOf(item) === 'Password' ? 'Password' : 'Value'} copied`,
          ),
        report,
      );
  };
  const download = (item: Item): void => {
    if (!accessAvailable(item)) return;
    locations.select({ store: item.store, path: item.path });
    void bridge
      .downloadFile(requestOf(item))
      .then(
        ({ saved }) =>
          toasts.show(
            saved
              ? `Downloaded ${nameOf(item.path)} (version ${item.version})`
              : 'Download cancelled',
          ),
        report,
      );
  };
  const openLink = (item: Item): void => {
    if (!accessAvailable(item)) return;
    void bridge
      .readItem(requestOf(item))
      .then((response) => {
        if (!accessAvailable(item)) return;
        if (
          response.store !== item.store ||
          response.path !== item.path ||
          response.version !== item.version
        ) {
          throw new Error('Could not verify link destination.');
        }
        // Resolve against the catalog to ensure the target is an item and not a folder.
        const target = catalog(snapshot).find(
          (candidate) =>
            candidate.store === item.store && candidate.path === response.value,
        );
        if (target)
          locations.select({ store: target.store, path: target.path });
        else toasts.show(`Linked item not found: ${response.value}`);
      }, report)
      .catch(report);
  };
  const reveal = (item: Item): void => {
    if (!accessAvailable(item)) return;
    locations.select({ store: item.store, path: item.path });
    // Pass revealed item to details panel for display.
    onReveal(item);
  };

  const rowHeight = state.query || location.kind === 'all' ? 50 : 40;
  const rowWindow = virtualListWindow({
    heights: items.map(() => rowHeight),
    listTop: listMetrics.top,
    scrollTop,
    viewport: listMetrics.viewport,
    overscan: 3,
  });
  const visibleRows = items.slice(rowWindow.start, rowWindow.end);
  const gridItems = items.slice(0, GRID_CAP);
  const callbacks = (item: Item) => ({
    onReveal: () => reveal(item),
    onCopyValue: () => copyValue(item),
    onDownload: () => download(item),
    onOpen: () => openLink(item),
    onDelete: () => {
      if (!accessAvailable(item)) return;
      locations.select({ store: item.store, path: item.path });
      // Confirm deletion for the selected item version.
      onDelete(item);
    },
    deleteDisabled:
      !storeReadable(snapshot, item.store) || !canChangeItem(snapshot, item),
  });

  return (
    <>
      {head}
      <Toolbar
        onNew={(itemKind) => {
          if (createStore && currentStoreAvailable(createStore))
            onNew(itemKind, createStore, createFolder);
        }}
        kind={state.kind}
        onKind={(kind) => {
          locations.setKind(kind);
        }}
        sort={state.sort}
        onSort={(sort) => {
          locations.setSort(sort);
        }}
        view={state.view}
        onView={(view) => {
          locations.setView(view);
        }}
        details={state.details}
        onDetails={(open) => {
          locations.setDetails(open);
        }}
        onSettings={
          store?.kind === 'team' ? () => onSettings(store.id) : undefined
        }
      />
      <div className="drop-area">
        {dropZone}
        {state.view === 'folders' && !state.query && items.length ? (
          <div className="folder-layout">
            {location.kind === 'all' && accessBands.length ? (
              <div className="bandstrip">
                {accessBands.map((band) => (
                  <Band key={band.key}>{band.text}</Band>
                ))}
              </div>
            ) : null}
            <FolderBrowser
              snapshot={snapshot}
              state={state}
              trees={trees}
              locations={locations}
            />
          </div>
        ) : (
          <div
            className="body"
            ref={bodyRef}
            onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
          >
            {location.kind === 'all'
              ? accessBands.map((band) => (
                  <Band key={band.key}>{band.text}</Band>
                ))
              : null}
            {state.view === 'grid' && items.length > GRID_CAP ? (
              <Band>
                Showing the first {GRID_CAP} cards. Use search or filters to
                view more items.
              </Band>
            ) : null}

            {!items.length ? (
              state.query ? (
                <div className="empty">
                  <h2>
                    No items {store ? `in ${store.name}` : 'here'} match “
                    {state.query}”
                  </h2>
                  <p>
                    Search covers item names, paths, and vaults. Item contents
                    are encrypted and not searched.
                  </p>
                </div>
              ) : (
                <div className="empty">
                  <div className="big">
                    <Icon
                      name={kindMeta ? (kindMeta.icon as FoksIconName) : 'key'}
                    />
                  </div>
                  <h2>No items yet</h2>
                  <p>
                    Save logins, secure notes, and credentials in this vault.
                  </p>
                  <NewItemButton
                    onNew={(itemKind) => {
                      if (createStore && currentStoreAvailable(createStore))
                        onNew(itemKind, createStore, createFolder);
                    }}
                  />
                </div>
              )
            ) : state.view !== 'grid' ? (
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
                        <TileSection snapshot={snapshot} store={sectionStore} />
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
            ) : location.kind === 'all' ? (
              storeDisplayOrder(snapshot).map((sectionStore) => {
                const id = sectionStore.id;
                const section = gridItems.filter((item) => item.store === id);
                if (!section.length || !sectionStore) return null;
                return (
                  <Fragment key={id}>
                    <TileSection snapshot={snapshot} store={sectionStore} />
                    <div className="tiles">
                      {section.map((item) => (
                        <Tile
                          key={`${item.store}|${item.path}`}
                          item={item}
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
                          {...callbacks(item)}
                        />
                      ))}
                    </div>
                  </Fragment>
                );
              })
            ) : (
              <>
                <div className="gsec first">
                  {kindMeta ? kindMeta.plural : 'All items'}
                  <span className="n">· {items.length}</span>
                </div>
                <div className="tiles">
                  {gridItems.map((item) => (
                    <Tile
                      key={`${item.store}|${item.path}`}
                      item={item}
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
                      {...callbacks(item)}
                    />
                  ))}
                </div>
              </>
            )}
          </div>
        )}
      </div>
    </>
  );
}
