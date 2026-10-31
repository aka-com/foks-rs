/**
 * One store's Trash: the entries moved into its `/.trash` role folders,
 * newest first, each with Restore and Delete Permanently. Where an entry was
 * and when it was deleted come from its name in Trash, or for a hashed name
 * from the `.origin` file beside it.
 */

import type { ReactNode } from 'react';
import { Button, EmptyState, Icon, Notice } from '../components';
import {
  fmtSize,
  ownersOnly,
  trashEntries,
  trashEntryLabel,
  trashUnavailable,
} from '../model';
import type { AgentSnapshot, Store, TrashEntry } from '../model';
import type { Bridge } from '../bridge';
import { itemTile } from './item-tile';
import { itemActionProblem } from './store-access';
import { useTrashOrigins } from './trash-origins';

function deletedLabel(deletedAt: number | null): string {
  if (deletedAt === null) return 'Deletion time not recorded';
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
    return entry.origin ? 'Location recorded beside the entry' : 'Not recorded';
  const cut = entry.original.lastIndexOf('/');
  return cut <= 0 ? 'Top level' : entry.original.slice(0, cut);
}

/** How many entries a store's Trash holds, and why Empty Trash is off, if it is. */
export function emptyTrashProblem(
  snapshot: AgentSnapshot,
  store: Store,
  accessNow: () => number,
): { count: number; problem: string | undefined } {
  // Empty Trash removes the entries this account can change and leaves the
  // rest; it is unavailable only when there are none.
  const all = trashEntries(snapshot, store.id);
  const problems = all.map((entry) =>
    itemActionProblem(snapshot, entry.item, true, accessNow()),
  );
  return {
    count: all.length,
    problem: problems.some((problem) => !problem) ? undefined : problems[0],
  };
}

export function TrashView({
  snapshot,
  bridge,
  store,
  entries: listed,
  accessNow,
  onRestore,
  onDeletePermanently,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  store: Store;
  entries: readonly TrashEntry[];
  accessNow: () => number;
  onRestore: (entry: TrashEntry) => void;
  onDeletePermanently: (entry: TrashEntry) => void;
}): ReactNode {
  const entries = useTrashOrigins(bridge, listed);
  return (
    <div className="trash-view">
      {trashUnavailable(snapshot, store.id) ? (
        <Notice severity="info" title="Trash is not set up">
          <p>
            An owner sets up Trash in this team by moving any item to Trash.
            Until then, deleting an item removes it permanently after you
            confirm, and folders cannot be deleted.
          </p>
        </Notice>
      ) : store.kind === 'team' ? (
        <Notice severity="info" title="Items keep their read access">
          <p>
            An item from a folder with narrower read access is filed with the
            access of that folder, so each member sees only the entries their
            role could read before.
          </p>
        </Notice>
      ) : null}
      {entries.length ? (
        <div
          className="trash-rows"
          role="list"
          aria-label={`Trash · ${store.name}`}
        >
          {entries.map((entry) => {
            const label = trashEntryLabel(entry);
            const problem = itemActionProblem(
              snapshot,
              entry.item,
              true,
              accessNow(),
            );
            const tile =
              entry.item.kind === 'Folder'
                ? { className: 'folder', icon: 'folder' as const }
                : itemTile(entry.item);
            return (
              <div
                key={entry.item.path}
                className="row trash-row"
                role="listitem"
                data-trash-path={entry.item.path}
              >
                <span className={`tile ${tile.className}`} aria-hidden="true">
                  <Icon name={tile.icon} />
                </span>
                <span className="name">
                  <b className="nm" title={entry.original ?? entry.name}>
                    {label}
                  </b>
                  <span className="offscreen">, </span>
                  <small title={entry.original ?? undefined}>
                    {originalLabel(entry)} · {deletedLabel(entry.deletedAt)} ·{' '}
                    {sizeLabel(entry)}
                    {store.kind === 'team' && ownersOnly(entry)
                      ? ' · Owners only'
                      : ''}
                  </small>
                </span>
                <span className="trash-acts">
                  <Button
                    size="sm"
                    icon="restore"
                    disabled={Boolean(problem)}
                    title={problem ?? `Restore ${label}`}
                    aria-label={`Restore ${label}`}
                    onClick={() => onRestore(entry)}
                  >
                    Restore
                  </Button>
                  <Button
                    size="sm"
                    variant="quiet"
                    danger
                    icon="trash"
                    disabled={Boolean(problem)}
                    title={problem ?? `Delete ${label} Permanently`}
                    aria-label={`Delete ${label} Permanently`}
                    onClick={() => onDeletePermanently(entry)}
                  />
                </span>
              </div>
            );
          })}
        </div>
      ) : (
        <EmptyState
          inList
          icon="trash"
          title="Trash Is Empty"
          body={`Items you delete in ${store.name} are moved here. They can be restored until they are deleted permanently.`}
        />
      )}
    </div>
  );
}
