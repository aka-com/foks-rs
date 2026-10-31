/**
 * The Trash sheets: Empty Trash for one vault, and restoring an entry whose
 * original path another item now holds.
 */

import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Icon, Inset, InsetRow, SheetDialog } from '../components';
import {
  nameOf,
  restoredCopyPath,
  storeOf,
  trashEntries,
  trashEntryLabel,
  trashRemovalProblem,
} from '../model';
import type { AgentSnapshot, Item } from '../model';
import type { Bridge } from '../bridge';
import type { MutationFailureHandler } from '../mutation-recovery';
import { useSheetGuard } from '../navigation-guard';
import { itemActionProblem } from './store-access';
import { useTrashOrigins } from './trash-origins';
import type { WriteWorkflow } from './write-workflows';

/** What permanent deletion says it does, in every sheet that offers it. */
export const PERMANENT_DELETE_COPY =
  'This removes it for everyone and cannot be undone. Earlier encrypted versions may remain on the server.';

/** Why a team member is offered permanent deletion instead of Trash. */
export const TRASH_UNAVAILABLE_COPY =
  'Trash is not set up in this team yet, and only an owner can set it up by moving any item to Trash.';

/**
 * Permanently deletes every entry in one store's Trash that this account can
 * change, folders with everything in them, in one host mutation. Entries
 * whose write role this account lacks stay in Trash.
 */
export function EmptyTrashSheet({
  snapshot,
  bridge,
  workflow,
  setWorkflow,
  onApplied,
  onMutationError,
  onBeforeRemoval,
  accessNow,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'empty-trash' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onApplied: (message: string, profile?: string) => Promise<void>;
  onMutationError: MutationFailureHandler;
  /** Resolves once automatic emptying, if it is running, has finished. */
  onBeforeRemoval: () => Promise<void>;
  accessNow: () => number;
}): ReactNode {
  const [listed] = useState(() => trashEntries(snapshot, workflow.store));
  const entries = useTrashOrigins(bridge, listed);
  const [emptying, setEmptying] = useState(false);
  const store = storeOf(snapshot, workflow.store);
  const canRemove = (item: Item): boolean =>
    !itemActionProblem(snapshot, item, true, accessNow());
  // A folder goes with everything in it, so it stays when anything under it
  // is an entry this account could not remove.
  const problems = entries.map(
    (entry) =>
      itemActionProblem(snapshot, entry.item, true, accessNow()) ??
      trashRemovalProblem(snapshot, entry, canRemove),
  );
  const removable = entries.filter((_, index) => !problems[index]);
  const kept = entries.length - removable.length;
  const problem = removable.length ? undefined : problems[0];
  useSheetGuard(
    emptying
      ? { verdict: 'refuse', reason: 'Wait for Trash to finish emptying.' }
      : null,
    !emptying,
  );
  const count = (amount: number): string =>
    `${amount} ${amount === 1 ? 'Item' : 'Items'}`;
  const empty = (): void => {
    if (emptying || !removable.length) return;
    setEmptying(true);
    void (async () => {
      try {
        await onBeforeRemoval();
        await bridge.emptyTrash({
          storeId: workflow.store,
          entries: removable.map(({ item }) => ({
            path: item.path,
            version: item.version,
            direntId: item.direntId,
          })),
        });
      } catch (error) {
        setWorkflow(null);
        await onMutationError(error);
        return;
      } finally {
        setEmptying(false);
      }
      setWorkflow(null);
      try {
        await onApplied(
          `Emptied Trash in ${store?.name ?? 'this vault'}`,
          store?.server,
        );
      } catch (failure) {
        await onMutationError(failure, { report: false });
      }
    })();
  };
  return (
    <SheetDialog
      danger
      onClose={() => setWorkflow(null)}
      glyph={
        <span className="kico md danger">
          <Icon name="trash" />
        </span>
      }
      dismissible={!emptying}
      title={`Permanently Delete ${count(removable.length)}?`}
      footer={
        <>
          <Button
            disabled={emptying}
            data-dialog-autofocus="true"
            onClick={() => setWorkflow(null)}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            danger
            disabled={emptying || !removable.length}
            title={problem}
            onClick={empty}
          >
            {emptying ? 'Deleting…' : 'Empty Trash'}
          </Button>
        </>
      }
    >
      {problem ? (
        <p className="action-error" role="status">
          {problem}
        </p>
      ) : null}
      <p>{PERMANENT_DELETE_COPY}</p>
      {kept && removable.length ? (
        <p>
          {kept} {kept === 1 ? 'item stays' : 'items stay'} in Trash because
          this account cannot remove {kept === 1 ? 'it' : 'them'}, or an entry
          inside {kept === 1 ? 'it' : 'them'}.
        </p>
      ) : null}
      <ul className="trash-empty-list">
        {entries.map((entry, index) => (
          <li
            key={entry.item.path}
            className={problems[index] ? 'kept' : undefined}
          >
            <Icon name={entry.item.kind === 'Folder' ? 'folder' : 'file'} />
            <span>{trashEntryLabel(entry)}</span>
            {entry.item.kind === 'Folder' ? (
              <small>
                folder · {entry.count} {entry.count === 1 ? 'item' : 'items'}
              </small>
            ) : null}
            {problems[index] ? <small>stays in Trash</small> : null}
          </li>
        ))}
      </ul>
    </SheetDialog>
  );
}

