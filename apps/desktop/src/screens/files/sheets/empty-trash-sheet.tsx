/** Empty Trash: permanent deletion of one vault's Trash entries. */

import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Icon, SheetDialog } from '../../../components';
import { storeOf, trashEntries, trashEntryLabel } from '../../../model';
import { useSheetGuard } from '../../../navigation-guard';
import { itemActionProblem } from '../../store-access';
import { useTrashOrigins } from '../../trash-origins';
import { useShellServices, useShellSnapshot } from '../../../shell-services';
import type { WriteWorkflow } from '../write-workflow-model';
import { PERMANENT_DELETE_COPY } from './delete-sheet';

/**
 * Permanently deletes every entry in one store's Trash that this account can
 * change, folders with everything in them, in one host mutation. Entries
 * whose write role this account lacks stay in Trash.
 */
export function EmptyTrashSheet({
  workflow,
  setWorkflow,
}: {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'empty-trash' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const {
    accessNow,
    bridge,
    refresh: onApplied,
    mutationError: onMutationError,
  } = useShellServices();
  const [listed] = useState(() => trashEntries(snapshot, workflow.store));
  const entries = useTrashOrigins(bridge, listed);
  const [emptying, setEmptying] = useState(false);
  const store = storeOf(snapshot, workflow.store);
  const problems = entries.map((entry) =>
    itemActionProblem(snapshot, entry.item, true, accessNow()),
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
    `${amount} ${amount === 1 ? 'item' : 'items'}`;
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
      title={`Permanently delete ${count(removable.length)}?`}
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
            onClick={() => {
              if (emptying || !removable.length) return;
              setEmptying(true);
              void (async () => {
                try {
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
            }}
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
          {count(kept)} {kept === 1 ? 'stays' : 'stay'} in Trash because this
          account cannot change {kept === 1 ? 'it' : 'them'}.
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
