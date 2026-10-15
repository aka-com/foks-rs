import { itemsReadiness } from '../model';
import { CollectionStatus } from '../components/collection-status';
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

import { Fragment, useEffect, useLayoutEffect, useRef, useState } from 'react';
import type {
  FocusEvent as ReactFocusEvent,
  KeyboardEvent as ReactKeyboardEvent,
  PointerEvent as ReactPointerEvent,
  ReactNode,
  RefObject,
} from 'react';
import { useToast } from '../../kit/toasts';
import { anyDialogOpen, ContextMenu, Menu } from '../../kit/overlay-primitives';
import { useDismissedOnboardingTip } from '../onboarding-tips';
import { virtualListWindow } from '../../kit/virtual-list';
import { rememberFilesView, storedFilesView } from '../files-view-pref';
import type { FilesView } from '../files-view-pref';
import {
  FILES_CARD_GAP,
  FILES_CARD_HEIGHT,
  FILES_CARD_HEIGHT_WITH_LOCATION,
  filesGridWindow,
} from './files-grid';
import {
  gridFocusTarget,
  revealOffset,
  rovingKey,
  treeEntries,
  treeKeyAction,
} from './files-keyboard';
import type { TreeEntry } from './files-keyboard';
import {
  Band,
  Button,
  Icon,
  MenuItem,
  Notice,
  PANE_SIZES,
  SegmentedControl,
  StoreMark,
  usePaneResize,
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
  isTrashPath,
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
  TRASH_ROOT,
  trashEntries,
} from '../model';
import type { Item, Store, AgentSnapshot, TrashEntry } from '../model';
import { TrashView } from './trash-view';
import type {
  KindFilter,
  LocationStore,
  LocationState,
  SortKey,
  SortDirection,
} from '../location';
import { normalizeCommandError } from '../bridge';
import type { Bridge, ItemField } from '../bridge';
import { useFileDrop } from '../file-drop';
import { fileDropPath } from './write-workflow-model';
import { writeBlockReason } from './store-choice';
import { exportedPasswords } from './export-passwords-sheet';
import type { NewKind } from './write-workflow-model';
import { copyActions, copyAndAnnounce } from './item-copy';
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
import {
  NO_ROWS,
  allRows,
  copyProblem,
  dropVerdict,
  extendRows,
  itemCount,
  rowKey,
  selectedRows,
  toggleRow,
} from './item-moves';
import type { DropVerdict, MultiSelection } from './item-moves';
import { matchesShortcut, shortcutLabel } from '../shortcuts';
import type { KeyPress } from '../shortcuts';
import { manageReason } from './group-model';
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
  const head = (key: SortKey, label: string): ReactNode => (
    <span
      role="columnheader"
      aria-sort={
        shown === key
          ? direction === 'asc'
            ? 'ascending'
            : 'descending'
          : undefined
      }
    >
      <button
        type="button"
        className={shown === key ? 'on' : undefined}
        aria-pressed={shown === key}
        title={`Sort by ${label.toLowerCase()} ${shown === key && direction === 'asc' ? 'descending' : 'ascending'}`}
        onClick={() =>
          onSort(key, shown === key && direction === 'asc' ? 'desc' : 'asc')
        }
      >
        {label}
        {shown === key ? (direction === 'asc' ? ' ↑' : ' ↓') : ''}
      </button>
    </span>
  );
  return (
    <div className={`hdr ${columnClass(columns)}`} role="row" aria-rowindex={1}>
      {head('name', 'Name')}
      {head('kind', 'Kind')}
      {columns.location ? head('group', 'Location') : null}
      <span role="columnheader" className="num">
        Size
      </span>
      {/* Names the cell that holds a row's own actions; it takes no space. */}
      <span role="columnheader" className="acts-head">
        Actions
      </span>
    </div>
  );
}

/**
 * Where a list row or card sits in the Files grid. The list view is a grid
 * of rows under the column headers; the card view is a grid of lines whose
 * cells are the cards. Only the entry carrying `tabIndex` 0 is a tab stop.
 */
interface RowNav {
  layout: 'list' | 'card';
  /** `data-nav-key`: the key the arrow keys move focus by. */
  key: string;
  tabIndex: 0 | -1;
  /** aria-rowindex of a list row; a card's line carries its own. */
  rowIndex?: number;
  /** aria-colindex of a card within its line. */
  colIndex?: number;
}

/**
 * The outer element's grid role and position for `nav`. A row cannot carry
 * aria-haspopup, so a list row names the key that opens its menu instead.
 */
function navAttributes(nav: RowNav, selected?: boolean) {
  return nav.layout === 'list'
    ? {
        role: 'row',
        'aria-rowindex': nav.rowIndex,
        'aria-selected': selected,
        'aria-keyshortcuts': 'Shift+F10',
        tabIndex: nav.tabIndex,
        'data-nav-key': nav.key,
      }
    : {
        role: 'gridcell',
        'aria-colindex': nav.colIndex,
        'aria-selected': selected,
        'aria-haspopup': 'menu' as const,
        tabIndex: nav.tabIndex,
        'data-nav-key': nav.key,
      };
}

/** The role of a row's columns: cells in the list, plain content in a card. */
function cellRole(nav: RowNav): 'gridcell' | undefined {
  return nav.layout === 'list' ? 'gridcell' : undefined;
}

/** The roving key of a subfolder listed above the selected folder's items. */
function folderNavKey(storeId: string, path: string): string {
  return `folder:${storeId}|${path}`;
}

interface RowProps {
  nav: RowNav;
  snapshot: AgentSnapshot;
  hues: ReadonlyMap<string, string>;
  item: Item;
  columns: Columns;
  selected: boolean;
  /** A store that can be read but not written: rows are drawn back. */
  readOnly: boolean;
  /**
   * How many rows the menu's Move and Trash act on: the size of the
   * multi-selection this row belongs to, or 0 when it acts on this row.
   */
  group: number;
  onSelect: () => void;
  /** ⌘-click toggles the row in the multi-selection; Shift-click extends it. */
  onPick: (mode: 'toggle' | 'extend') => void;
  /** Starts a drag of the row, or of the multi-selection it belongs to. */
  onPointerDown?: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onCopy: (item: Item, field: ItemField) => void;
  onReveal: (item: Item) => void;
  onDownload: (item: Item) => void;
  onTrash: (item: Item) => void;
  onMove?: (item: Item) => void;
  onCopyToVault?: (item: Item) => void;
  accessNow: () => number;
}

