/**
 * One store's Trash: the entries moved into its `/.trash` role folders,
 * newest first, each with Restore and Delete permanently. Where an entry was
 * and when it was deleted come from its name in Trash, or for a hashed name
 * from the `.origin` file beside it.
 */

import { useState } from 'react';
import type {
  FocusEvent as ReactFocusEvent,
  KeyboardEvent as ReactKeyboardEvent,
  ReactNode,
} from 'react';
import { Band, Button, Icon } from '../components';
import {
  fmtSize,
  formatRole,
  ownersOnly,
  parseRole,
  trashEntries,
  trashEntryLabel,
  trashUnavailable,
} from '../model';
import type { AgentSnapshot, Store, TrashEntry } from '../model';
import type { Bridge } from '../bridge';
import { matchesShortcut } from '../shortcuts';
import { gridFocusTarget, rovingKey } from './files-keyboard';
import { itemActionProblem } from './store-access';
import { useTrashOrigins } from './trash-origins';

function deletedLabel(deletedAt: number | null): string {
  if (deletedAt === null) return '—';
  return new Date(deletedAt).toLocaleString(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  });
}

function sizeLabel(entry: TrashEntry): string {
  if (entry.item.kind !== 'Folder') return fmtSize(entry.item.size);
  return `${entry.count} ${entry.count === 1 ? 'item' : 'items'}`;
}

/** Where an entry was, as its row shows it. */
function originalLabel(entry: TrashEntry): string {
  if (entry.original === null)
    return entry.origin ? 'Recorded beside the entry' : 'Not recorded';
  const cut = entry.original.lastIndexOf('/');
  return cut <= 0 ? '/' : entry.original.slice(0, cut);
}