/**
 * Restoring an entry from Trash found another item at its original path.
 * The entry can be restored under a new name beside it; nothing is replaced.
 */
export function RestoreConflictSheet({
  snapshot,
  workflow,
  setWorkflow,
  onRestoreTrash,
}: {
  snapshot: AgentSnapshot;
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'restore-conflict' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onRestoreTrash: (
    item: Item,
    destination: string,
  ) => Promise<'restored' | 'sheet' | 'failed'>;
}): ReactNode {
  const [restoring, setRestoring] = useState(false);
  const { item, destination } = workflow;
  const copy = restoredCopyPath(destination, (path) =>
    snapshot.items.some(
      (candidate) => candidate.store === item.store && candidate.path === path,
    ),
  );
  const occupant = snapshot.items.find(
    (candidate) =>
      candidate.store === item.store && candidate.path === destination,
  );
  useSheetGuard(
    restoring
      ? { verdict: 'refuse', reason: 'Wait for the restore to finish.' }
      : null,
    !restoring,
  );
  return (
    <SheetDialog
      onClose={() => setWorkflow(null)}
      dismissible={!restoring}
      glyph={
        <span className="kico md">
          <Icon name="restore" />
        </span>
      }
      title={`${nameOf(destination)} Is Already in Use`}
      footer={
        <>
          <Button
            disabled={restoring}
            data-dialog-autofocus="true"
            onClick={() => setWorkflow(null)}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={restoring}
            onClick={() => {
              if (restoring) return;
              setRestoring(true);
              void onRestoreTrash(item, copy).then(
                (outcome) => {
                  setRestoring(false);
                  // Another sheet that took over, such as a conflict on the
                  // new name, stays; a failure keeps this one for Cancel.
                  if (outcome === 'restored') setWorkflow(null);
                },
                () => setRestoring(false),
              );
            }}
          >
            {restoring ? 'Restoring…' : `Restore as ${nameOf(copy)}`}
          </Button>
        </>
      }
    >
      <p>
        Another {occupant?.kind === 'Folder' ? 'folder' : 'item'} is at{' '}
        <code>{destination}</code>. The entry from Trash can be restored beside
        it under a new name; neither is replaced.
      </p>
      <Inset>
        <InsetRow label="In use">
          <code>{destination}</code>
        </InsetRow>
        <InsetRow label="Restores to">
          <code>{copy}</code>
        </InsetRow>
      </Inset>
    </SheetDialog>
  );
}
