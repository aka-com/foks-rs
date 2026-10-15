/** The step a new item's save gives way to when its path is already taken. */

import { useCallback, useState } from 'react';
import type { ReactNode } from 'react';
import { DismissibleDialog } from '../../../../kit/overlay-primitives';
import { Button, KindIcon, Sheet } from '../../../components';
import type { FilterKind } from '../../../components';
import { displayPath, kindOf } from '../../../model';
import { NavigationPrompt, useSheetGuard } from '../../../navigation-guard';
import { useShellServices, useShellSnapshot } from '../../../shell-services';
import type { WriteWorkflow } from '../write-workflow-model';
import { discardNewItem } from './new-item-sheet';

export function ExistsSheet({
  workflow,
  setWorkflow,
  onOpenExisting,
}: {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'exists' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onOpenExisting: (
    workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'exists' }>,
  ) => Promise<void>;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const { bridge, commandError: onError } = useShellServices();
  const clash = snapshot.items.find(
    (item) => item.store === workflow.storeId && item.path === workflow.path,
  );
  // This step still holds the draft the new-item sheet carried here, so it is
  // abandoned under the same confirmation, from a navigation or from Escape
  // and the backdrop.
  const close = useCallback(() => {
    const sourcePath = workflow.draft?.sourcePath;
    if (sourcePath) void bridge.releaseImportFile(sourcePath).catch(onError);
    setWorkflow(null);
  }, [bridge, onError, setWorkflow, workflow.draft?.sourcePath]);
  useSheetGuard(discardNewItem(workflow.itemKind, close));
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);
  const sheet = (
    <Sheet
      glyph={
        clash ? <KindIcon kind={kindOf(clash) as FilterKind} /> : undefined
      }
      title={`An item already exists at ${displayPath(workflow.path)}`}
      footer={
        <>
          <Button
            onClick={() =>
              setWorkflow({
                kind: 'new',
                itemKind: workflow.itemKind,
                storeId: workflow.storeId,
                initialFolder: workflow.initialFolder,
                draft: workflow.draft,
              })
            }
          >
            Change path
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              void onOpenExisting(workflow).then(close, onError);
            }}
          >
            Open existing item
          </Button>
        </>
      }
    >
      <>
        <p>
          The upload was skipped. Open the existing item and replace it, or
          choose another path.
        </p>
      </>
    </Sheet>
  );
  return (
    <>
      <DismissibleDialog
        className="backdrop"
        aria-label="Item path already exists"
        onDismiss={() => setConfirmingDiscard(true)}
      >
        {sheet}
      </DismissibleDialog>
      {confirmingDiscard ? (
        <NavigationPrompt
          verdict={discardNewItem(workflow.itemKind)}
          onConfirm={close}
          onCancel={() => setConfirmingDiscard(false)}
        />
      ) : null}
    </>
  );
}
