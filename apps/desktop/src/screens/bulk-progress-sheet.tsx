import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { anyDialogOpen } from '../../kit/overlay-primitives';
import { useToast } from '../../kit/toasts';
import { Button, Icon, SheetDialog } from '../components';
import type { Bridge } from '../bridge';
import type { FoksIconName } from '../icons';
import {
  isTrashPath,
  nameOf,
  storeOf,
  trashUnavailable,
  UNDO_DURATION_MS,
} from '../model';
import type { AgentSnapshot, Item } from '../model';
import { noteMoved } from '../recent-favorites';
import { useSheetGuard } from '../navigation-guard';
import {
  baseName,
  bulkCounts,
  inFolder,
  itemCount,
  joinPath,
  moveRowCheck,
  parentOf,
  pendingRows,
  rowKey,
  runBulk,
  toFolder,
} from './item-moves';
import type { BulkAction, BulkRow, BulkStatus } from './item-moves';
import { itemActionProblem } from './store-access';
import type { WriteWorkflow } from './write-workflows';
import { useShellServices, useShellSnapshot } from '../shell-services';

export type BulkWorkflow = Extract<
  NonNullable<WriteWorkflow>,
  { kind: 'bulk-move' } | { kind: 'bulk-trash' } | { kind: 'bulk-restore' }
>;

type RestoreWorkflow = Extract<BulkWorkflow, { kind: 'bulk-restore' }>;

const STATUS_ICON: Readonly<
  Record<Exclude<BulkStatus, 'running'>, FoksIconName>
> = {
  pending: 'ellipsis',
  done: 'circleCheck',
  skipped: 'check',
  refused: 'alert',
  failed: 'close',
  uncertain: 'info',
};

/** Why an item is not moved to Trash with the others. */
const TRASH_NOT_SET_UP =
  'Trash is not set up in this team yet. Delete this item on its own to remove it permanently.';

/**
 * The request each row of `workflow` sends. A move to Trash records the
 * entry's path in `trashed`, under the item's row key, for its Undo.
 */
function bulkAction(
  workflow: BulkWorkflow,
  bridge: Bridge,
  accessNow: () => number,
  trashed: Map<string, string>,
): BulkAction {
  const writeProblem = (item: Item, snapshot: AgentSnapshot) => {
    const problem = itemActionProblem(snapshot, item, true, accessNow());
    return problem ? { status: 'refused' as const, message: problem } : null;
  };
  if (workflow.kind === 'bulk-move') {
    const { folder } = workflow;
    return {
      running: 'Moving…',
      check: (item, snapshot) =>
        moveRowCheck(item, snapshot, folder) ?? writeProblem(item, snapshot),
      apply: async (item) => {
        const destination = joinPath(folder, baseName(item.path));
        await bridge.moveItem({
          direntId: item.direntId,
          storeId: item.store,
          path: item.path,
          version: item.version,
          destination,
        });
        noteMoved(item, destination);
        return `Moved ${toFolder(folder)}`;
      },
    };
  }
  if (workflow.kind === 'bulk-restore') {
    // Rows show the items as they were; each acts on its entry in Trash.
    const entryPath = new Map(
      workflow.items.map((item, index) => [
        rowKey(item),
        workflow.trash[index],
      ]),
    );
    const original = new Map(
      workflow.items.map((item, index) => [
        rowKey({ store: item.store, path: workflow.trash[index] ?? '' }),
        item.path,
      ]),
    );
    return {
      running: 'Restoring…',
      locate: (item, snapshot) => {
        const path = entryPath.get(rowKey(item));
        return snapshot.items.find(
          (entry) => entry.store === item.store && entry.path === path,
        );
      },
      check: (entry, snapshot) => {
        const destination = original.get(rowKey(entry));
        if (!destination)
          return { status: 'refused', message: 'This item is not in Trash.' };
        if (
          snapshot.items.some(
            (item) => item.store === entry.store && item.path === destination,
          )
        )
          return {
            status: 'refused',
            message: `Something named ${nameOf(destination)} is already ${inFolder(parentOf(destination))}. Restore it from Trash to choose another name.`,
          };
        return writeProblem(entry, snapshot);
      },
      apply: async (entry) => {
        const destination = original.get(rowKey(entry)) ?? entry.path;
        await bridge.restoreTrashItem({
          storeId: entry.store,
          path: entry.path,
          version: entry.version,
          direntId: entry.direntId,
          destination,
        });
        return `Restored ${toFolder(parentOf(destination))}`;
      },
    };
  }
  return {
    running: 'Moving to Trash…',
    check: (item, snapshot) => {
      if (isTrashPath(item.path))
        return { status: 'skipped', message: 'Already in Trash' };
      if (trashUnavailable(snapshot, item.store))
        return { status: 'refused', message: TRASH_NOT_SET_UP };
      return writeProblem(item, snapshot);
    },
    apply: async (item) => {
      const { path } = await bridge.trashItem({
        storeId: item.store,
        path: item.path,
        version: item.version,
        direntId: item.direntId,
      });
      trashed.set(rowKey(item), path);
      return 'Moved to Trash';
    },
  };
}

