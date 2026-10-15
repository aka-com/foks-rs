/** Creates an account whose device key lives on a connected hardware key. */
import { useDeviceOperation } from '../use-device-operation';
import {
  createEnrollmentCommand,
  enrollmentSlot,
  validEnrollmentAttempts,
} from '../enrollment-workflow';
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
import {
  SECURITY_KEY_RETRY_RANGE,
  validSecurityKeyPin,
} from '../../../security-key-pin';
import { useShellServices } from '../../../shell-services';

export function EnrollSheet({
  store,
  card,
  onClose,
  onDone,
  onError,
}: {
  store: AccountStore;
  card?: { serial: number };
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const { bridge } = useShellServices();
  const access = useWorkflowAccess();
  const workflowTarget = { profile: store.server };
  const operation = 'yubi-create';
  const eligibility = access.props(operation, workflowTarget);
  const controller = useDeviceOperation(
    JSON.stringify([store?.id, operation, workflowTarget]),
  );
  const [alias, setAlias] = useState('');
  const [username, setUsername] = useState('');
  // The card list resolves after the sheet opens, so the default names the
  // card once it is known — unless the field has already been typed into.
  const [deviceName, setDeviceName] = useState('');
  const [deviceNameTouched, setDeviceNameTouched] = useState(false);
  const suggestedName = card ? `Hardware key ${card.serial}` : 'Hardware key';
  const shownName = deviceNameTouched ? deviceName : suggestedName;
  const [invite, setInvite] = useState('');
  const [pin, setPin] = useState('');
  const [puk, setPuk] = useState('');
  const [signingSlot, setSigningSlot] = useState('0x82');
  const [pqSlot, setPqSlot] = useState('0x83');
  const [pinAttempts, setPinAttempts] = useState(3);
  const [pukAttempts, setPukAttempts] = useState(3);
  const [busy, setBusy] = useState(false);
  const clear = (): void => {
    setInvite('');
    setPin('');
    setPuk('');
  };
  // The card is written in a single operation that cannot be resumed, so the
  // sheet answers for a move away while it runs. Before it runs, the PIN and
  // the unlock code are values the reader can enter again.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the card setup to finish.' }
      : pin || puk || invite
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
  const signing = enrollmentSlot(signingSlot);
  const pq = enrollmentSlot(pqSlot);
  const validAttempts = validEnrollmentAttempts(pinAttempts, pukAttempts);
  return (
    <SheetDialog
      width="wide"
      icon="settings"
      title="Create an account on a hardware key"
      onClose={() => {
        clear();
        onClose();
      }}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            title={eligibility.title}
            disabled={
              eligibility.disabled ||
              !card ||
              !alias.trim() ||
              !username.trim() ||
              !shownName.trim() ||
              !validSecurityKeyPin(pin) ||
              !validSecurityKeyPin(puk) ||
              signing === null ||
              pq === null ||
              signing === pq ||
              !validAttempts ||
              busy
            }
            onClick={() => {
              if (signing === null || pq === null) return;
              const command = createEnrollmentCommand({
                profile: store.server,
                alias,
                username,
                deviceName: shownName,
                invite,
                cardSerial: card?.serial ?? 0,
                signingSlot: signing,
                pqSlot: pq,
                pin,
                puk,
                pinAttempts,
                pukAttempts,
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
            Create account
          </Button>
        </>
      }
    >
      <ol className="sheet-steps">
        <li>If setup fails, the card must be reset, erasing its data.</li>
        <li>The card must have factory settings.</li>
        <li>
          <b>Save your unlock code.</b> It cannot be recovered.
        </li>
      </ol>
      {card ? (
        <p>
          <b>Hardware key {card.serial}</b> is connected. Choose an alias for
          this key below.
        </p>
      ) : (
        <Band label="Connect a hardware key">
          No security key is currently detected.
        </Band>
      )}
      <Inset>
        <Field
          label="Alias"
          value={alias}
          placeholder="work-key"
          onChange={setAlias}
        />
        <Field label="Username" value={username} onChange={setUsername} />
        <Field
          label="Device name"
          value={shownName}
          onChange={(next) => {
            setDeviceNameTouched(true);
            setDeviceName(next);
          }}
        />
        <SecurityKeyPinField label="Card PIN" value={pin} onChange={setPin} />
        <SecurityKeyPinField
          label="Unlock code"
          value={puk}
          onChange={setPuk}
        />
        <InsetRow label="Invite">
          <input
            type="password"
            value={invite}
            onChange={(event) => setInvite(event.target.value)}
          />
        </InsetRow>
      </Inset>
      <details className="adv">
        <summary>Advanced</summary>
        <Inset>
          <Field
            label="Signing slot"
            value={signingSlot}
            onChange={setSigningSlot}
            mono
          />
          <Field
            label="Post-quantum slot"
            value={pqSlot}
            onChange={setPqSlot}
            mono
          />
          <InsetRow label="PIN tries">
            <input
              type="number"
              min={SECURITY_KEY_RETRY_RANGE.min}
              max={SECURITY_KEY_RETRY_RANGE.max}
              value={pinAttempts}
              onChange={(event) => setPinAttempts(Number(event.target.value))}
            />
          </InsetRow>
          <InsetRow label="PUK tries">
            <input
              type="number"
              min={SECURITY_KEY_RETRY_RANGE.min}
              max={SECURITY_KEY_RETRY_RANGE.max}
              value={pukAttempts}
              onChange={(event) => setPukAttempts(Number(event.target.value))}
            />
          </InsetRow>
        </Inset>
      </details>
    </SheetDialog>
  );
}
