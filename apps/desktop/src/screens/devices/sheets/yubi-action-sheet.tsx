/**
 * One hardware-key command that takes typed values: the card PIN, PUK,
 * passphrase, sync, recovery and rotation actions.
 */
import { credentialCommand } from '../credential-workflow';
import { useDeviceOperation } from '../use-device-operation';
import { useWorkflowAccess } from '../../../workflow-context';
import { useState } from 'react';
import type { ReactNode } from 'react';
import { useSheetGuard } from '../../../navigation-guard';
import {
  Button,
  Field,
  Inset,
  InsetRow,
  SecurityKeyPinField,
  SheetDialog,
} from '../../../components';
import type { AccountStore } from '../../../model';
import { validSecurityKeyPin } from '../../../security-key-pin';
import { useShellServices } from '../../../shell-services';
import { YUBI_ACTION_LABELS, YUBI_WORKFLOWS } from '../credential-actions';
import type { SimpleYubiAction } from '../credential-actions';

export function YubiActionSheet({
  store,
  profile,
  action,
  alias,
  onClose,
  onDone,
  onError,
}: {
  store?: AccountStore;
  profile?: string;
  action: SimpleYubiAction;
  alias: string;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const { bridge } = useShellServices();
  const access = useWorkflowAccess();
  const operation = YUBI_WORKFLOWS[action];
  const workflowTarget = {
    profile: store?.server ?? profile,
    account: action === 'recover-management' ? store?.account : alias,
  };
  const eligibility = access.props(operation, workflowTarget);
  const controller = useDeviceOperation(
    JSON.stringify([store?.id, action, workflowTarget]),
  );
  const [pin, setPin] = useState('');
  const [other, setOther] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  const needsPin = !['pin-status', 'recover-management'].includes(action);
  const needsOther = [
    'change-pin',
    'set-passphrase',
    'change-passphrase',
    'verify-passphrase',
    'unblock',
    'change-puk',
  ].includes(action);
  const needsConfirmation =
    action === 'set-passphrase' || action === 'change-passphrase';
  // The second value is a PIN or PUK unless the action sets a passphrase.
  const otherIsCredential = ['change-pin', 'unblock', 'change-puk'].includes(
    action,
  );
  const valid = Boolean(
    alias &&
    (!needsPin ||
      (action === 'resume-rotation' && !pin) ||
      validSecurityKeyPin(pin)) &&
    (!needsOther || (otherIsCredential ? validSecurityKeyPin(other) : other)) &&
    (!needsConfirmation || other === confirmation),
  );
  const firstLabel =
    action === 'unblock' || action === 'change-puk'
      ? 'Current PUK'
      : 'Card PIN';
  const otherLabel =
    action === 'change-pin' || action === 'unblock'
      ? 'New PIN'
      : action === 'change-puk'
        ? 'New PUK'
        : 'Passphrase';
  // The card is the only thing that knows whether a command it was given has
  // been applied, so a command already sent is not abandoned from here.
  useSheetGuard(
    busy
      ? {
          verdict: 'refuse',
          reason: 'Wait for the security key operation to finish.',
        }
      : pin || other || confirmation
        ? {
            verdict: 'prompt',
            title: 'Discard card credentials?',
            body: 'The values typed here have not been submitted.',
            confirm: 'Discard',
            onConfirm: () => {
              setPin('');
              setOther('');
              setConfirmation('');
              onClose();
            },
          }
        : null,
  );
  const submit = (): void => {
    const profileName = store?.server ?? profile;
    if (!profileName) return;
    const command = credentialCommand({
      action,
      profile: profileName,
      alias,
      store,
      pin,
      other,
      confirmation,
    });
    if (!command) return;
    setPin('');
    setOther('');
    setConfirmation('');
    setBusy(true);
    const task = () =>
      access.run(operation, workflowTarget, () => bridge.runYubi(command));
    if (action === 'pin-status' || action === 'verify-passphrase') {
      void controller
        .read(task, onDone, onError)
        .finally(controller.settled(() => setBusy(false)));
      return;
    }
    void controller
      .run(
        task,
        onDone,
        onError,
        action === 'resume-enrollment' || action === 'resume-rotation'
          ? { kind: 'resumable', operation: action }
          : { kind: 'mutation' },
      )
      .finally(controller.settled(() => setBusy(false)));
  };
  return (
    <SheetDialog
      width="wide"
      icon="settings"
      title={YUBI_ACTION_LABELS[action]}
      onClose={() => {
        setPin('');
        setOther('');
        setConfirmation('');
        onClose();
      }}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabled={!valid || busy || eligibility.disabled}
            onClick={submit}
          >
            Continue
          </Button>
        </>
      }
    >
      <Inset>
        <InsetRow label="Key alias">
          <b>{alias || 'No key enrolled'}</b>
        </InsetRow>
        {needsPin ? (
          <SecurityKeyPinField
            label={firstLabel}
            value={pin}
            onChange={setPin}
          />
        ) : null}
        {needsOther && otherIsCredential ? (
          <SecurityKeyPinField
            label={otherLabel}
            value={other}
            onChange={setOther}
          />
        ) : needsOther ? (
          <InsetRow label={otherLabel}>
            <input
              type="password"
              value={other}
              placeholder="Enter a passphrase"
              onChange={(event) => setOther(event.target.value)}
            />
          </InsetRow>
        ) : null}
        {needsConfirmation ? (
          <Field
            label="Confirm"
            placeholder="Repeat passphrase"
            value={confirmation}
            onChange={setConfirmation}
            type="password"
          />
        ) : null}
      </Inset>
    </SheetDialog>
  );
}
