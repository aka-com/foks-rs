/**
 * Revokes a recovery phrase from an account after its alias is typed to
 * confirm.
 */
import { useDeviceOperation } from '../use-device-operation';
import { useWorkflowAccess } from '../../../workflow-context';
import { useState } from 'react';
import type { ReactNode } from 'react';
import { revokeRecoveryPhrase } from '../revocation-workflow';
import { useSheetGuard } from '../../../navigation-guard';
import type { BackupEnrollment } from '../../../bridge';
import { Button, Inset, InsetRow, SheetDialog } from '../../../components';
import type { AccountStore } from '../../../model';
import { useShellServices } from '../../../shell-services';

export function RevokeBackupSheet({
  store,
  backup,
  onClose,
  onDone,
  onError,
}: {
  store: AccountStore;
  backup: BackupEnrollment;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const { bridge } = useShellServices();
  const access = useWorkflowAccess();
  const workflowTarget = { profile: store.server, account: store.account };
  const eligibility = access.props('backup-revoke', workflowTarget);
  const controller = useDeviceOperation(
    JSON.stringify([store.id, workflowTarget, backup.backupId]),
  );
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  // Revoking a recovery phrase rotates every account key it could read.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the revocation to finish.' }
      : null,
  );
  return (
    <SheetDialog
      width="wide"
      icon="trash"
      title={`Revoke ${backup.backupAlias}?`}
      onClose={() => {
        if (busy) return;
        onClose();
      }}
      danger
      dismissible={!busy}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            disabled={
              confirmation !== backup.backupAlias ||
              busy ||
              eligibility.disabled
            }
            onClick={() => {
              setBusy(true);
              void controller
                .run(
                  () =>
                    revokeRecoveryPhrase(
                      bridge,
                      access,
                      store,
                      backup,
                      confirmation,
                    ),
                  onDone,
                  onError,
                )
                .finally(controller.settled(() => setBusy(false)));
            }}
          >
            Revoke recovery phrase
          </Button>
        </>
      }
    >
      <p>
        This recovery phrase can no longer recover the account, and the account
        keys are rotated. Type its name to confirm.
      </p>
      <Inset>
        <InsetRow label="Confirm">
          <input
            value={confirmation}
            onChange={(event) => setConfirmation(event.target.value)}
            placeholder={`type ${backup.backupAlias}`}
          />
        </InsetRow>
      </Inset>
    </SheetDialog>
  );
}