function Row({
  nav,
  snapshot,
  hues,
  item,
  columns,
  selected,
  readOnly,
  group,
  onSelect,
  onPick,
  onPointerDown,
  onCopy,
  onReveal,
  onDownload,
  onTrash,
  onMove,
  onCopyToVault,
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
  const copies = copyActions(item);
  /** What a key pressed on the row itself runs, if anything. */
  const rowShortcut = (event: KeyPress): (() => void) | undefined => {
    // Moving to Trash can be undone, for several chosen rows as for one, so
    // the key asks for nothing more.
    if (matchesShortcut(event, 'rowTrash')) return () => onTrash(item);
    if (matchesShortcut(event, 'rowOpen')) return onSelect;
    if (matchesShortcut(event, 'rowToggle')) return () => onPick('toggle');
    if (matchesShortcut(event, 'rowExtend')) return () => onPick('extend');
    if (onMove && matchesShortcut(event, 'rowRename'))
      return () => onMove(item);
    // A file has nothing to copy, and selected text is copied as text; the
    // key is left to the window for both.
    const selection = window.getSelection();
    if (selection && !selection.isCollapsed) return undefined;
    const copy = matchesShortcut(event, 'rowCopyUsername')
      ? copies.find(({ field }) => field === 'username')
      : matchesShortcut(event, 'rowCopy')
        ? copies[0]
        : undefined;
    return copy && (() => onCopy(item, copy.field));
  };
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
      key={label}
      type="button"
      // Reached with the arrow keys from the row, which is the tab stop.
      tabIndex={-1}
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
        {...navAttributes(nav, selected)}
        data-row-key={rowKey(item)}
        onClick={(event) => {
          if (event.metaKey || event.ctrlKey) onPick('toggle');
          else if (event.shiftKey) onPick('extend');
          else onSelect();
        }}
        onPointerDown={onPointerDown}
        onContextMenu={(event) => {
          event.preventDefault();
          rowRef.current?.focus();
          setMenuPoint({ x: event.clientX, y: event.clientY });
        }}
        onKeyDown={(event) => {
          if (matchesShortcut(event, 'rowMenu')) {
            event.preventDefault();
            const bounds = event.currentTarget.getBoundingClientRect();
            setMenuPoint({ x: bounds.left, y: bounds.bottom });
            return;
          }
          if (event.target !== event.currentTarget) return;
          const run = rowShortcut(event);
          if (!run) return;
          event.preventDefault();
          run();
        }}
      >
        <span className="name" role={cellRole(nav)}>
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
        <span className="cell kind" role={cellRole(nav)}>
          {kindLabel(kind)}
        </span>
        {columns.location ? (
          <span className="cell mark" role={cellRole(nav)}>
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
        <span className="cell num" role={cellRole(nav)}>
          {fmtSize(item.size)}
        </span>
        <span className="acts" role={cellRole(nav)}>
          {item.kind !== 'File' ? (
            <>
              {copies.map(({ field, label }) =>
                action(label, field === 'username' ? 'user' : 'copy', () =>
                  onCopy(item, field),
                ),
              )}
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
                {copies.map(({ field, label }) => (
                  <MenuItem
                    key={field}
                    icon={field === 'username' ? 'user' : 'copy'}
                    reason={readProblem}
                    onClick={choose(() => onCopy(item, field))}
                  >
                    {label}
                  </MenuItem>
                ))}
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
                {group > 1 ? `Move ${group} items to…` : 'Rename or move…'}
              </MenuItem>
            ) : null}
            {onCopyToVault && group <= 1 ? (
              <MenuItem
                reason={copyProblem(item) ?? readProblem}
                onClick={choose(() => onCopyToVault(item))}
              >
                Copy to vault…
              </MenuItem>
            ) : null}
            <MenuItem
              icon="trash"
              danger
              reason={writeProblem}
              onClick={choose(() => onTrash(item))}
            >
              {group > 1 ? `Move ${group} items to Trash` : 'Move to Trash'}
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
  nav,
  name,
  count,
  columns,
  onSelect,
  storeId,
  path,
  drop,
}: {
  nav: RowNav;
  storeId: string;
  path: string;
  name: string;
  count: number;
  columns: Columns;
  onSelect: () => void;
  /** Rows dragged over this folder: whether they can be dropped here. */
  drop?: 'ok' | 'no';
}): ReactNode {
  return (
    <div
      className={columnClass(
        columns,
        'row',
        'one',
        'folder',
        drop ? `drop-${drop}` : '',
      )}
      data-folder-store={storeId}
      data-folder-path={path}
      {...navAttributes(nav)}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.target !== event.currentTarget) return;
        if (!matchesShortcut(event, 'rowOpen')) return;
        event.preventDefault();
        onSelect();
      }}
    >
      <span className="name" role={cellRole(nav)}>
        <Icon name="folder" className="f" />
        <span className="nm" title={name}>
          {name}
        </span>
      </span>
      <span className="cell kind" role={cellRole(nav)}>
        Folder
      </span>
      {columns.location ? <span className="cell" role={cellRole(nav)} /> : null}
      <span className="cell num" role={cellRole(nav)}>
        {itemCount(count)}
      </span>
      {nav.layout === 'list' ? (
        <span className="acts-none" role="gridcell" />
      ) : null}
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
  drop,
  tree,
}: {
  storeId?: string;
  path?: string;
  /** Rows dragged over this folder: whether they can be dropped here. */
  drop?: 'ok' | 'no';
  /**
   * The row's place in a folder tree, where it is a treeitem: the whole row
   * is the focus target, and the twist and its own action are pointer
   * shortcuts for the arrow keys and the context menu. A row without one is
   * a single button outside any tree.
   */
  tree?: { key: string; level: number; tabIndex: 0 | -1 };
  depth: number;
  active: boolean;
  root?: boolean;
  /** The muted invitation row a section draws in place of no rows at all. */
  quiet?: boolean;
  icon: 'folder' | 'vault' | 'grid' | 'plus' | 'trash';
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
  const className = [
    'fn',
    active ? 'on' : '',
    root ? 'root' : '',
    quiet ? 'quiet' : '',
    drop ? `drop-${drop}` : '',
  ]
    .filter(Boolean)
    .join(' ');
  const style = { '--d': depth } as React.CSSProperties;
  const label = (
    <>
      {mark ?? <Icon name={icon} />}
      <span className="nm">{name}</span>
    </>
  );
  const trailing = (
    <>
      {action}
      {tag ?? (count === undefined ? null : <span className="c">{count}</span>)}
    </>
  );
  if (!tree)
    return (
      <div className={className} style={style}>
        <span className="twist none" aria-hidden="true" />
        <button
          type="button"
          className="fselect"
          aria-current={active ? 'location' : undefined}
          onClick={onSelect}
        >
          {label}
        </button>
        {trailing}
      </div>
    );
  return (
    <div
      className={className}
      data-folder-store={storeId}
      data-folder-path={path}
      data-tree-key={tree.key}
      style={style}
      role="treeitem"
      aria-level={tree.level}
      aria-expanded={expandable ? open : undefined}
      aria-selected={active}
      aria-haspopup={storeId ? 'menu' : undefined}
      tabIndex={tree.tabIndex}
      onClick={onSelect}
    >
      {/* Every row keeps the twist cell so names align down the column; a row
          with nothing to disclose leaves it empty. */}
      <span
        className={expandable ? (open ? 'twist open' : 'twist') : 'twist none'}
        aria-hidden="true"
        onClick={
          expandable
            ? (event) => {
                event.stopPropagation();
                onToggle();
              }
            : undefined
        }
      >
        {expandable ? <Icon name="chevronDown" /> : null}
      </span>
      <span className="fselect">{label}</span>
      {trailing}
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
  /** Moves an item, or a folder with its contents, to Trash. */
  onTrash: (item: Item) => void;
  /** Moves a Trash entry back to where it was. */
  onRestore?: (entry: TrashEntry) => void;
  /** Asks to remove a Trash entry permanently. */
  onDeletePermanently?: (item: Item) => void;
  /** Asks to remove every entry in a store's Trash permanently. */
  onEmptyTrash?: (storeId: string) => void;
  onMove?: (item: Item) => void;
  /** Copies an item into another vault, the same server's or another's. */
  onCopyToVault?: (item: Item) => void;
  /** Imports logins from a CSV export into a store, at `folder` when given. */
  onImport?: (storeId: string, folder?: string) => void;
  /** Exports the passwords of a store to a plain-text CSV file. */
  onExport?: (storeId: string) => void;
  /**
   * Moves several items of one store: to `folder` when a drag names it, or
   * to a folder chosen in the move sheet.
   */
  onMoveMany?: (items: readonly Item[], folder?: string) => void;
  /** Moves several items to Trash. */
  onTrashMany?: (items: readonly Item[]) => void;
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

/** The mounted list row or card whose `data-nav-key` is `key`. */
function navElement(
  container: HTMLElement | null,
  key: string,
): HTMLElement | undefined {
  return [
    ...(container?.querySelectorAll<HTMLElement>('[data-nav-key]') ?? []),
  ].find((element) => element.dataset.navKey === key);
}

/** The drawn folder tree row whose `data-tree-key` is `key`. */
function treeElement(
  container: HTMLElement,
  key: string,
): HTMLElement | undefined {
  return [...container.querySelectorAll<HTMLElement>('[data-tree-key]')].find(
    (element) => element.dataset.treeKey === key,
  );
}

/**
 * Returns focus to the list once the chosen rows are gone together with the
 * control that had it: Clear, or a move of every chosen row, unmounts the
 * selection bar and its focused button. Focus goes to the anchor row, else
 * to the row now at the first chosen row's place. A sheet that is still open
 * keeps focus until it closes.
 */
function ChosenRowsFocus({
  count,
  anchor,
  first,
  listed,
  body,
}: {
  count: number;
  anchor: string | null;
  /** The list index of the first chosen row. */
  first: number;
  listed: readonly string[];
  body: RefObject<HTMLDivElement | null>;
}): null {
  const pending = useRef<{ anchor: string | null; index: number } | null>(null);
  useEffect(() => {
    if (count) {
      pending.current = { anchor, index: first };
      return;
    }
    const last = pending.current;
    if (!last || anyDialogOpen()) return;
    pending.current = null;
    const active = document.activeElement;
    if (active && active !== document.body) return;
    const key =
      last.anchor && listed.includes(last.anchor)
        ? last.anchor
        : listed[Math.min(Math.max(last.index, 0), listed.length - 1)];
    const element = body.current;
    const rows = [
      ...(element?.querySelectorAll<HTMLElement>('[data-row-key]') ?? []),
    ];
    const target =
      rows.find((row) => row.dataset.rowKey === key) ??
      element?.querySelector<HTMLElement>('[tabindex="0"], button');
    target?.focus();
  });
  return null;
}

export function ItemsScreen({
  snapshot,
  bridge,
  state,
  locations,
  onReveal,
  onNew,
  onResume,
  onTrash,
  onRestore,
  onDeletePermanently,
  onEmptyTrash,
  onMove,
  onCopyToVault,
  onImport,
  onExport,
  onMoveMany,
  onTrashMany,
  onSettings,
  onTeamInfo,
  onNewFolder,
  onCommandError,
  onUploadDroppedFile,
  dropEnabled = true,
  accessNow = () => Date.now() / 1000,
}: ItemsScreenProps): ReactNode {
  const sidebar = usePaneResize<HTMLDivElement>(PANE_SIZES.files);
  // Rows chosen together, valid only for the view they were chosen in.
  const [multi, setMulti] = useState<{ scope: string; rows: MultiSelection }>({
    scope: '',
    rows: NO_ROWS,
  });
  const [drag, setDrag] = useState<{
    x: number;
    y: number;
    what: string;
    verdict: DropVerdict | null;
    target: string | null;
  } | null>(null);
  // Set when a drag ends, so the click that follows the release is not read
  // as a selection.
  const suppressClick = useRef(false);
  // Ends the drag in progress, if any; an unmount ends it unapplied.
  const endDrag = useRef<((drop: boolean) => void) | null>(null);
  useEffect(() => () => endDrag.current?.(false), []);
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
  // The list's and the tree's roving tab stops: the entry focus last moved
  // to, kept while it is still drawn.
  const [rowFocus, setRowFocus] = useState<string | null>(null);
  const [treeFocus, setTreeFocus] = useState<string | null>(null);
  // A row the arrow keys moved to before the virtual window had mounted it;
  // it takes focus once the scroll that brings it in has rendered.
  const pendingRowFocus = useRef<string | null>(null);
  useLayoutEffect(() => {
    const key = pendingRowFocus.current;
    if (!key) return;
    pendingRowFocus.current = null;
    navElement(bodyRef.current, key)?.focus({ preventScroll: true });
  });
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

  const trashCounts = new Map<string, number>();
  const trashCount = (storeId: string): number => {
    let count = trashCounts.get(storeId);
    if (count === undefined) {
      count = trashEntries(snapshot, storeId).length;
      trashCounts.set(storeId, count);
    }
    return count;
  };
  // The vaults and the teams are each one tree: one tab stop whose focused
  // row the arrow keys move, falling back to the selected folder.
  const treeRoots = (list: readonly StoreTree[]) =>
    list.map((tree) => ({
      storeId: tree.store.id,
      root: tree.root,
      trash: storeReadable(snapshot, tree.store.id),
    }));
  const vaultEntries = treeEntries(treeRoots(vaultTrees), closed, TRASH_ROOT);
  const teamEntries = treeEntries(treeRoots(teamTrees), closed, TRASH_ROOT);
  const selectedTreeKey =
    selected.store === ALL_ITEMS ? null : `${selected.store}|${selected.path}`;
  const treeStop = (entries: readonly TreeEntry[]): string | undefined =>
    rovingKey(
      entries.map((entry) => entry.key),
      treeFocus,
      selectedTreeKey,
    );
  const vaultStop = treeStop(vaultEntries);
  const teamStop = treeStop(teamEntries);
  const treePlace = (
    key: string,
    level: number,
    stop: string | undefined,
  ): { key: string; level: number; tabIndex: 0 | -1 } => ({
    key,
    level,
    tabIndex: key === stop ? 0 : -1,
  });
  const treeKeys =
    (entries: readonly TreeEntry[]) =>
    (event: ReactKeyboardEvent<HTMLDivElement>): void => {
      if (event.altKey || event.metaKey || event.ctrlKey) return;
      const row = event.target;
      if (!(row instanceof HTMLElement) || !row.dataset.treeKey) return;
      const index = entries.findIndex(
        (entry) => entry.key === row.dataset.treeKey,
      );
      const action = treeKeyAction(event.key, entries, index);
      if (!action) return;
      event.preventDefault();
      const entry = entries[action.index];
      if (action.kind === 'toggle') locations.toggleFolder(entry.key);
      else if (action.kind === 'select') row.click();
      else {
        setTreeFocus(entry.key);
        treeElement(event.currentTarget, entry.key)?.focus();
      }
    };
  const treeFocused = (event: ReactFocusEvent<HTMLDivElement>): void => {
    if (event.target instanceof HTMLElement && event.target.dataset.treeKey)
      setTreeFocus(event.target.dataset.treeKey);
  };
  const drawFolders = (
    folders: readonly FolderNode[],
    treeStore: Store,
    depth: number,
    stop: string | undefined,
  ): ReactNode =>
    folders.map((folder) => {
      const foldKey = `${treeStore.id}|${folder.path}`;
      const open = !closed.has(foldKey);
      return (
        <Fragment key={foldKey}>
          <TreeRow
            tree={treePlace(foldKey, depth + 1, stop)}
            storeId={treeStore.id}
            path={folder.path}
            depth={depth}
            active={
              selected.store === treeStore.id && selected.path === folder.path
            }
            icon="folder"
            name={displayPathComponent(folder.name)}
            count={folder.count}
            drop={dropOn(treeStore.id, folder.path)}
            open={open}
            expandable={folder.folders.length > 0}
            onSelect={() => selectFolder(treeStore, folder.path)}
            onToggle={() => locations.toggleFolder(foldKey)}
          />
          {open
            ? drawFolders(folder.folders, treeStore, depth + 1, stop)
            : null}
        </Fragment>
      );
    });

  // Render store root row and its expandable folder sub-tree.
  const storeRoot = (tree: StoreTree, stop: string | undefined): ReactNode => {
    const active = selected.store === tree.store.id && selected.path === '/';
    const foldKey = `${tree.store.id}|/`;
    const trashKey = `${tree.store.id}|${TRASH_ROOT}`;
    const open = !closed.has(foldKey);
    const expired = leaseExpired(tree.store);
    const hasTrash = storeReadable(snapshot, tree.store.id);
    return (
      <Fragment key={tree.store.id}>
        <TreeRow
          tree={treePlace(foldKey, 1, stop)}
          storeId={tree.store.id}
          path="/"
          depth={0}
          root
          drop={dropOn(tree.store.id, '/')}
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
          // Trash is drawn under the store, so a store without folders is
          // still a parent that opens and closes.
          expandable={tree.root.folders.length > 0 || hasTrash}
          action={
            tree.store.kind === 'team' ? (
              // A pointer shortcut: the row's menu offers Team settings to
              // the keyboard and to assistive technology.
              <span
                className="fact"
                title="Team settings"
                aria-hidden="true"
                onClick={(event) => {
                  event.stopPropagation();
                  onSettings(tree.store.id);
                }}
              >
                <Icon name="settings" />
              </span>
            ) : undefined
          }
          onSelect={() => selectFolder(tree.store, '/')}
          onToggle={() => locations.toggleFolder(foldKey)}
        />
        {open ? drawFolders(tree.root.folders, tree.store, 1, stop) : null}
        {open && hasTrash ? (
          <TreeRow
            tree={treePlace(trashKey, 2, stop)}
            depth={1}
            active={
              selected.store === tree.store.id && selected.path === TRASH_ROOT
            }
            quiet={!trashCount(tree.store.id)}
            icon="trash"
            name="Trash"
            count={trashCount(tree.store.id) || undefined}
            open={false}
            expandable={false}
            onSelect={() => selectFolder(tree.store, TRASH_ROOT)}
            onToggle={() => {}}
          />
        ) : null}
      </Fragment>
    );
  };

  const createStore =
    selectedTree && storeReadable(snapshot, selectedTree.store.id)
      ? selectedTree.store.id
      : store && storeReadable(snapshot, store.id)
        ? store.id
        : (defaultCreateStore(snapshot) ?? '');
  // Trash is not a place to create items in: New saves at the store's root.
  const trashSelected = Boolean(selectedTree) && isTrashPath(selected.path);
  const createFolder =
    selectedTree && selectedNode && selected.path !== '/' && !trashSelected
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
  const dropStore = trashSelected
    ? undefined
    : location.kind === 'store'
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
  const copyItemField = (item: Item, field: ItemField): void => {
    void copyAndAnnounce({
      bridge,
      item,
      field,
      toasts,
      problem: itemActionProblem(snapshot, item, false, accessNow()),
      onError: onCommandError,
    });
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

  /* ------------------------------------------------- multi-selection -- */

  // The rows the list draws, in order: what ⌘A selects and Shift-click
  // ranges over. Trash draws its own view.
  const listed = trashSelected ? [] : flatMode ? items : folderScopedItems;
  const listedKeys = listed.map(rowKey);
  const multiScope = [
    location.kind,
    location.kind === 'store' ? location.ref : '',
    state.folder,
    state.query,
    state.kind,
  ].join('\n');
  const chosenRows = multi.scope === multiScope ? multi.rows : NO_ROWS;
  const chosen = selectedRows(chosenRows, listed);
  const chosenKeys = new Set(chosen.map(rowKey));
  const setChosen = (rows: MultiSelection): void =>
    setMulti({ scope: multiScope, rows });
  // The row the details panel shows joins a multi-selection started beside it.
  const seed =
    state.selection && listedKeys.includes(rowKey(state.selection))
      ? rowKey(state.selection)
      : null;
  const pickRow = (item: Item, mode: 'toggle' | 'extend'): void => {
    if (suppressClick.current) return;
    const current = { keys: chosen.map(rowKey), anchor: chosenRows.anchor };
    setChosen(
      mode === 'toggle'
        ? toggleRow(current, rowKey(item), seed)
        : extendRows(current, listedKeys, rowKey(item), seed),
    );
  };
  /** The multi-selection `item` belongs to, when it has more than one row. */
  const groupOf = (item: Item): readonly Item[] | null =>
    chosen.length > 1 && chosenKeys.has(rowKey(item)) ? chosen : null;
  const writeProblems = (group: readonly Item[]): (string | undefined)[] =>
    group.map((item) => itemActionProblem(snapshot, item, true, accessNow()));
  const storeBlocked = (target: Store): string | null =>
    !availability(target).available
      ? storeDescription(snapshot, target)
      : writeBlockReason(snapshot, target);
  const moveGroupReason = (group: readonly Item[]): string | undefined => {
    if (new Set(group.map((item) => item.store)).size > 1)
      return 'Select items from one vault to move them together.';
    const problems = writeProblems(group);
    return problems.every(Boolean) ? problems[0] : undefined;
  };
  const trashGroupReason = (group: readonly Item[]): string | undefined => {
    const problems = writeProblems(group);
    return problems.every(Boolean) ? problems[0] : undefined;
  };
  const moveRow = (item: Item): void => {
    const group = groupOf(item);
    if (group && onMoveMany) {
      const reason = moveGroupReason(group);
      if (reason) toasts.show(reason, { tone: 'warning' });
      else onMoveMany(group);
      return;
    }
    if (onMove && allowAction(item, true)) onMove(item);
  };
  const copyRow = (item: Item): void => {
    if (onCopyToVault && allowAction(item, false)) onCopyToVault(item);
  };
  const trashRow = (item: Item): void => {
    const group = groupOf(item);
    if (group && onTrashMany) {
      const reason = trashGroupReason(group);
      if (reason) toasts.show(reason, { tone: 'warning' });
      else onTrashMany(group);
      return;
    }
    if (allowAction(item, true)) onTrash(item);
  };

  /**
   * Dragging rows onto a folder of the tree or the list moves them there.
   * The drag follows pointer events and finds the folder under the pointer
   * with `elementFromPoint`, because the window's native drag-and-drop
   * handler is reserved for files dropped from the operating system and
   * keeps HTML drag events from reaching the page on some platforms.
   */
  const startDrag = (
    event: ReactPointerEvent<HTMLDivElement>,
    item: Item,
  ): void => {
    if (
      !onMoveMany ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      (event.target instanceof Element && event.target.closest('button'))
    )
      return;
    const dragged = groupOf(item) ?? [item];
    const own = storeOf(snapshot, item.store);
    const ownName = own?.name ?? 'this vault';
    const blocked = own
      ? storeBlocked(own)
      : 'This vault is no longer available.';
    const what =
      dragged.length === 1 ? nameOf(item.path) : itemCount(dragged.length);
    const origin = { x: event.clientX, y: event.clientY };
    let active = false;
    let target: { store: string; path: string } | null = null;
    let verdict: DropVerdict | null = null;
    const judge = (x: number, y: number): void => {
      const element =
        typeof document.elementFromPoint === 'function'
          ? document.elementFromPoint(x, y)
          : null;
      const folder = element?.closest<HTMLElement>('[data-folder-store]');
      const store = folder?.dataset.folderStore;
      const path = folder?.dataset.folderPath;
      target = store && path ? { store, path } : null;
      verdict = target
        ? dropVerdict(dragged, target.store, target.path, ownName, blocked)
        : null;
    };
    const onMove = (next: PointerEvent): void => {
      if (!active) {
        if (Math.hypot(next.clientX - origin.x, next.clientY - origin.y) < 6)
          return;
        active = true;
        document.body.classList.add('dragging-rows');
        window.getSelection()?.removeAllRanges();
      }
      judge(next.clientX, next.clientY);
      setDrag({
        x: next.clientX,
        y: next.clientY,
        what,
        verdict,
        target: target ? `${target.store}|${target.path}` : null,
      });
    };
    const finish = (drop: boolean): void => {
      endDrag.current = null;
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onCancel);
      window.removeEventListener('keydown', onKey, true);
      if (!active) return;
      document.body.classList.remove('dragging-rows');
      suppressClick.current = true;
      window.setTimeout(() => {
        suppressClick.current = false;
      }, 0);
      setDrag(null);
      if (drop && verdict?.ok && target) onMoveMany(dragged, target.path);
    };
    const onUp = (next: PointerEvent): void => {
      if (active) judge(next.clientX, next.clientY);
      finish(true);
    };
    const onCancel = (): void => finish(false);
    const onKey = (next: KeyboardEvent): void => {
      if (next.key !== 'Escape' || !active) return;
      next.preventDefault();
      next.stopPropagation();
      finish(false);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onCancel);
    window.addEventListener('keydown', onKey, true);
    endDrag.current = finish;
  };
  const dropOn = (storeId: string, path: string): 'ok' | 'no' | undefined =>
    drag?.target === `${storeId}|${path}`
      ? drag.verdict?.ok
        ? 'ok'
        : 'no'
      : undefined;

  const selectionBar =
    chosen.length && (onMoveMany || onTrashMany) ? (
      <div className="selbar" role="toolbar" aria-label="Selected items">
        <span className="n" aria-live="polite">
          {chosen.length} selected
        </span>
        {onMoveMany ? (
          <Button
            size="sm"
            icon="folder"
            disabled={Boolean(moveGroupReason(chosen))}
            title={moveGroupReason(chosen)}
            onClick={() => onMoveMany(chosen)}
          >
            Move to…
          </Button>
        ) : null}
        {onTrashMany ? (
          <Button
            size="sm"
            icon="trash"
            danger
            disabled={Boolean(trashGroupReason(chosen))}
            title={trashGroupReason(chosen)}
            onClick={() => onTrashMany(chosen)}
          >
            Move to Trash
          </Button>
        ) : null}
        <span className="sp" />
        <Button size="sm" onClick={() => setChosen(NO_ROWS)}>
          Clear
        </Button>
      </div>
    ) : null;

  /* ------------------------------------------------- keyboard movement -- */

  // The list or card grid is one tab stop. Its entries in order: the
  // selected folder's subfolders, then the listed items.
  const navKeys = trashSelected
    ? []
    : [
        ...(!flatMode && selectedTree
          ? folders.map((folder) =>
              folderNavKey(selectedTree.store.id, folder.path),
            )
          : []),
        ...listedKeys,
      ];
  const navStop = rovingKey(navKeys, rowFocus, seed);
  const navIndex = navStop === undefined ? -1 : navKeys.indexOf(navStop);
  const cardLayout = view === 'grid';
  const lineSize = cardLayout ? gridWindow.columns : 1;
  const cardHeight = columns.location
    ? FILES_CARD_HEIGHT_WITH_LOCATION
    : FILES_CARD_HEIGHT;
  const linePitch = cardLayout ? cardHeight + FILES_CARD_GAP : ROW_HEIGHT;
  // The column headers are the list grid's first row while they are drawn.
  const headerRows = !cardLayout && !selectionBar ? 1 : 0;
  const navFor = (key: string, index: number): RowNav => ({
    layout: cardLayout ? 'card' : 'list',
    key,
    tabIndex: key === navStop ? 0 : -1,
    rowIndex: cardLayout ? undefined : index + 1 + headerRows,
    colIndex: cardLayout ? (index % lineSize) + 1 : undefined,
  });
  /** Scrolls a windowed list so that the entry at `index` is in view. */
  const revealRow = (index: number): void => {
    const body = bodyRef.current;
    if (!flatMode || !body) return;
    const sticky =
      body.querySelector<HTMLElement>(
        '.list-window > .selbar, .files-items > .hdr',
      )?.offsetHeight ?? 0;
    const next = revealOffset({
      top: listMetrics.top + Math.floor(index / lineSize) * linePitch,
      height: cardLayout ? cardHeight : ROW_HEIGHT,
      scrollTop,
      viewport: listMetrics.viewport,
      stickyTop: sticky,
    });
    if (next !== scrollTop) {
      body.scrollTop = next;
      setScrollTop(next);
    }
  };
  /**
   * Moves focus to the entry at `index`, first scrolling a windowed list so
   * that the entry is mounted and in view.
   */
  const focusRow = (index: number): void => {
    const key = navKeys[index];
    if (key === undefined) return;
    const body = bodyRef.current;
    revealRow(index);
    setRowFocus(key);
    const element = navElement(body, key);
    if (element) element.focus({ preventScroll: flatMode });
    else pendingRowFocus.current = key;
  };
  const gridKeys = (event: ReactKeyboardEvent<HTMLDivElement>): void => {
    if (event.altKey || event.metaKey || event.ctrlKey || event.shiftKey)
      return;
    // Keys typed in a menu opened from a row reach this handler through
    // React's tree; only the grid's own entries count.
    const target = event.target;
    if (!(target instanceof HTMLElement)) return;
    const entry = target.closest<HTMLElement>('[data-nav-key]');
    if (!entry || !event.currentTarget.contains(entry)) return;
    const index = navKeys.indexOf(entry.dataset.navKey ?? '');
    if (index < 0) return;
    if (!cardLayout && matchesShortcut(event, 'rowActions')) {
      // In the list, Left and Right move between a row and its own actions.
      const stops = [
        entry,
        ...entry.querySelectorAll<HTMLElement>('.acts button'),
      ];
      const at = stops.indexOf(target);
      const next = stops[at + (event.key === 'ArrowRight' ? 1 : -1)];
      if (at < 0 || !next) return;
      event.preventDefault();
      next.focus();
      return;
    }
    const next = gridFocusTarget(event.key, {
      index,
      count: navKeys.length,
      columns: lineSize,
      page: Math.max(1, Math.floor(listMetrics.viewport / linePitch)),
    });
    if (next === null) return;
    event.preventDefault();
    focusRow(next);
  };
  const rowFocused = (event: ReactFocusEvent<HTMLDivElement>): void => {
    const entry =
      event.target instanceof HTMLElement
        ? event.target.closest<HTMLElement>('[data-nav-key]')
        : null;
    if (!entry?.dataset.navKey || !event.currentTarget.contains(entry)) return;
    setRowFocus(entry.dataset.navKey);
    // The tab stop outside the virtual window is drawn one line beyond its
    // edge, not where it belongs; focus returning to it (Tab after scrolling
    // the list away) scrolls the list to its real place.
    const index = navKeys.indexOf(entry.dataset.navKey);
    if (index >= 0 && (index < rowWindow.start || index >= rowWindow.end))
      revealRow(index);
  };
  /** The card grid's lines, from the line holding entry `first`. */
  const cardLines = (
    entries: readonly ReactNode[],
    first: number,
  ): ReactNode[] => {
    const lines: ReactNode[] = [];
    for (let at = 0; at < entries.length; at += lineSize) {
      const line = Math.floor((first + at) / lineSize);
      lines.push(
        <div
          key={line}
          className="files-grid-line"
          role="row"
          aria-rowindex={line + 1}
        >
          {entries.slice(at, at + lineSize)}
        </div>,
      );
    }
    return lines;
  };
  const cardGridStyle = {
    '--files-columns': lineSize,
  } as React.CSSProperties;

  const row = (item: Item, index: number): ReactNode => (
    <Row
      key={`${item.store}|${item.path}`}
      nav={navFor(rowKey(item), index)}
      snapshot={snapshot}
      hues={hues}
      item={item}
      columns={columns}
      selected={
        chosen.length
          ? chosenKeys.has(rowKey(item))
          : state.selection?.store === item.store &&
            state.selection.path === item.path
      }
      readOnly={readOnly}
      group={groupOf(item)?.length ?? 0}
      onSelect={() => {
        if (suppressClick.current) return;
        setChosen(NO_ROWS);
        locations.select({ store: item.store, path: item.path });
      }}
      onPick={(mode) => pickRow(item, mode)}
      onPointerDown={(event) => startDrag(event, item)}
      onCopy={copyItemField}
      onReveal={revealItem}
      onDownload={downloadItem}
      onMove={onMove || onMoveMany ? moveRow : undefined}
      onCopyToVault={onCopyToVault ? copyRow : undefined}
      onTrash={trashRow}
      accessNow={accessNow}
    />
  );

  const paneRows = selectedTree
    ? [
        ...folders.map((folder, index) => (
          <FolderRow
            nav={navFor(
              folderNavKey(selectedTree.store.id, folder.path),
              index,
            )}
            storeId={selectedTree.store.id}
            path={folder.path}
            key={folder.path}
            name={displayPathComponent(folder.name)}
            count={folder.count}
            columns={columns}
            drop={dropOn(selectedTree.store.id, folder.path)}
            onSelect={() => selectFolder(selectedTree.store, folder.path)}
          />
        )),
        ...folderScopedItems.map((item, index) =>
          row(item, folders.length + index),
        ),
      ]
    : [];

  const contextFolder = folderMenu
    ? snapshot.items.find(
        (item) =>
          item.store === folderMenu.storeId &&
          item.path === folderMenu.path &&
          item.kind === 'Folder',
      )
    : undefined;
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
    // Every row that opens this menu is its own focus target: a tree row, a
    // folder row or card, or the single vault's heading.
    row.focus({ preventScroll: true });
    folderMenuAnchor.current = row;
    setFolderMenu({
      x,
      y,
      storeId: row.dataset.folderStore,
      path: row.dataset.folderPath,
      trigger: row,
    });
    return true;
  };
  const vaultHeading = vaultTrees.length === 1 ? 'Your vault' : 'Vaults';
  const allCount = treeItems.length;
  /** The current view's name in the title row. */
  const scopeLabel =
    selected.store === ALL_ITEMS
      ? 'All items'
      : trashSelected
        ? `Trash · ${selectedStore?.name ?? ''}`
        : selected.path !== '/'
          ? nameOf(selected.path)
          : (selectedStore?.name ?? 'All items');
  const trashShown =
    trashSelected && selectedTree
      ? trashEntries(snapshot, selectedTree.store.id).filter(
          (entry) =>
            !state.query ||
            [entry.name, entry.original ?? '']
              .join('\n')
              .toLowerCase()
              .includes(state.query.toLowerCase()),
        )
      : [];
  const shownCount =
    selected.store === ALL_ITEMS
      ? items.length
      : trashSelected
        ? trashShown.length
        : (selectedNode?.count ?? 0);
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
          <kbd>{shortcutLabel('newPassword')}</kbd>
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
          <kbd>{shortcutLabel('newDocument')}</kbd>
        </button>
        {onImport ? (
          <button
            type="button"
            className="card"
            onClick={() => {
              if (createStore && currentStoreAvailable(createStore))
                onImport(createStore, createFolder);
            }}
          >
            <span className="ci">
              <Icon name="download" />
            </span>
            <b>Import passwords</b>
            <small>
              From a CSV export of 1Password, Bitwarden, KeePass or a browser.
            </small>
          </button>
        ) : null}
      </div>
      {/* Items live on the server, so an account already used with the FOKS
          CLI brings its items once it is connected; there is no CLI export. */}
      <p>
        Already use FOKS on the command line?{' '}
        <button
          type="button"
          className="lnk"
          onClick={() =>
            locations.navigate({
              kind: 'settings',
              section: 'account',
              ...(selectedStore ? { profile: selectedStore.server } : {}),
            })
          }
        >
          Connect that account
        </button>{' '}
        in Settings › Account › Import from FOKS CLI, and its items appear here.
      </p>
    </div>
  );

  const searchMiss = (
    <div className="empty">
      <div className="big">
        <Icon name="search" />
      </div>
      <h2>Nothing matches “{state.query}”</h2>
      <p>
        Try a shorter word, or press {shortcutLabel('search')} to search every
        vault and team.
      </p>
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

  const itemReadiness = itemsReadiness(
    snapshot,
    selected.store === ALL_ITEMS ? undefined : selected.store,
  );
  // The windowed list keeps its tab stop mounted after it scrolls out of the
  // window: the entry is drawn one line beyond the window's edge, in place of
  // that much spacer, so focus and the way back into the list survive.
  const pinnedItem =
    flatMode && (navIndex < rowWindow.start || navIndex >= rowWindow.end)
      ? items[navIndex]
      : undefined;
  const pinnedBefore = pinnedItem !== undefined && navIndex < rowWindow.start;
  const pinnedAfter = pinnedItem !== undefined && navIndex >= rowWindow.end;
  const pinnedEntry = pinnedItem ? row(pinnedItem, navIndex) : null;
  // Keyed as the line it belongs to, so the card keeps its element, and
  // with it focus, when that line leaves the window.
  const pinnedLine = pinnedItem ? (
    <div
      key={Math.floor(navIndex / lineSize)}
      className="files-grid-line"
      role="row"
      aria-rowindex={Math.floor(navIndex / lineSize) + 1}
    >
      {pinnedEntry}
    </div>
  ) : null;
  const padTop = rowWindow.padTop - (pinnedBefore ? linePitch : 0);
  const padBottom = Math.max(
    0,
    rowWindow.padBottom - (pinnedAfter ? linePitch : 0),
  );
  const windowEntries = visibleRows.map((item, index) =>
    row(item, rowWindow.start + index),
  );
  // One keyed list, so a row that leaves the window for the pinned place is
  // moved rather than mounted again.
  const listEntries = [
    ...(pinnedBefore ? [pinnedEntry] : []),
    ...windowEntries,
    ...(pinnedAfter ? [pinnedEntry] : []),
  ];
  const windowLines = [
    ...(pinnedBefore ? [pinnedLine] : []),
    ...cardLines(windowEntries, rowWindow.start),
    ...(pinnedAfter ? [pinnedLine] : []),
  ];
  const gridAttributes = {
    className: 'files-items',
    role: 'grid',
    'aria-label': state.query ? `Results for “${state.query}”` : scopeLabel,
    'aria-rowcount': cardLayout
      ? Math.ceil(navKeys.length / lineSize)
      : navKeys.length + headerRows,
    'aria-colcount': cardLayout ? lineSize : undefined,
    'aria-multiselectable': onMoveMany || onTrashMany ? true : undefined,
    onKeyDown: gridKeys,
    onFocus: rowFocused,
  };
  const listBody =
    trashSelected && selectedTree ? (
      <TrashView
        snapshot={snapshot}
        bridge={bridge}
        store={selectedTree.store}
        entries={trashShown}
        accessNow={accessNow}
        onRestore={(entry) => {
          if (allowAction(entry.item, true)) onRestore?.(entry);
        }}
        onDeletePermanently={(entry) => {
          if (allowAction(entry.item, true)) onDeletePermanently?.(entry.item);
        }}
        onEmptyTrash={() => onEmptyTrash?.(selectedTree.store.id)}
      />
    ) : !items.length && itemReadiness !== 'ready' ? (
      <CollectionStatus state={itemReadiness} label="items" />
    ) : flatMode ? (
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
          {selectionBar}
          <div {...gridAttributes}>
            {selectionBar ? null : contentsHeader}
            <div className="virtual-rows" ref={listRef}>
              {padTop ? (
                <div className="virtual-spacer" style={{ height: padTop }} />
              ) : null}
              {cardLayout ? (
                <div
                  className="files-grid"
                  style={{
                    ...cardGridStyle,
                    paddingBottom: gridWindow.trailingGap,
                  }}
                >
                  {windowLines}
                </div>
              ) : (
                listEntries
              )}
              {padBottom ? (
                <div className="virtual-spacer" style={{ height: padBottom }} />
              ) : null}
            </div>
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
        {selectionBar}
        <div {...gridAttributes}>
          {selectionBar ? null : contentsHeader}
          {cardLayout ? (
            <div className="files-grid" ref={listRef} style={cardGridStyle}>
              {cardLines(paneRows, 0)}
            </div>
          ) : (
            <div className="virtual-rows" ref={listRef}>
              {paneRows}
            </div>
          )}
        </div>
        {nextUp}
      </div>
    );
  const listShown =
    !trashSelected && (flatMode ? items.length > 0 : !showEmptyFolder);

  return (
    <div className="drop-area">
      <ChosenRowsFocus
        count={chosen.length}
        anchor={chosenRows.anchor}
        first={chosen[0] ? listedKeys.indexOf(rowKey(chosen[0])) : -1}
        listed={listedKeys}
        body={bodyRef}
      />
      {dropZone}
      <div
        ref={sidebar.ref}
        style={sidebar.style}
        className="folder-layout"
        onContextMenu={(event) => {
          if (showFolderMenu(event.target, event.clientX, event.clientY))
            event.preventDefault();
        }}
        onKeyDown={(event) => {
          if (!matchesShortcut(event, 'treeMenu')) return;
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
              {folderMenu.path !== '/' && onMove ? (
                <MenuItem
                  reason={
                    contextFolder
                      ? itemActionProblem(
                          snapshot,
                          contextFolder,
                          true,
                          accessNow(),
                        )
                      : 'Refresh this vault to load the folder before moving it.'
                  }
                  onClick={() => {
                    setFolderMenu(null);
                    if (contextFolder && allowAction(contextFolder, true))
                      onMove(contextFolder);
                  }}
                >
                  Rename or move…
                </MenuItem>
              ) : null}
              {folderMenu.path !== '/' ? (
                <MenuItem
                  icon="trash"
                  danger
                  reason={
                    contextFolder
                      ? itemActionProblem(
                          snapshot,
                          contextFolder,
                          true,
                          accessNow(),
                        )
                      : 'Refresh this vault to load the folder before moving it to Trash.'
                  }
                  onClick={() => {
                    setFolderMenu(null);
                    if (contextFolder && allowAction(contextFolder, true))
                      onTrash(contextFolder);
                  }}
                >
                  Move folder to Trash
                </MenuItem>
              ) : null}
              {folderMenu.path === '/' && contextStore && onImport ? (
                <MenuItem
                  icon="download"
                  reason={
                    !availability(contextStore).available
                      ? storeDescription(snapshot, contextStore)
                      : (writeBlockReason(snapshot, contextStore) ?? undefined)
                  }
                  onClick={() => {
                    setFolderMenu(null);
                    onImport(contextStore.id);
                  }}
                >
                  Import passwords…
                </MenuItem>
              ) : null}
              {folderMenu.path === '/' && contextStore && onExport ? (
                <MenuItem
                  icon="upload"
                  reason={
                    !availability(contextStore).available
                      ? storeDescription(snapshot, contextStore)
                      : exportedPasswords(snapshot, contextStore.id)
                        ? undefined
                        : 'This vault has no passwords to export.'
                  }
                  onClick={() => {
                    setFolderMenu(null);
                    onExport(contextStore.id);
                  }}
                >
                  Export passwords…
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
                  <MenuItem
                    icon="settings"
                    onClick={() => {
                      setFolderMenu(null);
                      onSettings(contextStore.id);
                    }}
                  >
                    Team settings
                  </MenuItem>
                  <div role="separator" className="menu-separator" />
                  {(
                    [
                      ['add', 'user', 'Add FOKS user…', 'roster'],
                      ['add-team', 'users', 'Add FOKS team…', 'federation'],
                      ['invite', 'door', 'Invite new user…', 'roster'],
                    ] as const
                  ).map(([intent, icon, label, source]) => (
                    <MenuItem
                      key={intent}
                      icon={icon}
                      reason={manageReason(snapshot, contextStore, source)}
                      onClick={() => {
                        setFolderMenu(null);
                        locations.navigate({
                          kind: 'teams',
                          ref: contextStore.id,
                          open: intent,
                        });
                      }}
                    >
                      {label}
                    </MenuItem>
                  ))}
                </>
              ) : null}
            </Menu>
          </ContextMenu>
        ) : null}
        <div className="folder-split">
          {sidebar.handle}
          <aside id={sidebar.id} className="tpane" aria-label="Folders">
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
              {vaultTrees.length ? (
                <div
                  role="tree"
                  aria-label={vaultHeading}
                  onKeyDown={treeKeys(vaultEntries)}
                  onFocus={treeFocused}
                >
                  {vaultTrees.map((tree) => storeRoot(tree, vaultStop))}
                </div>
              ) : null}
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
              {teamTrees.length ? (
                <div
                  role="tree"
                  aria-label="Teams"
                  onKeyDown={treeKeys(teamEntries)}
                  onFocus={treeFocused}
                >
                  {teamTrees.map((tree) => storeRoot(tree, teamStop))}
                </div>
              ) : (
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
              )}
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
                      {itemReadiness === 'ready'
                        ? itemCount(shownCount)
                        : `${shownCount} items loaded`}
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
              onKeyDown={(event) => {
                // Keys typed in a menu or sheet opened from a row reach this
                // handler through React's tree; only the list's own count.
                if (
                  !(event.target instanceof Node) ||
                  !event.currentTarget.contains(event.target)
                )
                  return;
                if (matchesShortcut(event, 'rowsClear') && chosen.length) {
                  event.preventDefault();
                  setChosen(NO_ROWS);
                  return;
                }
                if (
                  matchesShortcut(event, 'rowsAll') &&
                  listed.length &&
                  (onMoveMany || onTrashMany)
                ) {
                  event.preventDefault();
                  setChosen(allRows(listedKeys));
                }
              }}
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
              {items.length > 0 ? (
                <CollectionStatus
                  state={itemReadiness}
                  label="remaining items"
                />
              ) : null}
              {listBody}
            </div>
            {listShown ? dropStrip : null}
          </section>
        </div>
      </div>
      {drag ? (
        <div
          className="drag-ghost"
          style={{ left: drag.x + 14, top: drag.y + 14 }}
          aria-hidden="true"
        >
          <span className="what">
            <Icon name="file" />
            {drag.what}
          </span>
          <span className={drag.verdict?.ok ? 'lab' : 'lab no'}>
            {drag.verdict?.label ?? 'Drop on a folder to move'}
          </span>
        </div>
      ) : null}
    </div>
  );
}
