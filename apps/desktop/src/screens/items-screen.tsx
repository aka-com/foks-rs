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
import type { KeyboardEvent, MouseEvent, ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import { commandKey } from '/kit/keys';
import { anyDialogOpen, ContextMenu, Menu } from '/kit/overlay-primitives';
import { useDismissedOnboardingTip } from '../onboarding-tips';
import { virtualListWindow } from '/kit/virtual-list';
import { useFilesView } from '../files-view-pref';
import {
  FILES_CARD_GAP,
  FILES_CARD_HEIGHT,
  FILES_CARD_HEIGHT_WITH_LOCATION,
  filesGridWindow,
} from './files-grid';
import {
  Button,
  EmptyState,
  Icon,
  MenuItem,
  Notice,
  useListbox,
} from '../components';
import type { ListboxOptionProps } from '../components';
import type { FoksIconName } from '../icons';
import {
  KINDS,
  catalog,
  catalogFolders,
  defaultCreateStore,
  displayPathComponent,
  fmtSize,
  kindLabel,
  kindOf,
  nameOf,
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
import type { FolderNode } from './scope';
import { useFolderMenu } from './folder-menu';
import { itemTile } from './item-tile';
import {
  itemActionProblem,
  StoreAccessNotice,
  storeAccessBands,
} from './store-access';
import type { AccessProblem } from './store-access';

/* --------------------------------------------------------------- pieces -- */

function emptyStoreCopy(store: Store): string {
  return store.kind === 'team'
    ? 'Items stored in this team vault are accessible to team members according to their assigned roles.'
    : 'Passwords and documents you add are private to this vault.';
}

function itemCount(count: number): string {
  return `${count} ${count === 1 ? 'item' : 'items'}`;
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
  onCopyPath: (item: Item) => void;
  onDelete: (item: Item) => void;
  onMove?: (item: Item) => void;
  accessNow: () => number;
  /** The row's listbox option attributes (`useListbox`). */
  option: ListboxOptionProps;
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
  onCopyPath,
  onDelete,
  onMove,
  accessNow,
  option,
}: RowProps): ReactNode {
  const rowRef = useRef<HTMLDivElement>(null);
  const pressTimer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(pressTimer.current), []);
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
      tabIndex={-1}
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
        className={[
          'row',
          'item',
          selected ? 'sel' : '',
          readOnly ? 'ro' : '',
          menuPoint ? 'menu-target' : '',
        ]
          .filter(Boolean)
          .join(' ')}
        data-row-key={`${item.store}|${item.path}`}
        {...option}
        aria-haspopup="menu"
        // A row selects on the press, as a macOS list does, once the press
        // has focused it (so a Discard changes? alert the guarded selection
        // opens keeps the focus); the click that follows has nothing left to
        // do. A press on a hover action, a modified click and the secondary
        // button leave the selection to the click and the context menu.
        onMouseDown={(event) => {
          if (
            event.button !== 0 ||
            event.ctrlKey ||
            event.metaKey ||
            event.shiftKey ||
            event.altKey ||
            (event.target instanceof Element &&
              event.target.closest('.acts button'))
          )
            return;
          window.clearTimeout(pressTimer.current);
          pressTimer.current = window.setTimeout(() => {
            pressTimer.current = undefined;
            onSelect();
          });
        }}
        onClick={() => {
          if (pressTimer.current === undefined && !selected) onSelect();
        }}
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
          {/* VoiceOver reads the option's text as one run; the hidden commas
              keep "github.com, Personal › logins, 142 B" apart. */}
          <span className="offscreen">, </span>
          <small>{where}</small>
        </span>
        <span className="offscreen">, </span>
        <span className="r">{fmtSize(item.size)}</span>
        {/* Pointer shortcuts only: the context menu (Shift+F10) carries the
            same actions for the keyboard and VoiceOver. */}
        <span className="acts" aria-hidden="true">
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
              Show Details
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
                  icon="eye"
                  reason={readProblem}
                  onClick={choose(() => onReveal(item))}
                >
                  Show Value
                </MenuItem>
                <MenuItem
                  icon="copy"
                  reason={readProblem}
                  onClick={choose(() => onCopy(item))}
                >
                  {kindOf(item) === 'Password' ? 'Copy Password' : 'Copy Value'}
                </MenuItem>
              </>
            )}
            <MenuItem
              icon="folder"
              reason={readProblem}
              onClick={choose(() => onCopyPath(item))}
            >
              Copy Path
            </MenuItem>
            <div className="menu-separator" role="separator" />
            {onMove ? (
              <MenuItem
                icon="pencil"
                reason={writeProblem}
                onClick={choose(() => onMove(item))}
              >
                Rename or Move…
              </MenuItem>
            ) : null}
            <MenuItem
              icon="trash"
              danger
              shortcut="⌘⌫"
              reason={writeProblem}
              onClick={choose(() => onDelete(item))}
            >
              Delete…
            </MenuItem>
          </Menu>
        </ContextMenu>
      ) : null}
    </>
  );
}

