/**
 * Revokes a hardware key from an account after the alias is typed to confirm.
 */
import { useDeviceOperation } from '../use-device-operation';
import { useWorkflowAccess } from '../../../workflow-context';
import { useState } from 'react';
import type { ReactNode } from 'react';
import { revokeSecurityKey } from '../revocation-workflow';
import { useSheetGuard } from '../../../navigation-guard';
import { Button, Inset, InsetRow, SheetDialog } from '../../../components';
import type { AccountStore } from '../../../model';
import { useShellServices } from '../../../shell-services';

export function RevokeSheet({
  store,
  alias,
  onClose,
  onDone,
  onError,
}: {
  store: AccountStore;
  alias: string;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const { bridge } = useShellServices();
  const access = useWorkflowAccess();
  const workflowTarget = { profile: store.server, account: store.account };
  const eligibility = access.props('yubi-revoke', workflowTarget);
  const controller = useDeviceOperation(
    JSON.stringify([store.id, workflowTarget, alias]),
  );
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  // A revocation rotates account keys and cannot be taken back, so the sheet
  // that started it is where its answer is read. The typed alias above it is a
  // confirmation, not content: nothing is lost by typing it again.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the revocation to finish.' }
      : null,
  );
  return (
    <SheetDialog
      width="wide"
      icon="trash"
      title={`Revoke ${alias}?`}
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
            disabled={confirmation !== alias || busy || eligibility.disabled}
            onClick={() => {
              setBusy(true);
              void controller
                .run(
                  () =>
                    revokeSecurityKey(
                      bridge,
                      access,
                      store,
                      alias,
                      confirmation,
                    ),
                  onDone,
                  onError,
                )
                .finally(controller.settled(() => setBusy(false)));
            }}
          >
            Revoke {alias}
          </Button>
        </>
      }
    >
      <p>This key loses access to the account. Type its alias to confirm.</p>
      <Inset>
        <InsetRow label="Confirm">
          <input
            value={confirmation}
            onChange={(event) => setConfirmation(event.target.value)}
            placeholder={`type ${alias}`}
          />
        </InsetRow>
      </Inset>
    </SheetDialog>
  );
}