/** What a finished run did, in the words the toast and the title use. */
function finishedSummary(
  workflow: BulkWorkflow,
  rows: readonly BulkRow[],
): string {
  const counts = bulkCounts(rows);
  const done = rows.filter((row) => row.status === 'done');
  const what =
    done.length === 1 && done[0]
      ? nameOf(done[0].item.path)
      : itemCount(done.length);
  if (workflow.kind === 'bulk-restore')
    return done.length === rows.length
      ? `Restored ${what}`
      : `Restored ${done.length} of ${itemCount(rows.length)}`;
  const where =
    workflow.kind === 'bulk-move' ? toFolder(workflow.folder) : 'to Trash';
  if (!done.length && counts.skipped === rows.length)
    return workflow.kind === 'bulk-move'
      ? `${itemCount(rows.length)} already ${inFolder(workflow.folder)}`
      : `${itemCount(rows.length)} already in Trash`;
  if (done.length + counts.skipped === rows.length)
    return `Moved ${what} ${where}`;
  return `Moved ${done.length} of ${itemCount(rows.length)} ${where}`;
}

/** The Undo of the rows of a bulk-trash run that reached Trash. */
function restoreWorkflow(
  rows: readonly BulkRow[],
  trashed: ReadonlyMap<string, string>,
): RestoreWorkflow | null {
  const back = rows.flatMap((row) => {
    const path = trashed.get(rowKey(row.item));
    return row.status === 'done' && path ? [{ item: row.item, path }] : [];
  });
  if (!back.length) return null;
  return {
    kind: 'bulk-restore',
    items: back.map(({ item }) => item),
    trash: back.map(({ path }) => path),
  };
}

/**
 * Moves several items, to one folder of their store, to Trash or back out
 * of Trash, one request at a time, and lists each item's outcome. When
 * every item moved (or already was where it was sent) the sheet closes with
 * a toast, which offers Undo after a move to Trash; otherwise it stays open
 * with the reasons, and Retry failed sends again only the items whose
 * request certainly did not run.
 */
