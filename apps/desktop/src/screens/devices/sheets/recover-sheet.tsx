/** Connects an account on this device from its recovery phrase. */
import { useDeviceOperation } from '../use-device-operation';
import { recoverAccount } from '../pairing-workflow';
import { useWorkflowAccess } from '../../../workflow-context';
import { useState } from 'react';
import type { ReactNode } from 'react';
import { useSheetGuard } from '../../../navigation-guard';
import {
  Button,
  Field,
  Inset,
  InsetRow,
  SheetDialog,
} from '../../../components';
import type { AccountStore } from '../../../model';
import { useShellServices } from '../../../shell-services';

export function RecoverSheet({
  store,
  onClose,
  onDone,
  onError,
}: {
  store: AccountStore;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const { bridge } = useShellServices();
  const access = useWorkflowAccess();
  const workflowTarget = { profile: store.server };
  const eligibility = access.props('account-recover', workflowTarget);
  const controller = useDeviceOperation(
    JSON.stringify([store.server, store.id]),
  );
  const [target, setTarget] = useState(store.account);
  const [device, setDevice] = useState('This device');
  const [phrase, setPhrase] = useState('');
  const [busy, setBusy] = useState(false);
  // Recovery adds a device: once the phrase has been sent, the sheet is where
  // its answer arrives. Before that it holds only what was typed.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the recovery to finish.' }
      : phrase
        ? {
            verdict: 'prompt',
            title: 'Discard recovery phrase?',
            body: 'The recovery phrase typed here has not been submitted.',
            confirm: 'Discard',
            onConfirm: () => {
              setPhrase('');
              onClose();
            },
          }
        : null,
  );
  return (
    <SheetDialog
      width="wide"
      icon="settings"
      title="Connect account via recovery key"
      onClose={() => {
        setPhrase('');
        onClose();
      }}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            title={eligibility.title}
            disabled={
              !target || !device || !phrase || busy || eligibility.disabled
            }
            onClick={() => {
              const once = phrase;
              setPhrase('');
              setBusy(true);
              void controller
                .run(
                  () =>
                    recoverAccount(
                      bridge,
                      access,
                      store.server,
                      target,
                      once,
                      device,
                    ),
                  onDone,
                  onError,
                )
                .finally(controller.settled(() => setBusy(false)));
            }}
          >
            Recover
          </Button>
        </>
      }
    >
      <p>Enter a recovery phrase for this account to add this device to it.</p>
      <Inset>
        <Field label="Local alias" value={target} onChange={setTarget} />
        <Field label="Device name" value={device} onChange={setDevice} />
        <InsetRow label="Recovery phrase">
          <textarea
            value={phrase}
            onChange={(event) => setPhrase(event.target.value)}
          />
        </InsetRow>
      </Inset>
    </SheetDialog>
  );
}
