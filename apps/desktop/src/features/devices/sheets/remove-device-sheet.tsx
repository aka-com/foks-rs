/**
 * Removes a device from an account and lists the items it could read for
 * exposure review.
 */
import { useDeviceOperation } from '../use-device-operation';
import { useWorkflowAccess } from '../../../workflow-context';
import { useState } from 'react';
import type { ReactNode } from 'react';
import { removeDevice } from '../revocation-workflow';
import { useSheetGuard } from '../../../navigation-guard';
import type { AccountDevice } from '../../../bridge';
import { Button, Inset, InsetRow, SheetDialog } from '../../../components';
import type { AccountStore } from '../../../model';
import {
  accountExposure,
  entriesOf,
  exposureOrder,
  withExposureReview,
} from '../../../exposure-review';
import {
  ExposureSummary,
  unstoredReview,
} from '../../../screens/exposure-review';
import { useToast } from '../../../../kit/toasts';
import { useShellServices, useShellSnapshot } from '../../../shell-services';

export function RemoveDeviceSheet({
  store,
  device,
  onClose,
  onDone,
  onError,
}: {
  store: AccountStore;
  device: AccountDevice;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const { bridge } = useShellServices();
  const access = useWorkflowAccess();
  const workflowTarget = { profile: store.server, account: store.account };
  const eligibility = access.props('device-remove', workflowTarget);
  const controller = useDeviceOperation(
    JSON.stringify([store.id, workflowTarget, device.id]),
  );
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  const toasts = useToast();
  const expected = device.name ?? device.id;
  // The device held the account's keys, so it could read every item in the
  // account's own store and its teams. The list waits while any of them is
  // being read, and names a store whose items could not be listed.
  const exposure = accountExposure(snapshot, store);
  const readiness = exposure.readiness;
  const exposed = exposureOrder(exposure.items);
  // Removing a device is a write the agent has already been given.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the removal to finish.' }
      : null,
  );
  return (
    <SheetDialog
      width="wide"
      icon="trash"
      title={`Remove ${device.name ?? 'device'}?`}
      // A removal already handed to the agent cannot be called back, so
      // Escape, the backdrop and Cancel all stop answering while it runs —
      // the same guard the revoke sheets carry.
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
              confirmation !== expected ||
              busy ||
              eligibility.disabled ||
              readiness === 'loading'
            }
            onClick={() => {
              setBusy(true);
              const entries = entriesOf(exposed);
              // The display cache can outlive the native catalog's retained
              // device list. Re-read before a destructive action so native
              // target/current-device validation uses fresh records.
              void controller
                .run(
                  async () => {
                    await withExposureReview(
                      {
                        scope: store.id,
                        party: device.id,
                        name: expected,
                        cause: 'device-removed',
                        entries,
                      },
                      () => removeDevice(bridge, access, store, device.id),
                      () => toasts.show(unstoredReview(expected)),
                    );
                  },
                  onDone,
                  onError,
                )
                .finally(controller.settled(() => setBusy(false)));
            }}
          >
            Remove device
          </Button>
        </>
      }
    >
      <p>
        This device loses access to the account. Data already on it stays there;
        change any secrets it could read if it is not under your control.
      </p>
      <Inset>
        <InsetRow label="Confirm">
          <input
            value={confirmation}
            onChange={(event) => setConfirmation(event.target.value)}
            placeholder={`type ${expected}`}
          />
        </InsetRow>
      </Inset>
      <ExposureSummary
        snapshot={snapshot}
        name={expected}
        heading={`Items ${expected} could read`}
        empty="This account has no items this device could read."
        items={exposed}
        readiness={readiness}
        unlisted={exposure.unlisted}
        teamsUnlisted={exposure.teamsUnlisted}
      />
    </SheetDialog>
  );
}