export function BulkProgressSheet({
  workflow,
  setWorkflow,
}: {
  workflow: BulkWorkflow;
  setWorkflow: (workflow: WriteWorkflow) => void;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const {
    bridge,
    refresh: onApplied,
    mutationError: onMutationError,
    reloadCatalog: onReload,
    accessNow,
  } = useShellServices();
  const toasts = useToast();
  const [rows, setRows] = useState<readonly BulkRow[]>(() =>
    pendingRows(workflow.items),
  );
  const [running, setRunning] = useState(true);
  const started = useRef(false);
  // Each Trash entry this sheet created, by the item's row key.
  const trashed = useRef(new Map<string, string>());
  const stores = new Set(workflow.items.map((item) => item.store));
  const [only] = stores;
  const profile =
    stores.size === 1 && only ? storeOf(snapshot, only)?.server : undefined;
  const restoring = workflow.kind === 'bulk-restore';
  useSheetGuard(
    running
      ? {
          verdict: 'refuse',
          reason:
            workflow.kind === 'bulk-move'
              ? 'Wait for the moves to finish.'
              : restoring
                ? 'Wait for the items to be restored.'
                : 'Wait for the items to move to Trash.',
        }
      : null,
    !running,
  );

  const run = async (
    base: readonly BulkRow[],
    indexes: readonly number[],
  ): Promise<void> => {
    setRunning(true);
    const finished = await runBulk({
      rows: base,
      indexes,
      action: bulkAction(workflow, bridge, accessNow, trashed.current),
      // Only the items' own profile is read again before each item.
      reload: () => onReload(profile),
      onRows: setRows,
      onFailure: (error, item) =>
        onMutationError(error, { item, report: false }),
    });
    setRunning(false);
    const counts = bulkCounts(finished);
    if (!counts.refused && !counts.failed && !counts.uncertain) {
      setWorkflow(null);
      const undo =
        workflow.kind === 'bulk-trash'
          ? restoreWorkflow(finished, trashed.current)
          : null;
      try {
        await onApplied(
          finishedSummary(workflow, finished),
          profile,
          undo
            ? {
                action: { label: 'Undo', onAction: () => startUndo(undo) },
                durationMs: UNDO_DURATION_MS,
              }
            : undefined,
        );
      } catch (failure) {
        await onMutationError(failure, { report: false });
      }
      return;
    }
    // The list behind the sheet shows what did move. A failed read is
    // reported by the catalog's own freshness state.
    if (counts.done) void onReload(profile).catch(() => undefined);
  };

  /** Starts the Undo of a move to Trash once the sheet is closed. */
  const startUndo = (undo: RestoreWorkflow): void => {
    // Another sheet may hold a draft that replacing it would discard.
    if (anyDialogOpen()) {
      toasts.show(
        'Close the open sheet first. The items stay in Trash, where Restore returns each one.',
        { tone: 'warning' },
      );
      return;
    }
    setWorkflow(undo);
  };

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    void run(
      rows,
      rows.map((_, index) => index),
    );
    // The run starts once, when the sheet opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const counts = bulkCounts(rows);
  const retry = (): void => {
    if (running) return;
    const indexes = rows.flatMap((row, index) =>
      row.status === 'failed' ? [index] : [],
    );
    if (!indexes.length) return;
    const base = rows.map((row) =>
      row.status === 'failed'
        ? { ...row, status: 'pending' as const, message: 'Waiting' }
        : row,
    );
    setRows(base);
    void run(base, indexes);
  };
  // A run that stopped short of every item can still undo what reached Trash.
  const undo =
    workflow.kind === 'bulk-trash' && !running
      ? restoreWorkflow(rows, trashed.current)
      : null;
  const total = itemCount(rows.length);
  const title = running
    ? workflow.kind === 'bulk-move'
      ? `Moving ${total} ${toFolder(workflow.folder)}`
      : restoring
        ? `Restoring ${total}`
        : `Moving ${total} to Trash`
    : finishedSummary(workflow, rows);
  const status = running
    ? `${counts.done + counts.skipped + counts.refused + counts.failed + counts.uncertain} of ${rows.length} done`
    : [
        counts.refused
          ? `${counts.refused} not ${restoring ? 'restored' : 'moved'}`
          : '',
        counts.failed ? `${counts.failed} failed` : '',
        counts.uncertain ? `${counts.uncertain} unknown` : '',
      ]
        .filter(Boolean)
        .join(' · ');
  return (
    <SheetDialog
      title={title}
      width="mid"
      glyph={
        <span className="kic Folder" aria-hidden="true">
          <Icon
            name={
              workflow.kind === 'bulk-move'
                ? 'folder'
                : restoring
                  ? 'restore'
                  : 'trash'
            }
          />
        </span>
      }
      onClose={() => setWorkflow(null)}
      dismissible={!running}
      footer={
        <>
          {undo ? (
            <Button icon="restore" onClick={() => setWorkflow(undo)}>
              Undo
            </Button>
          ) : null}
          <Button
            disabled={running || !counts.failed}
            icon="refresh"
            onClick={retry}
          >
            Retry failed
          </Button>
          <Button
            variant="primary"
            disabled={running}
            data-dialog-autofocus="true"
            onClick={() => setWorkflow(null)}
          >
            {running ? (restoring ? 'Restoring…' : 'Moving…') : 'Done'}
          </Button>
        </>
      }
    >
      <p role="status">{status}</p>
      <ul className="bulk-rows" aria-label="Items">
        {rows.map((row) => (
          <li
            key={`${row.item.store}|${row.item.path}`}
            className={`bulk-row ${row.status}`}
          >
            {row.status === 'running' ? (
              <span className="spin" aria-hidden="true" />
            ) : (
              <Icon name={STATUS_ICON[row.status]} />
            )}
            <span className="t">
              <b>{nameOf(row.item.path)}</b>
              <small>{row.message}</small>
            </span>
          </li>
        ))}
      </ul>
    </SheetDialog>
  );
}