/** The list key of a folder row or section. */
function folderRowKey(storeId: string, path: string): string {
  return `${storeId}|${path}/`;
}

/** Return, Space and ⌘↓ open a folder row or section. */
function opensFolder(event: KeyboardEvent<HTMLElement>): boolean {
  if (event.target !== event.currentTarget) return false;
  return (
    ((event.key === 'Enter' || event.key === ' ') &&
      !commandKey(event) &&
      !event.altKey) ||
    (event.key === 'ArrowDown' && commandKey(event))
  );
}

/** A subfolder of the selected folder, listed above its items. */
function FolderRow({
  name,
  count,
  onSelect,
  storeId,
  path,
  option,
}: {
  storeId: string;
  path: string;
  name: string;
  count: number;
  onSelect: () => void;
  option: ListboxOptionProps;
}): ReactNode {
  return (
    <div
      className="row folder"
      data-folder-store={storeId}
      data-folder-path={path}
      data-row-key={folderRowKey(storeId, path)}
      aria-haspopup="menu"
      {...option}
      aria-selected={false}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (!opensFolder(event)) return;
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
        <span className="offscreen">, </span>
        <small>Folder</small>
      </span>
      <span className="offscreen">, </span>
      <span className="r">{itemCount(count)}</span>
    </div>
  );
}

/**
 * A first-level folder's section at a store's root: its name and count over
 * its own items. The label is itself an option that opens the folder, so
 * the folder stays reachable from the list with the keyboard and its
 * context menu.
 */
function FolderSection({
  name,
  count,
  onSelect,
  storeId,
  path,
  option,
}: {
  storeId: string;
  path: string;
  name: string;
  count: number;
  onSelect: () => void;
  option: ListboxOptionProps;
}): ReactNode {
  return (
    <div
      className="gsec folder-sec"
      data-folder-store={storeId}
      data-folder-path={path}
      data-row-key={folderRowKey(storeId, path)}
      aria-haspopup="menu"
      aria-label={`${name}, folder, ${itemCount(count)}`}
      title={`Open ${name}`}
      {...option}
      aria-selected={false}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (!opensFolder(event)) return;
        event.preventDefault();
        onSelect();
      }}
    >
      <span className="nm">{name}</span>
      <span className="n">{count}</span>
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

/** `pendingFocus` value for the first row of whatever the list shows next. */
const FIRST_ROW = '*';

/** One row of the list: an item, a folder row, or a folder's section. */
type ListEntry =
  | { type: 'item'; key: string; item: Item }
  | { type: 'folder' | 'section'; key: string; node: FolderNode };

