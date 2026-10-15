/** Restoring a Trash entry whose original path another item now holds. */

import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Inset, InsetRow, SheetDialog } from '../../../components';
import { nameOf, restoredCopyPath } from '../../../model';
import type { Item } from '../../../model';
import { useSheetGuard } from '../../../navigation-guard';
import { useShellSnapshot } from '../../../shell-services';
import type { WriteWorkflow } from '../../write-workflow-model';

/**
 * Restoring an entry from Trash found another item at its original path.
 * The entry can be restored under a new name beside it; nothing is replaced.
 */
export function RestoreConflictSheet({
  workflow,
  setWorkflow,
  onRestoreTrash,
}: {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'restore-conflict' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onRestoreTrash: (item: Item, destination: string) => Promise<void>;
}): ReactNode {
  const snapshot = useShellSnapshot();
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
      title={`${nameOf(destination)} is already in use`}
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
              void onRestoreTrash(item, copy).finally(() => {
                setRestoring(false);
                setWorkflow(null);
              });
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
