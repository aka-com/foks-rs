/** The New folder sheet: one folder's name in a vault and folder. */

import { useCallback, useState } from 'react';
import type { ReactNode } from 'react';
import { DismissibleDialog } from '../../../../kit/overlay-primitives';
import {
  Band,
  Button,
  Field,
  Icon,
  Inset,
  InsetRow,
  SectionLabel,
  Sheet,
} from '../../../components';
import { canCreateInStore, storeAvailability, storeOf } from '../../../model';
import { normalizeCommandError } from '../../../bridge';
import type { GuardVerdict } from '../../../location';
import {
  NavigationPrompt,
  useSheetGuard,
  useTabSheetState,
} from '../../../navigation-guard';
import { newFolderRoles } from '../../item-roles';
import { writeBlockReason } from '../../store-choice';
import { useShellServices, useShellSnapshot } from '../../../shell-services';
import { namedPath } from '../write-workflow-model';
import type { WriteWorkflow } from '../write-workflow-model';
import { SAVING_REFUSAL } from './new-item-sheet';

export function NewFolderSheet({
  workflow,
  setWorkflow,
}: {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'new-folder' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const {
    bridge,
    refresh: onApplied,
    mutationError: onMutationError,
    accessNow,
  } = useShellServices();
  const [name, setName] = useTabSheetState('folder.name', '');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);
  const store = storeOf(snapshot, workflow.storeId);
  const blocked = store ? writeBlockReason(snapshot, store) : null;
  const validName = Boolean(
    name.trim() &&
    name.trim() !== '.' &&
    name.trim() !== '..' &&
    !/[\\/]/.test(name),
  );
  const canWrite = Boolean(
    store &&
    !blocked &&
    storeAvailability(snapshot, store, { nowSeconds: accessNow() }).available &&
    canCreateInStore(snapshot, store.id),
  );
  const close = useCallback(() => setWorkflow(null), [setWorkflow]);
  const discard = (): Extract<GuardVerdict, { verdict: 'prompt' }> => ({
    verdict: 'prompt',
    title: 'Discard new folder?',
    body: 'The folder has not been created.',
    confirm: 'Discard',
  });
  useSheetGuard(
    saving
      ? { verdict: 'refuse', reason: SAVING_REFUSAL }
      : name
        ? { ...discard(), onConfirm: close }
        : null,
    !saving,
  );
  const dismiss = () => {
    if (saving) return;
    if (name) setConfirmingDiscard(true);
    else close();
  };
  const submit = async () => {
    if (!store || !canWrite || !validName || saving) return;
    setSaving(true);
    setError(null);
    try {
      await bridge.createFolder({
        storeId: store.id,
        path: namedPath(name.trim(), '/', workflow.initialFolder),
        ...newFolderRoles(store),
      });
      await onApplied(`Folder created in ${store.name}`, store.server);
      close();
    } catch (failure) {
      setError(normalizeCommandError(failure).message);
      await onMutationError(failure, { report: false });
    } finally {
      setSaving(false);
    }
  };
  return (
    <>
      <DismissibleDialog
        className="backdrop"
        aria-label="New folder"
        onDismiss={dismiss}
      >
        <Sheet
          width="mid"
          glyph={
            <span className="kic Folder" aria-hidden="true">
              <Icon name="folder" />
            </span>
          }
          title="New folder"
          footer={
            <>
              <Button onClick={dismiss}>Cancel</Button>
              <Button
                variant="primary"
                disabled={!canWrite || !validName || saving}
                onClick={() => void submit()}
              >
                {saving ? 'Creating…' : 'Create folder'}
              </Button>
            </>
          }
        >
          <SectionLabel>Location</SectionLabel>
          <Inset>
            <InsetRow label="Vault">
              <span>{store?.name ?? 'Vault unavailable'}</span>
            </InsetRow>
            {workflow.initialFolder && workflow.initialFolder !== '/' ? (
              <InsetRow label="Folder">
                <span>{workflow.initialFolder}</span>
              </InsetRow>
            ) : null}
            <Field
              label="Name"
              value={name}
              onChange={(next) => {
                setName(next);
                setError(null);
              }}
              placeholder="Folder name"
            />
          </Inset>
          {name && !validName ? (
            <p className="action-error" role="alert">
              Enter a name without slashes, “.”, or “..”.
            </p>
          ) : null}
          {store && blocked ? (
            <Band live>{`${store.name}: ${blocked}`}</Band>
          ) : null}
          {error ? (
            <p className="action-error" role="alert">
              {error}
            </p>
          ) : null}
        </Sheet>
      </DismissibleDialog>
      {confirmingDiscard ? (
        <NavigationPrompt
          verdict={discard()}
          onConfirm={close}
          onCancel={() => setConfirmingDiscard(false)}
        />
      ) : null}
    </>
  );
}
