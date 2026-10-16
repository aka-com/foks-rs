/** Adds a connected hardware key as an authorized device of an account. */
import { useDeviceOperation } from '../use-device-operation';
import { provisionEnrollmentCommand } from '../enrollment-workflow';
import { useWorkflowAccess } from '../../../workflow-context';
import { useState } from 'react';
import type { ReactNode } from 'react';
import { useSheetGuard } from '../../../navigation-guard';
import {
  Band,
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

export function ProvisionSheet({
  store,
  cards,
  onClose,
  onDone,
  onError,
}: {
  store: AccountStore;
  cards: { serial: number }[];
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const { bridge } = useShellServices();
  const access = useWorkflowAccess();
  const workflowTarget = { profile: store.server, account: store.account };
  const operation = 'yubi-provision';
  const eligibility = access.props(operation, workflowTarget);
  const controller = useDeviceOperation(
    JSON.stringify([store?.id, operation, workflowTarget]),
  );
  const [targetAlias, setTargetAlias] = useState('');
  const [deviceName, setDeviceName] = useState(
    cards[0] ? `Hardware key ${cards[0].serial}` : 'Hardware key',
  );
  const [serial, setSerial] = useState(cards[0]?.serial ?? 0);
  const [pin, setPin] = useState('');
  const [puk, setPuk] = useState('');
  const [busy, setBusy] = useState(false);
  const clear = (): void => {
    setPin('');
    setPuk('');
  };
  // Provisioning writes the card in one operation, as enrollment does.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the card setup to finish.' }
      : pin || puk
        ? {
            verdict: 'prompt',
            title: 'Discard card credentials?',
            body: 'The PIN and unlock code typed here have not been submitted.',
            confirm: 'Discard',
            onConfirm: () => {
              clear();
              onClose();
            },
          }
        : null,
  );
  const valid = Boolean(
    cards.some((card) => card.serial === serial) &&
    targetAlias.trim() &&
    deviceName.trim() &&
    validSecurityKeyPin(pin) &&
    validSecurityKeyPin(puk),
  );
  return (
    <SheetDialog
      width="wide"
      icon="settings"
      title="Connect a hardware key"
      onClose={() => {
        clear();
        onClose();
      }}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabled={!valid || busy || eligibility.disabled}
            onClick={() => {
              const command = provisionEnrollmentCommand({
                accountStoreId: store.id,
                targetAlias,
                deviceName,
                cardSerial: serial,
                pin,
                puk,
              });
              clear();
              setBusy(true);
              void controller
                .run(
                  () =>
                    access.run(operation, workflowTarget, () =>
                      bridge.runYubi(command),
                    ),
                  onDone,
                  onError,
                )
                .finally(controller.settled(() => setBusy(false)));
            }}
          >
            Connect
          </Button>
        </>
      }
    >
      {cards.length ? null : (
        <Band label="Connect a hardware key">
          No security key is currently detected.
        </Band>
      )}
      <Inset>
        <Field
          label="Key alias"
          value={targetAlias}
          placeholder="new-key"
          onChange={setTargetAlias}
        />
        <Field
          label="Device name"
          value={deviceName}
          onChange={setDeviceName}
        />
        {cards.length ? (
          <InsetRow label="Connected card">
            <select
              value={serial}
              onChange={(event) => setSerial(Number(event.target.value))}
            >
              {cards.map((card) => (
                <option key={card.serial} value={card.serial}>
                  Hardware key {card.serial}
                </option>
              ))}
            </select>
          </InsetRow>
        ) : null}
        <SecurityKeyPinField label="Card PIN" value={pin} onChange={setPin} />
        <SecurityKeyPinField
          label="Unlock code"
          value={puk}
          onChange={setPuk}
        />
      </Inset>
    </SheetDialog>
  );
}
