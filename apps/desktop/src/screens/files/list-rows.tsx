/**
 * The rows of the Files list and card grid: the column header, an item's row
 * or card, and a subfolder's row, with the roving-focus attributes they share.
 */

import { useRef, useState } from 'react';
import type { PointerEvent as ReactPointerEvent, ReactNode } from 'react';
import {
  anyDialogOpen,
  ContextMenu,
  Menu,
} from '../../../kit/overlay-primitives';
import { Icon, MenuItem, StoreMark } from '../../components';
import type { FilterKind } from '../../components';
import type { FoksIconName } from '../../icons';
import {
  fmtSize,
  kindLabel,
  kindOf,
  nameOf,
  prefixOf,
  storeOf,
} from '../../model';
import type { Item, AgentSnapshot } from '../../model';
import type { SortKey, SortDirection } from '../../location';
import type { ItemField } from '../../bridge';
import { copyActions } from '../item-copy';
import { whereOf } from '../../model/scope';
import { copyProblem, itemCount, rowKey } from '../item-moves';
import { matchesShortcut } from '../../shortcuts';
import type { KeyPress } from '../../shortcuts';
import { itemActionProblem } from '../store-access';

/** The table's column set: Location is drawn only while rows span stores. */
export interface Columns {
  location: boolean;
}

function columnClass(columns: Columns, ...more: string[]): string {
  return ['cols', columns.location ? 'loc' : '', ...more]
    .filter(Boolean)
    .join(' ');
}

export function ListHeader({
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
export interface RowNav {
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
export function folderNavKey(storeId: string, path: string): string {
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

export function Row({
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

/** A subfolder of the selected folder, listed above its items. */
export function FolderRow({
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
