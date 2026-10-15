/** Permanent deletion of an entry in Trash, or of an item outside Trash. */

import { useId, useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Icon, SheetDialog } from '../../../components';
import { nameOf, parseTrashName, storeOf } from '../../../model';
import type { Item } from '../../../model';
import { normalizeCommandError } from '../../../bridge';
import { useSheetGuard } from '../../../navigation-guard';
import { itemActionProblem } from '../../store-access';
import { useShellServices, useShellSnapshot } from '../../../shell-services';
import type { WriteWorkflow } from '../write-workflow-model';

/** What permanent deletion says it does, in every sheet that offers it. */
export const PERMANENT_DELETE_COPY =
  'Removes it for everyone. Earlier encrypted versions may remain on the server.';

/** What an entry in Trash is called: its original name where recorded. */
function trashLabel(item: Item): string {
  const parsed = parseTrashName(nameOf(item.path));
  return parsed?.kind === 'path' ? nameOf(parsed.original) : nameOf(item.path);
}

/** Why a team member is offered permanent deletion instead of Trash. */
export const TRASH_UNAVAILABLE_COPY =
  'Trash is not set up in this team yet, and only an owner can set it up by moving any item to Trash.';

/**
 * Confirmation dialog for permanently deleting an entry in Trash, or an item
 * in a team whose Trash this account cannot set up.
 */
export function DeleteSheet({
  workflow,
  setWorkflow,
  onDeleteConflict,
}: {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'delete' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onDeleteConflict: (item: Item) => Promise<void>;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const {
    accessNow,
    bridge,
    refresh: onApplied,
    mutationError: onMutationError,
  } = useShellServices();
  const [deleting, setDeleting] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const [failureMessage, setFailureMessage] = useState<string | null>(null);
  const [, recheckAccess] = useState(0);
  const accessDescriptionId = useId();
  const problem = itemActionProblem(snapshot, workflow.item, true, accessNow());
  useSheetGuard(
    deleting
      ? { verdict: 'refuse', reason: 'Wait for the deletion to finish.' }
      : null,
    !deleting,
  );
  return (
    <SheetDialog
      danger
      onClose={() => setWorkflow(null)}
      glyph={
        <span className="kico md danger">
          <Icon name="trash" />
        </span>
      }
      dismissible={!deleting}
      title={`Delete ${trashLabel(workflow.item)} permanently?`}
      footer={
        <>
          <Button disabled={deleting} onClick={() => setWorkflow(null)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            danger
            disabled={deleting || uncertain || Boolean(problem)}
            title={problem}
            aria-describedby={problem ? accessDescriptionId : undefined}
            onClick={() => {
              if (deleting || uncertain) return;
              if (
                itemActionProblem(snapshot, workflow.item, true, accessNow())
              ) {
                recheckAccess((revision) => revision + 1);
                return;
              }
              setDeleting(true);
              setFailureMessage(null);
              void (async () => {
                try {
                  await bridge.removeItem({
                    direntId: workflow.item.direntId,
                    storeId: workflow.item.store,
                    path: workflow.item.path,
                    version: workflow.item.version,
                    ...(workflow.outsideTrash ? { outsideTrash: true } : {}),
                  });
                  setWorkflow(null);
                  // Keep refresh failure separate from the acknowledged deletion.
                  try {
                    await onApplied(
                      `Deleted ${trashLabel(workflow.item)} permanently`,
                      storeOf(snapshot, workflow.item.store)?.server,
                    );
                  } catch (failure) {
                    await onMutationError(failure, { report: false });
                  }
                } catch (error) {
                  const typed = normalizeCommandError(error);
                  setUncertain(typed.ambiguous);
                  setFailureMessage(typed.message);
                  if (typed.code === 'conflict') {
                    // Changed or already gone: the shell's refresh finds out
                    // which and says so, rather than this sheet assuming.
                    setWorkflow(null);
                    await onDeleteConflict(workflow.item);
                  } else {
                    await onMutationError(error, { item: workflow.item });
                  }
                } finally {
                  setDeleting(false);
                }
              })();
            }}
          >
            {deleting ? 'Deleting…' : 'Delete permanently'}
          </Button>
        </>
      }
    >
      {failureMessage ? <p role="alert">{failureMessage}</p> : null}
      {problem ? (
        <p id={accessDescriptionId} className="action-error" role="status">
          {problem}
        </p>
      ) : null}
      {workflow.outsideTrash ? <p>{TRASH_UNAVAILABLE_COPY}</p> : null}
      <p>
        {workflow.item.kind === 'Folder'
          ? `This folder and everything in it will be deleted. ${PERMANENT_DELETE_COPY}`
          : PERMANENT_DELETE_COPY}
      </p>
    </SheetDialog>
  );
}