export function TrashView({
  snapshot,
  bridge,
  store,
  entries: listed,
  accessNow,
  onRestore,
  onDeletePermanently,
  onEmptyTrash,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  store: Store;
  entries: readonly TrashEntry[];
  accessNow: () => number;
  onRestore: (entry: TrashEntry) => void;
  onDeletePermanently: (entry: TrashEntry) => void;
  onEmptyTrash: () => void;
}): ReactNode {
  const entries = useTrashOrigins(bridge, listed);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const selected = entries.find((entry) => entry.item.path === selectedPath);
  // The list is one tab stop, as the Files list is: the row focus last moved
  // to, else the selected row, else the first carries tabIndex 0.
  const [focusPath, setFocusPath] = useState<string | null>(null);
  const paths = entries.map((entry) => entry.item.path);
  const stop = rovingKey(paths, focusPath, selectedPath);
  const gridKeys = (event: ReactKeyboardEvent<HTMLDivElement>): void => {
    if (event.altKey || event.metaKey || event.ctrlKey || event.shiftKey)
      return;
    const target = event.target;
    if (!(target instanceof HTMLElement)) return;
    const row = target.closest<HTMLElement>('[data-trash-path]');
    if (!row || !event.currentTarget.contains(row)) return;
    if (matchesShortcut(event, 'trashActions')) {
      // Left and Right move between a row and its enabled actions.
      const stops = [
        row,
        ...row.querySelectorAll<HTMLElement>('.trash-acts button:enabled'),
      ];
      const at = stops.indexOf(target);
      const next = stops[at + (event.key === 'ArrowRight' ? 1 : -1)];
      if (at < 0 || !next) return;
      event.preventDefault();
      next.focus();
      return;
    }
    const rows = [
      ...event.currentTarget.querySelectorAll<HTMLElement>('[data-trash-path]'),
    ];
    const scroller = row.closest<HTMLElement>('.folder-body');
    const next = gridFocusTarget(event.key, {
      index: rows.indexOf(row),
      count: rows.length,
      columns: 1,
      page: Math.max(
        1,
        Math.floor((scroller?.clientHeight ?? 0) / (row.offsetHeight || 1)),
      ),
    });
    if (next === null) return;
    event.preventDefault();
    setFocusPath(rows[next].dataset.trashPath ?? null);
    rows[next].focus();
  };
  const rowFocused = (event: ReactFocusEvent<HTMLDivElement>): void => {
    const row =
      event.target instanceof HTMLElement
        ? event.target.closest<HTMLElement>('[data-trash-path]')
        : null;
    if (row?.dataset.trashPath) setFocusPath(row.dataset.trashPath);
  };
  // Empty Trash removes the entries this account can change and leaves the
  // rest; it is unavailable only when there are none.
  const all = trashEntries(snapshot, store.id);
  const problems = all.map((entry) =>
    itemActionProblem(snapshot, entry.item, true, accessNow()),
  );
  const emptyProblem = problems.some((problem) => !problem)
    ? undefined
    : problems[0];
  return (
    <div className="trash-view">
      {trashUnavailable(snapshot, store.id) ? (
        <Band severity="info" label="Trash is not set up">
          Trash is not set up in this team yet. An owner sets it up by moving
          any item to Trash. Until then, deleting an item removes it permanently
          after you confirm, and folders cannot be deleted.
        </Band>
      ) : null}
      {store.kind === 'team' ? (
        <Band severity="info" label="Trash access">
          Items keep their read access in Trash. An item from a folder with
          narrower read access is filed with the access of that folder, so each
          member sees only the entries their role could reach before.
        </Band>
      ) : null}
      <div className="trash-bar">
        <span>
          {entries.length
            ? 'Newest first. Restore moves an entry back to where it was.'
            : ''}
        </span>
        <Button
          size="sm"
          danger
          icon="trash"
          disabled={!all.length || Boolean(emptyProblem)}
          title={emptyProblem}
          onClick={onEmptyTrash}
        >
          Empty Trash…
        </Button>
      </div>
      {entries.length ? (
        <div
          className="list-window"
          role="grid"
          aria-label={`Trash · ${store.name}`}
          aria-rowcount={entries.length + 1}
          onKeyDown={gridKeys}
          onFocus={rowFocused}
        >
          <div className="hdr trash-cols" role="row" aria-rowindex={1}>
            <span role="columnheader">Name</span>
            <span role="columnheader">Original location</span>
            <span role="columnheader">Deleted</span>
            <span role="columnheader" className="trash-size">
              Size
            </span>
            {/* Names the actions cell; the column has no visible heading. */}
            <span role="columnheader" className="acts-head">
              Actions
            </span>
          </div>
          {entries.map((entry, index) => {
            const label = trashEntryLabel(entry);
            const problem = itemActionProblem(
              snapshot,
              entry.item,
              true,
              accessNow(),
            );
            return (
              <div
                key={entry.item.path}
                className={[
                  'row',
                  'one',
                  'trash-cols',
                  selectedPath === entry.item.path ? 'sel' : '',
                ]
                  .filter(Boolean)
                  .join(' ')}
                role="row"
                aria-rowindex={index + 2}
                aria-selected={selectedPath === entry.item.path}
                aria-label={`${label}, deleted ${deletedLabel(entry.deletedAt)}`}
                tabIndex={entry.item.path === stop ? 0 : -1}
                data-trash-path={entry.item.path}
                onClick={() => setSelectedPath(entry.item.path)}
                onKeyDown={(event) => {
                  if (event.target !== event.currentTarget) return;
                  if (matchesShortcut(event, 'trashSelect')) {
                    event.preventDefault();
                    setSelectedPath(entry.item.path);
                  } else if (
                    matchesShortcut(event, 'trashDelete') &&
                    !problem
                  ) {
                    event.preventDefault();
                    onDeletePermanently(entry);
                  }
                }}
              >
                <span className="name" role="gridcell">
                  <Icon
                    name={entry.item.kind === 'Folder' ? 'folder' : 'file'}
                    className="f"
                  />
                  <span className="nm" title={entry.original ?? entry.name}>
                    {label}
                  </span>
                  {store.kind === 'team' && ownersOnly(entry) ? (
                    <span className="trash-lock" title="Owners only">
                      <Icon name="lock" />
                      Owners only
                    </span>
                  ) : null}
                </span>
                <span
                  className="cell"
                  role="gridcell"
                  title={entry.original ?? undefined}
                >
                  {originalLabel(entry)}
                </span>
                <span className="cell" role="gridcell">
                  {deletedLabel(entry.deletedAt)}
                </span>
                <span className="cell trash-size" role="gridcell">
                  {sizeLabel(entry)}
                </span>
                <span className="trash-acts" role="gridcell">
                  <Button
                    size="sm"
                    icon="restore"
                    // Reached with the arrow keys from the row, which is
                    // the tab stop.
                    tabIndex={-1}
                    disabled={Boolean(problem)}
                    title={problem ?? `Restore ${label}`}
                    aria-label={`Restore ${label}`}
                    onClick={(event) => {
                      event.stopPropagation();
                      onRestore(entry);
                    }}
                  >
                    Restore
                  </Button>
                  <Button
                    size="sm"
                    variant="quiet"
                    danger
                    icon="trash"
                    tabIndex={-1}
                    disabled={Boolean(problem)}
                    title={problem ?? `Delete ${label} permanently`}
                    aria-label={`Delete ${label} permanently`}
                    onClick={(event) => {
                      event.stopPropagation();
                      onDeletePermanently(entry);
                    }}
                  />
                </span>
              </div>
            );
          })}
        </div>
      ) : (
        <div className="empty">
          <div className="big">
            <Icon name="trash" />
          </div>
          <h2>Trash is empty</h2>
          <p>
            Items you delete in {store.name} are moved here. They can be
            restored until they are deleted permanently.
          </p>
        </div>
      )}
      {selected ? (
        <p className="trash-detail" role="status">
          Stored at <code>{selected.item.path}</code>
          {' · Readable by '}
          {(() => {
            const role = parseRole(selected.item.read);
            return role ? formatRole(role) : 'an unknown role';
          })()}
        </p>
      ) : null}
    </div>
  );
}
