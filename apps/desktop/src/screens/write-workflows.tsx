/**
 * The write overlay: the sheet each write workflow opens. The sheets are in
 * `files/sheets/` and in the `*-sheet.tsx` modules beside this one; the
 * workflow type is in `write-workflow-model.ts`.
 */

import { MoveSheet } from './move-item-sheet';
import { BulkProgressSheet } from './bulk-progress-sheet';
import type { ReactNode } from 'react';
import type { Item } from '../model';
import { CopyItemSheet } from './copy-item-sheet';
import { ImportPasswordsSheet } from './import-passwords-sheet';
import { ExportPasswordsSheet } from './export-passwords-sheet';
import type { WriteWorkflow } from './write-workflow-model';
import { NewSheet } from './files/sheets/new-item-sheet';
import { NewFolderSheet } from './files/sheets/new-folder-sheet';
import { ExistsSheet } from './files/sheets/exists-sheet';
import { ConflictSheet } from './files/sheets/conflict-sheet';
import { DeleteSheet } from './files/sheets/delete-sheet';
import { RestoreConflictSheet } from './files/sheets/restore-conflict-sheet';
import { EmptyTrashSheet } from './files/sheets/empty-trash-sheet';

export function WriteOverlay({
  workflow,
  setWorkflow,
  onRefreshConflict,
  onDiscardConflict,
  onDeleteConflict,
  onOpenExisting,
  onRestoreTrash,
}: {
  workflow: WriteWorkflow;
  setWorkflow: (workflow: WriteWorkflow) => void;
  /**
   * Refreshes after an edit conflict and says what the conflict sheet gives
   * way to: nothing, when the item is there to review, or a new-item sheet
   * carrying the edit when the item turned out to have been deleted.
   */
  onRefreshConflict: (
    item: Item,
    draft: string,
    operation?: 'edit' | 'replace',
  ) => Promise<WriteWorkflow>;
  onDiscardConflict: () => void;
  /** Refreshes after a delete conflict and states whether the item is still there. */
  onDeleteConflict: (item: Item) => Promise<void>;
  onOpenExisting: (
    workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'exists' }>,
  ) => Promise<void>;
  /** Moves a Trash entry to `destination`, outside Trash. */
  onRestoreTrash: (item: Item, destination: string) => Promise<void>;
}): ReactNode {
  if (!workflow) return null;
  // The sheet supplies its own dialog: Escape and the backdrop are decided by
  // the draft it holds, the same state its navigation guard answers from.
  if (workflow.kind === 'new')
    return <NewSheet workflow={workflow} setWorkflow={setWorkflow} />;
  if (workflow.kind === 'new-folder')
    return <NewFolderSheet workflow={workflow} setWorkflow={setWorkflow} />;
  // The already-exists step carries the same draft, so it supplies its own
  // dialog for the same reason.
  if (workflow.kind === 'exists')
    return (
      <ExistsSheet
        workflow={workflow}
        setWorkflow={setWorkflow}
        onOpenExisting={onOpenExisting}
      />
    );
  if (workflow.kind === 'move' || workflow.kind === 'move-many')
    return (
      <MoveSheet
        items={workflow.kind === 'move' ? [workflow.item] : workflow.items}
        setWorkflow={setWorkflow}
      />
    );
  if (workflow.kind === 'copy')
    return <CopyItemSheet item={workflow.item} setWorkflow={setWorkflow} />;
  if (workflow.kind === 'export')
    return (
      <ExportPasswordsSheet
        storeId={workflow.storeId}
        setWorkflow={setWorkflow}
      />
    );
  if (workflow.kind === 'import')
    return (
      <ImportPasswordsSheet workflow={workflow} setWorkflow={setWorkflow} />
    );
  if (
    workflow.kind === 'bulk-move' ||
    workflow.kind === 'bulk-trash' ||
    workflow.kind === 'bulk-restore'
  )
    return (
      // The Undo of a bulk-trash run replaces it with a run of its own.
      <BulkProgressSheet
        key={workflow.kind}
        workflow={workflow}
        setWorkflow={setWorkflow}
      />
    );
  if (workflow.kind === 'restore-conflict')
    return (
      <RestoreConflictSheet
        workflow={workflow}
        setWorkflow={setWorkflow}
        onRestoreTrash={onRestoreTrash}
      />
    );
  if (workflow.kind === 'empty-trash')
    return <EmptyTrashSheet workflow={workflow} setWorkflow={setWorkflow} />;
  if (workflow.kind === 'conflict')
    return (
      <ConflictSheet
        workflow={workflow}
        setWorkflow={setWorkflow}
        onRefreshConflict={onRefreshConflict}
        onDiscardConflict={onDiscardConflict}
      />
    );
  return (
    <DeleteSheet
      workflow={workflow}
      setWorkflow={setWorkflow}
      onDeleteConflict={onDeleteConflict}
    />
  );
}
