import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Icon, SheetDialog } from '../components';
import type { Bridge } from '../bridge';
import type { FoksIconName } from '../icons';
import { isTrashPath, nameOf, storeOf, trashUnavailable } from '../model';
import type { AgentSnapshot, Item } from '../model';
import type { MutationFailureHandler } from '../mutation-recovery';
import { useSheetGuard } from '../navigation-guard';
import {
  baseName,
  bulkCounts,
  inFolder,
  itemCount,
  joinPath,
  moveRowCheck,
  pendingRows,
  runBulk,
  toFolder,
} from './item-moves';
import type { BulkAction, BulkRow, BulkStatus } from './item-moves';
import { itemActionProblem } from './store-access';
import type { WriteWorkflow } from './write-workflows';

export type BulkWorkflow = Extract<
  NonNullable<WriteWorkflow>,
  { kind: 'bulk-move' } | { kind: 'bulk-trash' }
>;

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

function bulkAction(
  workflow: BulkWorkflow,
  bridge: Bridge,
  accessNow: () => number,
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
        await bridge.moveItem({
          direntId: item.direntId,
          storeId: item.store,
          path: item.path,
          version: item.version,
          destination: joinPath(folder, baseName(item.path)),
        });
        return `Moved ${toFolder(folder)}`;
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
      await bridge.trashItem({
        storeId: item.store,
        path: item.path,
        version: item.version,
        direntId: item.direntId,
      });
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

/**
 * Moves several items, to one folder of their store or to Trash, one
 * request at a time, and lists each item's outcome. When every item moved
 * (or already was where it was sent) the sheet closes with a toast;
 * otherwise it stays open with the reasons, and Retry failed sends again
 * only the items whose request certainly did not run.
 */
export function BulkProgressSheet({
  snapshot,
  bridge,
  workflow,
  setWorkflow,
  onApplied,
  onMutationError,
  onReload,
  accessNow,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  workflow: BulkWorkflow;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onApplied: (message: string, profile?: string) => Promise<void>;
  onMutationError: MutationFailureHandler;
  /** Reads the catalog again and resolves with it. */
  onReload: () => Promise<AgentSnapshot>;
  accessNow: () => number;
}): ReactNode {
  const [rows, setRows] = useState<readonly BulkRow[]>(() =>
    pendingRows(workflow.items),
  );
  const [running, setRunning] = useState(true);
  const started = useRef(false);
  const stores = new Set(workflow.items.map((item) => item.store));
  const [only] = stores;
  const profile =
    stores.size === 1 && only ? storeOf(snapshot, only)?.server : undefined;
  useSheetGuard(
    running
      ? {
          verdict: 'refuse',
          reason:
            workflow.kind === 'bulk-move'
              ? 'Wait for the moves to finish.'
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
      action: bulkAction(workflow, bridge, accessNow),
      reload: onReload,
      onRows: setRows,
      onFailure: (error, item) =>
        onMutationError(error, { item, report: false }),
    });
    setRunning(false);
    const counts = bulkCounts(finished);
    if (!counts.refused && !counts.failed && !counts.uncertain) {
      setWorkflow(null);
      try {
        await onApplied(finishedSummary(workflow, finished), profile);
      } catch (failure) {
        await onMutationError(failure, { report: false });
      }
      return;
    }
    // The list behind the sheet shows what did move. A failed read is
    // reported by the catalog's own freshness state.
    if (counts.done) void onReload().catch(() => undefined);
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
  const total = itemCount(rows.length);
  const title = running
    ? workflow.kind === 'bulk-move'
      ? `Moving ${total} ${toFolder(workflow.folder)}`
      : `Moving ${total} to Trash`
    : finishedSummary(workflow, rows);
  const status = running
    ? `${counts.done + counts.skipped + counts.refused + counts.failed + counts.uncertain} of ${rows.length} done`
    : [
        counts.refused ? `${counts.refused} not moved` : '',
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
          <Icon name={workflow.kind === 'bulk-move' ? 'folder' : 'trash'} />
        </span>
      }
      onClose={() => setWorkflow(null)}
      dismissible={!running}
      footer={
        <>
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
            {running ? 'Moving…' : 'Done'}
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