function itemEntry(item: Item): ListEntry {
  return { type: 'item', key: `${item.store}|${item.path}`, item };
}

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
  // The row to focus once it has rendered (after opening a folder, or going
  // up to the enclosing one), or FIRST_ROW.
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
        // Empty folders hold nothing of a narrowed kind.
        state.kind === 'All'
          ? catalogFolders(snapshot).filter(
              (item) => item.store === selectedStore.id,
            )
          : [],
      )
    : undefined;
  const selectedNode = selectedRoot
    ? (folderAt(selectedRoot, selected.path) ?? selectedRoot)
    : undefined;

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

  const inOrder = <T,>(list: readonly T[]): T[] =>
    state.sortDirection === 'desc' ? [...list].reverse() : [...list];
  const folders = inOrder(selectedNode?.folders ?? []);
  const folderScopedItems = selectedNode?.items ?? [];
  const showEmptyFolder =
    Boolean(selectedStore) && !folders.length && !folderScopedItems.length;

  // What a store or folder lists, in order. A folder lists its subfolders,
  // then its items. A store's root lists its own items, then a section per
  // first-level folder holding that folder's subfolders and items, so the
  // store's contents (and a selected item in a folder) are on screen.
  const atStoreRoot =
    !flatMode && Boolean(selectedStore) && selected.path === '/';
  const paneEntries: ListEntry[] = [];
  if (selectedStore && !flatMode) {
    const storeId = selectedStore.id;
    const folderEntry = (
      type: 'folder' | 'section',
      node: FolderNode,
    ): ListEntry => ({ type, key: folderRowKey(storeId, node.path), node });
    if (atStoreRoot) {
      paneEntries.push(...folderScopedItems.map(itemEntry));
      for (const folder of folders) {
        paneEntries.push(
          folderEntry('section', folder),
          ...inOrder(folder.folders).map((sub) => folderEntry('folder', sub)),
          ...folder.items.map(itemEntry),
        );
      }
    } else {
      paneEntries.push(
        ...folders.map((folder) => folderEntry('folder', folder)),
        ...folderScopedItems.map(itemEntry),
      );
    }
  }
  const listEntries = flatMode ? items.map(itemEntry) : paneEntries;
  const entryAt = new Map(
    listEntries.map((entry, index) => [entry.key, { entry, index }]),
  );
  const selectedKey = state.selection
    ? `${state.selection.store}|${state.selection.path}`
    : null;

  /** Scrolls a windowed list so the row at `index` is mounted and in view. */
  const revealIndex = (index: number): void => {
    const body = bodyRef.current;
    if (!body || !flatMode) return;
    const lineHeight =
      view === 'grid'
        ? (selected.store === ALL_ITEMS
            ? FILES_CARD_HEIGHT_WITH_LOCATION
            : FILES_CARD_HEIGHT) + FILES_CARD_GAP
        : ROW_HEIGHT;
    const line =
      view === 'grid' ? Math.floor(index / gridWindow.columns) : index;
    const top = listMetrics.top + line * lineHeight;
    const height = body.clientHeight || listMetrics.viewport;
    let next = body.scrollTop;
    if (top < next) next = top;
    else if (top + lineHeight > next + height) next = top + lineHeight - height;
    if (next === body.scrollTop) return;
    body.scrollTop = next;
    setScrollTop(next);
  };

  // The rows are one listbox: one Tab stop, Up/Down/Home/End and
  // type-select. Reaching an item selects it; a folder row only takes focus,
  // and Return opens it.
  const listbox = useListbox({
    keys: listEntries.map((entry) => entry.key),
    selectedKey,
    renderedKeys: flatMode
      ? visibleRows.map((item) => `${item.store}|${item.path}`)
      : undefined,
    // Cards: Left and Right step a card, Up and Down a line of cards.
    columns: view === 'grid' ? gridWindow.columns : 1,
    labelOf: (key) => {
      const entry = entryAt.get(key)?.entry;
      if (!entry) return '';
      return entry.type === 'item'
        ? nameOf(entry.item.path)
        : displayPathComponent(entry.node.name);
    },
    onMove: (key) => {
      const found = entryAt.get(key);
      if (!found) return;
      revealIndex(found.index);
      if (found.entry.type === 'item')
        locations.select({
          store: found.entry.item.store,
          path: found.entry.item.path,
        });
    },
  });

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
    const rows = [
      ...(bodyRef.current?.querySelectorAll<HTMLElement>('[data-row-key]') ??
        []),
    ];
    const row =
      key === FIRST_ROW
        ? rows[0]
        : rows.find((candidate) => candidate.dataset.rowKey === key);
    if (!row) return;
    pendingFocus.current = null;
    row.focus({ preventScroll: true });
    row.scrollIntoView?.({ block: 'nearest' });
  });

  // Show the unavailable state when a stale link or catalog refresh refers to
  // a store that is no longer present.
  if (location.kind === 'store' && !store) {
    return (
      <div className="body">
        <EmptyState
          icon="lock"
          title="Vault Unavailable"
          body="This vault is no longer on this device. Refresh, or choose another vault from the sidebar."
        />
      </div>
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
    pendingFocus.current ??= FIRST_ROW;
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
  const copyPath = (item: Item): void => {
    if (!allowAction(item)) return;
    void bridge.copyItemPath(request(item)).then(
      () => toasts.show('Path copied'),
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
  // A store that cannot be opened keeps the list layout: its head (with a
  // lock), the notice saying why and how to recover, and no rows — its
  // items stay concealed (the catalog withholds them) and the details
  // column says No Selection.
  const scopeState = scopeStore
    ? storeDescriptionState(snapshot, scopeStore)
    : 'normal';
  const scopeProblem: AccessProblem | null =
    scopeState !== 'normal'
      ? scopeState
      : expiredHere
        ? 'check-in-expired'
        : null;
  // The lock marks a lapsed or missing check-in, as the sidebar does.
  const scopeLocked =
    scopeProblem === 'check-in-expired' ||
    scopeProblem === 'check-in-unavailable';
  const openServer = (profile: string): void =>
    locations.navigate({ kind: 'settings', section: 'account', profile });

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
      onCopyPath={copyPath}
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
      option={listbox.optionProps(`${item.store}|${item.path}`)}
    />
  );

  const paneRows = selectedStore
    ? paneEntries.map((entry) => {
        if (entry.type === 'item') return row(entry.item);
        const props = {
          storeId: selectedStore.id,
          path: entry.node.path,
          name: displayPathComponent(entry.node.name),
          count: entry.node.count,
          onSelect: () => selectFolder(selectedStore, entry.node.path),
          option: listbox.optionProps(entry.key),
        };
        return entry.type === 'section' ? (
          <FolderSection key={entry.key} {...props} />
        ) : (
          <FolderRow key={entry.key} {...props} />
        );
      })
    : [];

  /** ⌘↑ goes to the enclosing folder, with the folder just left focused. */
  const onListPaneKeyDown = (event: KeyboardEvent<HTMLElement>): void => {
    folderMenu.handlers.onKeyDown(event);
    if (event.defaultPrevented) return;
    if (event.key !== 'ArrowUp' || !commandKey(event) || event.altKey) return;
    if (!selectedStore || selected.path === '/') return;
    if (
      event.target instanceof Element &&
      event.target.closest('input, textarea, select')
    )
      return;
    event.preventDefault();
    const parent =
      selected.path.slice(0, selected.path.lastIndexOf('/')) || '/';
    pendingFocus.current = folderRowKey(selectedStore.id, selected.path);
    selectFolder(selectedStore, parent);
  };

  /**
   * A click on the list's empty space clears the selection, as in Finder;
   * a click anywhere else (bars, sidebar, menus) leaves it alone. The
   * selection change is guarded, so unsaved edits prompt first.
   */
  const onListBodyClick = (event: MouseEvent<HTMLDivElement>): void => {
    if (!state.selection || event.button !== 0) return;
    if (
      event.target instanceof Element &&
      event.target.closest(
        '[data-option-key], .list-head, .bandstrip, .access-notice, .nextup, .notice, .starter, .empty, button, a, input',
      )
    )
      return;
    locations.select(null, { closeDetails: true });
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
          <span className="ci pw">
            <Icon name="key" />
          </span>
          <b>New Password</b>
          <small>A login, API key or any secret value.</small>
          <kbd>⌘N</kbd>
        </button>
        <button
          type="button"
          className="card"
          onClick={() => createNew('Document')}
        >
          <span className="ci file">
            <Icon name="upload" />
          </span>
          <b>Add Document</b>
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
      title="No Items"
      body={
        scope && selected.path === '/'
          ? emptyStoreCopy(scope)
          : scope
            ? 'This folder is empty.'
            : 'Save passwords and documents to get started.'
      }
    >
      {scope && !readOnly && currentStoreAvailable(scope.id) ? (
        <Button
          onClick={() =>
            createNew(state.kind === 'Document' ? 'Document' : 'Password')
          }
        >
          {state.kind === 'Document' ? 'New Document' : 'New Password'}
        </Button>
      ) : null}
    </EmptyState>
  );

  const listShown = flatMode ? items.length > 0 : !showEmptyFolder;
  // The list foot: where a dropped file goes, and the store's uploads.
  const dropHint =
    state.query || readOnly || !dropTarget || dropTarget.blocked
      ? null
      : dropTarget.store.name;
  const listFoot =
    scopeStore && !scopeProblem ? (
      <div className="list-foot">
        {dropHint && listShown ? (
          <span
            className="drop-hint"
            title={`Drop a file to add it to ${dropHint}`}
          >
            <Icon name="upload" />
            <span className="t">Drop a file to add it here</span>
          </span>
        ) : null}
        <span className="sp" />
        <FileUploadsPanel
          key={scopeStore.id}
          bridge={bridge}
          storeId={scopeStore.id}
          storeName={scopeStore.name}
        />
      </div>
    ) : null;

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
          <div
            className="virtual-rows"
            ref={(node) => {
              listRef.current = node;
              listbox.listProps.ref.current = node;
            }}
            role={listbox.listProps.role}
            aria-label={scopeLabel}
            onKeyDown={listbox.listProps.onKeyDown}
            onFocus={listbox.listProps.onFocus}
          >
            {rowWindow.padTop ? (
              <div
                className="virtual-spacer"
                aria-hidden="true"
                style={{ height: rowWindow.padTop }}
              />
            ) : null}
            {view === 'grid' ? (
              <div
                role="presentation"
                className={
                  selected.store === ALL_ITEMS
                    ? 'files-grid with-location'
                    : 'files-grid'
                }
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
                aria-hidden="true"
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
        <div
          className={view === 'grid' ? 'files-grid' : 'virtual-rows'}
          {...listbox.listProps}
          aria-label={scopeLabel}
        >
          {paneRows}
        </div>
      </div>
    );

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
        <p className="search-scope">
          in {scopeLabel}
          {kindMeta && selected.store !== ALL_ITEMS
            ? ` · ${kindMeta.plural}`
            : ''}
        </p>
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
        <h2>
          {scopeLabel}
          {scopeLocked ? (
            <span className="lock" role="img" aria-label="Locked">
              <Icon name="lock" />
            </span>
          ) : null}
        </h2>
        <p>
          {kindMeta ? (
            <button
              type="button"
              className="kind-filter"
              title="Show all kinds"
              aria-label={`Showing ${kindMeta.plural.toLowerCase()} only. Show all kinds`}
              onClick={() => locations.setKind('All')}
            >
              {kindMeta.plural}
              <Icon name="close" />
            </button>
          ) : null}
          {/* A store that cannot be read has no count to give: its items
              are withheld, not absent. */}
          {scopeProblem
            ? ''
            : itemReadiness === 'ready'
              ? `${itemCount(shownCount)} · `
              : `${shownCount} items loaded · `}
          {serverDisplayLabelForStore(snapshot, selectedStore)}
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
        onKeyDown={onListPaneKeyDown}
      >
        <div
          className="body folder-body rows"
          ref={bodyRef}
          onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
          onClick={onListBodyClick}
        >
          {listHead}
          {location.kind === 'all' && accessBands.length ? (
            <div className="bandstrip">
              {accessBands.map((band) => (
                <Notice
                  key={band.key}
                  severity="crit"
                  icon={band.title === 'Check-in expired' ? 'lock' : undefined}
                  title={band.title}
                  actions={
                    <Button onClick={() => openServer(band.server)}>
                      Check Status
                    </Button>
                  }
                >
                  <p>{band.text}</p>
                </Notice>
              ))}
            </div>
          ) : null}
          {scopeProblem && scopeStore ? (
            <div className="access-notice" role="status">
              <StoreAccessNotice
                snapshot={snapshot}
                store={scopeStore}
                state={scopeProblem}
                onOpenServer={openServer}
                onFinishSetup={() => void onResume(scopeStore.id)}
              />
            </div>
          ) : (
            <>
              {items.length > 0 ? (
                <CollectionStatus
                  state={itemReadiness}
                  label="remaining items"
                />
              ) : null}
              {listBody}
            </>
          )}
        </div>
        {listFoot}
      </section>
    </div>
  );
}
