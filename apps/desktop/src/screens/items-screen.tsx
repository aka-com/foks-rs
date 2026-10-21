import { FileUploadsPanel } from './file-uploads';
import { itemsReadiness } from '../model';
import { CollectionStatus } from '../components/collection-status';
/**
 * The Files list: the list column of the raised panel. The sidebar's store
 * tree (`shell/files-tree.tsx`) chooses what it shows; the details panel is
 * the column the shell mounts beside it.
 *
 * Rows are the window's 42px list rows: a kind tile, the name over a muted
 * second line, and the size at the trailing edge. All Items and search list
 * every match flat, with the store and folder as the second line; a store or
 * folder lists its subfolders and then its own items, under a head naming
 * it. Search and the New control are in the list bar above; the view menu
 * there switches between rows and cards and sets the sort.
 */

import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import { anyDialogOpen, ContextMenu, Menu } from '/kit/overlay-primitives';
import { useDismissedOnboardingTip } from '../onboarding-tips';
import { virtualListWindow } from '/kit/virtual-list';
import { useFilesView } from '../files-view-pref';
import { filesGridWindow } from './files-grid';
import {
  Band,
  Button,
  EmptyState,
  Icon,
  MenuItem,
  Notice,
} from '../components';
import type { FoksIconName } from '../icons';
import { PageHeader } from '../shell/page-header';
import {
  KINDS,
  catalog,
  defaultCreateStore,
  displayPathComponent,
  fmtSize,
  kindLabel,
  kindOf,
  nameOf,
  partiesOf,
  prefixOf,
  serverDisplayLabelForStore,
  storeDescription,
  storeDescriptionState,
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
import { useFolderMenu } from './folder-menu';
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

/** The tile class and glyph a row draws for its item's kind. */
function itemTile(item: Item): { className: string; icon: FoksIconName } {
  if (kindOf(item) === 'Password') return { className: 'pw', icon: 'key' };
  if (item.kind === 'File') return { className: 'file', icon: 'file' };
  return { className: 'text', icon: 'fileText' };
}

interface RowProps {
  snapshot: AgentSnapshot;
  item: Item;
  /** Rows from several places: the second line names the store and folder. */
  flat: boolean;
  selected: boolean;
  /** A store that can be read but not written: rows are drawn back. */
  readOnly: boolean;
  onSelect: () => void;
  onCopy: (item: Item) => void;
  onReveal: (item: Item) => void;
  onDownload: (item: Item) => void;
  onDelete: (item: Item) => void;
  onMove?: (item: Item) => void;
  accessNow: () => number;
}

function Row({
  snapshot,
  item,
  flat,
  selected,
  readOnly,
  onSelect,
  onCopy,
  onReveal,
  onDownload,
  onDelete,
  onMove,
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
  const name = nameOf(item.path);
  const prefix = prefixOf(item.path);
  const tile = itemTile(item);
  const where = flat
    ? [whereOf(snapshot, item), ...(prefix ? prefix.split('/') : [])].join(
        ' › ',
      )
    : `${kindLabel(kindOf(item) === 'Password' ? 'Password' : 'Document')} · v${item.version}`;
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
        className={['row', 'item', selected ? 'sel' : '', readOnly ? 'ro' : '']
          .filter(Boolean)
          .join(' ')}
        data-row-key={`${item.store}|${item.path}`}
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
        <span className={`tile ${tile.className}`} aria-hidden="true">
          <Icon name={tile.icon} />
        </span>
        <span className="name">
          <b className="nm" title={name}>
            {name}
          </b>
          <small>{where}</small>
        </span>
        <span className="r">{fmtSize(item.size)}</span>
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
            {onMove ? (
              <MenuItem
                reason={writeProblem}
                onClick={choose(() => onMove(item))}
              >
                Rename or move…
              </MenuItem>
            ) : null}
            <MenuItem
              icon="trash"
              danger
              shortcut="⌘⌫"
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

/** A subfolder of the selected folder, listed above its items. */
function FolderRow({
  name,
  count,
  onSelect,
  storeId,
  path,
}: {
  storeId: string;
  path: string;
  name: string;
  count: number;
  onSelect: () => void;
}): ReactNode {
  return (
    <div
      className="row folder"
      data-folder-store={storeId}
      data-folder-path={path}
      data-row-key={`${storeId}|${path}/`}
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
      <span className="tile folder" aria-hidden="true">
        <Icon name="folder" />
      </span>
      <span className="name">
        <b className="nm" title={name}>
          {name}
        </b>
        <small>Folder</small>
      </span>
      <span className="r">{itemCount(count)}</span>
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
  onMove?: (item: Item) => void;
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

/** The window's list row: two lines at 13/1.3 and 12/1.3 with 5px padding. */
const ROW_HEIGHT = 42;

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
  onMove,
  onSettings,
  onTeamInfo,
  onNewFolder,
  onCommandError,
  onUploadDroppedFile,
  dropEnabled = true,
  accessNow = () => Date.now() / 1000,
}: ItemsScreenProps): ReactNode {
  const toasts = useToast();
  const view = useFilesView();
  const bodyRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [listMetrics, setListMetrics] = useState({
    top: 0,
    viewport: 600,
    width: 600,
  });
  // The layouts have different heights; a change of view starts at the top
  // without changing the folder, filters, sorting, or selected item.
  const shownView = useRef(view);
  useEffect(() => {
    if (shownView.current === view) return;
    shownView.current = view;
    if (bodyRef.current) bodyRef.current.scrollTop = 0;
    setScrollTop(0);
  }, [view]);
  const [nextUpDismissed, setNextUpDismissed] =
    useDismissedOnboardingTip('add-password');
  const folderMenu = useFolderMenu(snapshot, {
    onNewFolder,
    onMove,
    onTeamInfo,
    onNavigate: (location) => locations.navigate(location),
    accessNow,
  });
  // The row arrow keys moved to, focused once it has rendered.
  const pendingFocus = useRef<string | null>(null);
  const { location } = state;
  const store =
    location.kind === 'store' ? storeOf(snapshot, location.ref) : undefined;
  const storePage = location.kind === 'store';
  const selected = folderSelection(location, state.folder);
  // A search runs over the sidebar's selection — one store and folder, or
  // every item from All Items — which is what the placeholder promises.
  const items = scopedItems(snapshot, state).filter(
    (item) =>
      selected.store === ALL_ITEMS ||
      (item.store === selected.store && underFolder(item.path, selected.path)),
  );
  const selectedStore =
    selected.store === ALL_ITEMS
      ? undefined
      : (storeOf(snapshot, selected.store) ?? undefined);
  // The selected store's folders, from the whole catalog filtered only by
  // kind, so search never changes where New saves.
  const selectedRoot = selectedStore
    ? folderTree(
        scopedItems(snapshot, {
          ...state,
          query: '',
          location: { kind: 'all' },
        }).filter((item) => item.store === selectedStore.id),
      )
    : undefined;
  const selectedNode = selectedRoot
    ? (folderAt(selectedRoot, selected.path) ?? selectedRoot)
    : undefined;

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

  useEffect(() => {
    const key = pendingFocus.current;
    if (!key) return;
    const row = [
      ...(bodyRef.current?.querySelectorAll<HTMLElement>('[data-row-key]') ??
        []),
    ].find((candidate) => candidate.dataset.rowKey === key);
    if (!row) return;
    pendingFocus.current = null;
    row.focus({ preventScroll: true });
    row.scrollIntoView?.({ block: 'nearest' });
  });

  // Show the unavailable state when a stale link or catalog refresh refers to
  // a store that is no longer present.
  if (location.kind === 'store' && !store) {
    return (
      <>
        <PageHeader title="Vault unavailable" subtitle="" />
        <div className="body">
          <Notice severity="crit" title="Vault no longer available">
            <p>Refresh, or choose another vault from the sidebar.</p>
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
  const availability = (candidate: Store) =>
    storeAvailability(snapshot, candidate, { nowSeconds: accessNow() });
  /** True if the store's check-in lease has expired (read-only). */
  const leaseExpired = (candidate: Store): boolean => {
    const access = availability(candidate);
    return !access.available && access.reason === 'check-in-expired';
  };

  /** Selects `path` in `target`, keeping a store page's address. */
  const selectFolder = (target: Store, path: string): void => {
    const samePage = storePage && target.id === location.ref;
    if (storePage && !samePage) locations.navigate({ kind: 'all' });
    locations.setFolder(folderKey(samePage, target.id, path));
  };

  const createStore =
    selectedStore && storeReadable(snapshot, selectedStore.id)
      ? selectedStore.id
      : store && storeReadable(snapshot, store.id)
        ? store.id
        : (defaultCreateStore(snapshot) ?? '');
  const createFolder =
    selectedStore && selectedNode && selected.path !== '/'
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
        ? selectedStore
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

  // Search and All Items window a flat result set in either layout; a store
  // or folder lists its direct children.
  const flatMode = Boolean(state.query) || selected.store === ALL_ITEMS;
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
    location: selected.store === ALL_ITEMS,
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
    Boolean(selectedStore) && !folders.length && !folderScopedItems.length;

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

  const scopeStore = selectedStore ?? store;
  // Reads work from the local copy while writes do not: the list says so once,
  // above the rows, and draws them back rather than hiding them.
  const readOnlyReason = scopeStore
    ? !availability(scopeStore).available
      ? storeDescription(snapshot, scopeStore)
      : writeBlockReason(snapshot, scopeStore)
    : null;
  const readOnly = Boolean(readOnlyReason);
  const expiredHere = Boolean(scopeStore && leaseExpired(scopeStore));

  const row = (item: Item): ReactNode => (
    <Row
      key={`${item.store}|${item.path}`}
      snapshot={snapshot}
      item={item}
      flat={flatMode}
      selected={
        state.selection?.store === item.store &&
        state.selection.path === item.path
      }
      readOnly={readOnly}
      onSelect={() => locations.select({ store: item.store, path: item.path })}
      onCopy={copyItem}
      onReveal={revealItem}
      onDownload={downloadItem}
      onMove={
        onMove
          ? (item) => {
              if (allowAction(item, true)) onMove(item);
            }
          : undefined
      }
      onDelete={(item) => {
        if (allowAction(item, true)) onDelete(item);
      }}
      accessNow={accessNow}
    />
  );

  const paneRows = selectedStore
    ? [
        ...folders.map((folder) => (
          <FolderRow
            storeId={selectedStore.id}
            path={folder.path}
            key={folder.path}
            name={displayPathComponent(folder.name)}
            count={folder.count}
            onSelect={() => selectFolder(selectedStore, folder.path)}
          />
        )),
        ...folderScopedItems.map(row),
      ]
    : [];

  /**
   * Up and Down move the selection along the rows the list shows, and the
   * focus with it, so the keyboard keeps working after a click.
   */
  const shownItems = flatMode ? items : folderScopedItems;
  const onListKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    if (event.altKey || event.metaKey || event.ctrlKey) return;
    const target = event.target as HTMLElement;
    if (!target.matches('.row')) return;
    const current = shownItems.findIndex(
      (item) =>
        item.store === state.selection?.store &&
        item.path === state.selection.path,
    );
    const next =
      current < 0
        ? 0
        : Math.min(
            Math.max(current + (event.key === 'ArrowDown' ? 1 : -1), 0),
            shownItems.length - 1,
          );
    const item = shownItems[next];
    if (!item) return;
    event.preventDefault();
    pendingFocus.current = `${item.store}|${item.path}`;
    locations.select({ store: item.store, path: item.path });
  };

  /** The current view's name in the list head. */
  const scopeLabel =
    selected.store === ALL_ITEMS
      ? kindMeta
        ? kindMeta.plural
        : 'All Items'
      : selected.path !== '/'
        ? nameOf(selected.path)
        : (selectedStore?.name ?? 'All Items');
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
    <EmptyState
      inList
      icon="search"
      title={`Nothing matches “${state.query}”`}
      body="Try a shorter word, or press ⌘K to search every vault and team."
    />
  );

  const emptyHere = (scope: Store | undefined): ReactNode => (
    <EmptyState
      inList
      icon={kindMeta?.icon ?? 'key'}
      title="No items yet"
      body={
        scope && selected.path === '/'
          ? emptyStoreCopy(scope)
          : scope
            ? 'This folder is empty.'
            : 'Save passwords and documents to get started.'
      }
    />
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
        <button
          type="button"
          className="nt"
          onClick={() => createNew('Password')}
        >
          Next: Try adding a password.
        </button>
        <span className="sp" />
        <button
          type="button"
          className="tbtn x"
          title="Not now"
          aria-label="Not now"
          onClick={() => setNextUpDismissed(true)}
        >
          <Icon name="close" />
        </button>
      </div>
    );

  const itemReadiness = itemsReadiness(
    snapshot,
    selected.store === ALL_ITEMS ? undefined : selected.store,
  );
  const listBody =
    !items.length && itemReadiness !== 'ready' ? (
      <CollectionStatus state={itemReadiness} label="items" />
    ) : flatMode ? (
      !items.length ? (
        state.query ? (
          searchMiss
        ) : vaultEmpty ? (
          starter
        ) : (
          emptyHere(scopeStore)
        )
      ) : (
        <div className="list-window">
          {nextUp}
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
                {visibleRows.map(row)}
              </div>
            ) : (
              visibleRows.map(row)
            )}
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
      vaultEmpty ? (
        starter
      ) : (
        emptyHere(selectedStore)
      )
    ) : (
      <div className="list-window">
        {nextUp}
        <div className={view === 'grid' ? 'files-grid' : 'virtual-rows'}>
          {paneRows}
        </div>
      </div>
    );
  const listShown = flatMode ? items.length > 0 : !showEmptyFolder;

  // The head names a store or folder (with the folders above it), or the
  // search; All Items and the kind rows are named by the sidebar alone.
  const crumbs =
    selectedStore && selected.path !== '/'
      ? [
          { label: selectedStore.name, path: '/' },
          ...selected.path
            .split('/')
            .filter(Boolean)
            .slice(0, -1)
            .map((part, index, parts) => ({
              label: displayPathComponent(part),
              path: `/${parts.slice(0, index + 1).join('/')}`,
            })),
        ]
      : [];
  const listHead = state.query ? (
    <div className="list-head">
      <div className="t">
        <h2>Results for “{state.query}”</h2>
        <p className="search-scope">in {scopeLabel}</p>
      </div>
    </div>
  ) : selectedStore ? (
    <div className="list-head">
      <div className="t">
        {crumbs.length ? (
          <nav className="crumb" aria-label="Folder path">
            {crumbs.map((crumb, index) => (
              <span key={crumb.path} className="crumb-step">
                {index > 0 ? <Icon name="chevronRight" /> : null}
                <button
                  type="button"
                  onClick={() => selectFolder(selectedStore, crumb.path)}
                >
                  {crumb.label}
                </button>
              </span>
            ))}
          </nav>
        ) : null}
        <h2>{scopeLabel}</h2>
        <p>
          {itemReadiness === 'ready'
            ? itemCount(shownCount)
            : `${shownCount} items loaded`}
          {people ? ` · ${people} ${people === 1 ? 'member' : 'members'}` : ''}
          {` · ${serverDisplayLabelForStore(snapshot, selectedStore)}`}
        </p>
      </div>
      {selectedStore.kind === 'team' && selected.path === '/' ? (
        <Button size="sm" onClick={() => onSettings(selectedStore.id)}>
          Manage…
        </Button>
      ) : null}
    </div>
  ) : null;

  return (
    <div className="drop-area">
      {dropZone}
      {folderMenu.menu}
      <section
        className="lpane"
        aria-label="Folder contents"
        onContextMenu={folderMenu.handlers.onContextMenu}
        onKeyDown={folderMenu.handlers.onKeyDown}
      >
        {scopeStore ? (
          <FileUploadsPanel
            key={scopeStore.id}
            bridge={bridge}
            storeId={scopeStore.id}
          />
        ) : null}
        <div
          className="body folder-body rows"
          ref={bodyRef}
          onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
          onKeyDown={onListKeyDown}
        >
          {listHead}
          {location.kind === 'all' && accessBands.length ? (
            <div className="bandstrip">
              {accessBands.map((band) => (
                <Band key={band.key}>{band.text}</Band>
              ))}
            </div>
          ) : null}
          {expiredHere && scopeStore ? (
            <div className="lnotice" role="status">
              <Icon name="alert" />
              <span className="t">
                <b>Your server session has expired.</b>
                <p>
                  Items in {scopeStore.name} are unavailable until it checks in
                  again.
                </p>
              </span>
              <Button size="sm" onClick={() => void onResume(scopeStore.id)}>
                Check in
              </Button>
            </div>
          ) : null}
          {items.length > 0 ? (
            <CollectionStatus state={itemReadiness} label="remaining items" />
          ) : null}
          {listBody}
        </div>
        {listShown ? dropStrip : null}
      </section>
    </div>
  );
}
