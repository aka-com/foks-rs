/**
 * The sheet an edit or a replacement gives way to when the item changed or
 * was removed elsewhere.
 */

import { useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { Dialog } from '../../../../kit/overlay-primitives';
import { Button, Icon, KindIcon, Sheet } from '../../../components';
import type { FilterKind } from '../../../components';
import { kindOf } from '../../../model';
import type { Item } from '../../../model';
import { useShellServices } from '../../../shell-services';
import type { WriteWorkflow } from '../write-workflow-model';

/**
 * Modal sheet displayed when a save conflict occurs, prompting the user to review or discard changes.
 */
export function ConflictSheet({
  workflow,
  setWorkflow,
  onRefreshConflict,
  onDiscardConflict,
}: {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'conflict' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onRefreshConflict: (
    item: Item,
    draft: string,
    operation?: 'edit' | 'replace',
  ) => Promise<WriteWorkflow>;
  onDiscardConflict: () => void;
}): ReactNode {
  const { commandError: onError } = useShellServices();
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);
  const replacing = workflow.operation === 'replace';

  if (confirmingDiscard)
    return (
      <Dialog
        className="backdrop"
        role="alertdialog"
        aria-label="Discard unsaved changes"
        onKeyDown={(event: KeyboardEvent<HTMLDivElement>) => {
          // Escape closes the confirmation dialog without discarding the draft.
          if (event.key === 'Escape') setConfirmingDiscard(false);
        }}
      >
        <Sheet
          glyph={
            <span className="kico md danger">
              <Icon name="trash" />
            </span>
          }
          title="Discard unsaved changes?"
          footer={
            <>
              <Button onClick={() => setConfirmingDiscard(false)}>
                {replacing ? 'Keep replacing' : 'Keep editing'}
              </Button>
              <Button variant="primary" danger onClick={onDiscardConflict}>
                Discard changes
              </Button>
            </>
          }
        >
          <p>Your changes have not been saved.</p>
        </Sheet>
      </Dialog>
    );

  return (
    <Dialog
      className="backdrop"
      aria-label={replacing ? 'Replace conflict' : 'Edit conflict'}
      onKeyDown={(event: KeyboardEvent<HTMLDivElement>) => {
        if (event.key === 'Escape') setConfirmingDiscard(true);
      }}
    >
      <Sheet
        glyph={<KindIcon kind={kindOf(workflow.item) as FilterKind} />}
        title="Item changed elsewhere"
        footer={
          <>
            <Button onClick={() => setConfirmingDiscard(true)}>
              {replacing ? 'Discard replacement' : 'Discard my edit'}
            </Button>
            <Button
              variant="primary"
              onClick={() => {
                // The refresh finds out whether the item was changed or
                // removed, and says what this sheet gives way to.
                void onRefreshConflict(
                  workflow.item,
                  workflow.draft,
                  workflow.operation,
                ).then(
                  (next) => setWorkflow(next),
                  (error) => onError(error),
                );
              }}
            >
              Refresh and review
            </Button>
          </>
        }
      >
        <>
          <p>
            {/* The agent reports one conflict for an item that was changed and
                one that was removed, so this does not claim to know which. */}
            {replacing
              ? 'This file changed or was removed while you were replacing it. Refresh to review the current item, then choose the replacement again.'
              : 'This item was changed or removed while you were editing. Refresh to see what happened; your edit is kept until you discard it.'}
          </p>
        </>
      </Sheet>
    </Dialog>
  );
}
