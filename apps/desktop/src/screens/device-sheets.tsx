import { useTabSheetState } from '../navigation-guard';
import {
  accountPassphrase,
  accountPassphraseStatus,
  credentialCommand,
} from './devices/credential-workflow';
import { queuedDeviceWork } from './devices/operation-controller';
import { useDeviceOperation } from './devices/use-device-operation';
import {
  acceptPairing,
  finishPairing,
  recoverAccount,
} from './devices/pairing-workflow';
import {
  createEnrollmentCommand,
  provisionEnrollmentCommand,
  enrollmentSlot,
  validEnrollmentAttempts,
} from './devices/enrollment-workflow';
import { useWorkflowAccess } from '../workflow-context';
import type { WorkflowOperation } from '../model/workflow-availability';
/**
 * The sheets that act on one account's devices, keys and passphrase.
 *
 * They are shared: Devices opens the pairing, paper-key, YubiKey and removal
 * sheets, and Settings › Account opens the passphrase and card-credential
 * ones. Each is
 * the sheet that pane opened before the three tabs were split apart, with the
 * same commands behind it.
 */

import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useConcealOnInactive } from '../use-conceal-on-inactive';
import {
  removeDevice,
  revokePaperKey,
  revokeSecurityKey,
} from './devices/revocation-workflow';
import { useSheetGuard } from '../navigation-guard';
import type {
  AccountDevice,
  BackupEnrollment,
  Bridge,
  PairingOffer,
  PassphraseStatus,
} from '../bridge';
import { normalizeCommandError } from '../bridge';
import {
  Band,
  Button,
  CopyBox,
  Field,
  Icon,
  Inset,
  InsetRow,
  RadioCard,
  RadioGroup,
  SheetDialog,
} from '../components';
import type { AccountStore } from '../model';
import { deviceAlertRegistry } from './device-alert';

/** A YubiKey command that is one sheet of typed values. */
export type SimpleYubiAction =
  | 'sync'
  | 'pin-status'
  | 'change-pin'
  | 'set-passphrase'
  | 'change-passphrase'
  | 'verify-passphrase'
  | 'unblock'
  | 'change-puk'
  | 'recover-management'
  | 'recover-subkey'
  | 'resume-enrollment'
  | 'resume-rotation'
  | 'rotate';

/**
 * What "Add a device" leads to. Pairing is two directions, and
 * each is its own card: this Mac hands out a phrase, or types the one the
 * other Mac is showing.
 */
type AddChoice = 'pair' | 'pair-accept' | 'phrase' | 'provision';

/** What the passphrase sheet does with what is typed into it. */
export type PassphraseMode = 'set' | 'change';

/** The label a YubiKey sheet's title reads, by the action it performs. */
export const YUBI_ACTION_LABELS: Readonly<Record<SimpleYubiAction, string>> = {
  sync: 'Sync the key account',
  'pin-status': 'Card PIN status',
  'change-pin': 'Change the card PIN',
  'set-passphrase': 'Set the key passphrase',
  'change-passphrase': 'Change the key passphrase',
  'verify-passphrase': 'Verify the key passphrase',
  unblock: 'Unblock the card PIN',
  'change-puk': 'Change the unlock code',
  'recover-management': 'Restore management access',
  'recover-subkey': 'Restore the signing key',
  'resume-enrollment': 'Resume enrollment',
  'resume-rotation': 'Resume key rotation',
  rotate: 'Rotate the management key',
};

export const YUBI_WORKFLOWS: Readonly<
  Record<SimpleYubiAction, WorkflowOperation>
> = {
  sync: 'account-sync',
  'pin-status': 'yubi-pin',
  'change-pin': 'yubi-pin',
  'set-passphrase': 'passphrase',
  'change-passphrase': 'passphrase',
  'verify-passphrase': 'passphrase',
  unblock: 'yubi-pin',
  'change-puk': 'yubi-pin',
  'recover-management': 'yubi-recover-management',
  'recover-subkey': 'yubi-recover-subkey',
  'resume-enrollment': 'yubi-resume',
  'resume-rotation': 'yubi-rotate',
  rotate: 'yubi-rotate',
};

/**
 * The chooser the Devices page's primary action opens: the ways this account
 * gains a key, each leading to the sheet that already did it.
 */
export function AddDeviceSheet({
  store,
  onChoose,
  onClose,
}: {
  store: AccountStore;
  onChoose: (choice: AddChoice) => void;
  onClose: () => void;
}): ReactNode {
  const access = useWorkflowAccess();
  const [choice, setChoice] = useState<AddChoice>('pair');
  const operation: WorkflowOperation =
    choice === 'phrase'
      ? 'backup-create'
      : choice === 'provision'
        ? 'yubi-provision'
        : choice === 'pair-accept'
          ? 'device-accept'
          : 'device-pair';
  const eligibility = access.props(operation, {
    profile: store.server,
    account: choice === 'pair-accept' ? undefined : store.account,
  });
  return (
    <DeviceSheetFrame
      title="Add a device"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            {...eligibility}
            onClick={() => {
              if (
                !access.availability(operation, {
                  profile: store.server,
                  account: choice === 'pair-accept' ? undefined : store.account,
                }).available
              )
                return;
              onChoose(choice);
            }}
          >
            Continue
          </Button>
        </>
      }
    >
      <RadioGroup label="What to add">
        <RadioCard
          icon="laptop"
          title="Pair another device"
          detail="Get a pairing phrase here, to type on the device you are adding."
          selected={choice === 'pair'}
          onSelect={() => setChoice('pair')}
        />
        <RadioCard
          icon="laptop"
          title="Pair this device with another account"
          detail="Add this device to another account, by entering the pairing phrase displayed on your other device."
          selected={choice === 'pair-accept'}
          onSelect={() => setChoice('pair-accept')}
        />
        <RadioCard
          icon="file"
          title="Create a new recovery paper key"
          detail="Add a phrase you can recover this account with."
          selected={choice === 'phrase'}
          onSelect={() => setChoice('phrase')}
        />
        <RadioCard
          icon="key"
          title="Connect a new YubiKey"
          detail="Add a connected YubiKey as an authorized device for this account."
          selected={choice === 'provision'}
          onSelect={() => setChoice('provision')}
        />
      </RadioGroup>
    </DeviceSheetFrame>
  );
}

/** The frame every sheet on this page shares: a glyph, a title and a footer. */
function DeviceSheetFrame({
  title,
  children,
  footer,
  onClose,
  danger = false,
  dismissible = true,
}: {
  title: string;
  children: ReactNode;
  footer: ReactNode;
  onClose: () => void;
  danger?: boolean;
  dismissible?: boolean;
}): ReactNode {
  return (
    <SheetDialog
      width="wide"
      danger={danger}
      onClose={onClose}
      dismissible={dismissible}
      title={title}
      footer={footer}
      glyph={
        <span className={`server-mark ${danger ? 'danger' : ''}`}>
          <Icon name={danger ? 'trash' : 'gear'} />
        </span>
      }
    >
      {children}
    </SheetDialog>
  );
}

export function PhraseSheet({
  bridge,
  profile,
  accountAlias,
  seedPhrase,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  profile: string;
  accountAlias: string;
  seedPhrase?: string;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const access = useWorkflowAccess();
  const target = { profile, account: accountAlias };
  const eligibility = access.props('backup-create', target);
  const controller = useDeviceOperation(
    JSON.stringify([profile, accountAlias]),
  );
  const [phrase, setPhrase] = useState<string | null>(() => seedPhrase ?? null);
  const [alias, setAlias] = useState('paper-backup');
  const [written, setWritten] = useState(false);
  const [busy, setBusy] = useState(false);
  const words = phrase?.split(/\s+/) ?? [];
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const discard = (): void => {
    setPhrase(null);
    setWritten(false);
    onClose();
  };
  // Navigation requires saving or explicitly dismissing the displayed key.
  useSheetGuard(
    phrase
      ? { verdict: 'refuse', reason: 'Save or dismiss the paper key first.' }
      : busy
        ? { verdict: 'refuse', reason: 'Wait for the paper key to finish.' }
        : null,
  );
  return (
    <DeviceSheetFrame
      title={phrase ? 'Save paper key' : 'Create paper key'}
      onClose={discard}
      dismissible={!busy}
      footer={
        <>
          {phrase ? (
            <>
              <Button disabled={busy} onClick={discard}>
                Cancel
              </Button>
              <Button
                variant="primary"
                title={eligibility.title}
                disabled={!written || busy || eligibility.disabled}
                onClick={() => {
                  setBusy(true);
                  const once = phrase;
                  setPhrase(null);
                  void controller
                    .run(
                      () =>
                        access.run('backup-create', target, () =>
                          bridge.commitOwnerBackup(
                            profile,
                            accountAlias,
                            alias,
                            once,
                          ),
                        ),
                      onDone,
                      onError,
                    )
                    .finally(controller.settled(() => setBusy(false)));
                }}
              >
                Save paper key
              </Button>
            </>
          ) : (
            <>
              <Button onClick={discard}>Cancel</Button>
              <Button
                variant="primary"
                title={eligibility.title}
                disabled={!alias.trim() || busy || eligibility.disabled}
                onClick={() => {
                  setBusy(true);
                  void access
                    .run('backup-create', target, () =>
                      bridge.prepareOwnerBackup(
                        profile,
                        accountAlias,
                        alias.trim(),
                      ),
                    )
                    .then((result) => {
                      if (mounted.current) setPhrase(result.phrase);
                    })
                    .catch(onError)
                    .finally(() => setBusy(false));
                }}
              >
                Generate phrase
              </Button>
            </>
          )}
        </>
      }
    >
      {phrase ? (
        <>
          <p>
            Write these {words.length} words down now. The phrase cannot be
            shown again after you close this.
          </p>
          <div className="words">
            {words.map((word, index) => (
              <span className="word" key={`${index}-${word}`}>
                <i>{index + 1}</i>
                {word}
              </span>
            ))}
          </div>
          <label className="checkline">
            <input
              type="checkbox"
              checked={written}
              onChange={(event) => setWritten(event.target.checked)}
            />
            I have written down these words
          </label>
        </>
      ) : (
        <Inset>
          <Field label="Paper key name" value={alias} onChange={setAlias} />
        </Inset>
      )}
    </DeviceSheetFrame>
  );
}

export function PairSheet({
  bridge,
  store,
  initialMode,
  onCopy,
  onBack,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  initialMode: 'offer' | 'accept';
  /** Copies the revealed phrase, which is live-form only. */
  onCopy: (text: string) => void;
  /** Back to the chooser, which is where the direction was picked. */
  onBack?: () => void;
  onClose: () => void;
  onDone: (message: string) => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  // The direction is the chooser's answer, and the two steps are written for
  // it: there is no control here that silently changes what Start would do.
  const mode = initialMode;
  const access = useWorkflowAccess();
  const operation = mode === 'offer' ? 'device-pair' : 'device-accept';
  const workflowTarget = {
    profile: store.server,
    account: mode === 'offer' ? store.account : undefined,
  };
  const eligibility = access.props(operation, workflowTarget);
  const controller = useDeviceOperation(
    JSON.stringify([store?.id, operation, workflowTarget]),
  );
  const [offer, setOffer] = useState<PairingOffer | null>(null);
  /** Whether the phrase on screen came from an offer the agent still held. */
  const [resumed, setResumed] = useState(false);
  const [target, setTarget] = useTabSheetState(
    'pairing.target',
    store.account,
    mode === 'accept',
  );
  const [device, setDevice] = useTabSheetState(
    'pairing.device',
    'This device',
    mode === 'accept',
  );
  const [phrase, setPhrase] = useTabSheetState(
    'pairing.phrase',
    '',
    mode === 'accept',
  );
  const [busy, setBusy] = useState(false);
  useConcealOnInactive(
    () => {
      setOffer(null);
      setResumed(false);
    },
    mode === 'offer' && offer !== null,
  );
  const queued = <T,>(task: () => Promise<T>): Promise<T> =>
    queuedDeviceWork(
      bridge,
      store.server,
      access,
      operation,
      workflowTarget,
      task,
    );
  const act = (
    task: () => Promise<unknown>,
    message: string,
    onSuccess?: () => void,
  ): void => {
    setOffer(null);
    setPhrase('');
    setBusy(true);
    void controller
      .run(
        () => queued(task),
        async () => {
          onSuccess?.();
          await onDone(message);
        },
        onError,
        { kind: 'resumable', operation: 'device-pairing' },
      )
      .finally(controller.settled(() => setBusy(false)));
  };
  const revealOffer = (
    task: () => Promise<PairingOffer>,
    held = false,
  ): void => {
    setOffer(null);
    setResumed(held);
    setBusy(true);
    const isCurrent = controller.capture();
    void queued(task)
      .then((next) => {
        if (!isCurrent()) return;
        if (next.accountAlias !== store.account)
          throw new Error('pairing offer returned a different account.');
        setOffer(next);
        // The Devices tab's rail dot has no way to ask the agent whether an
        // offer is open; this is the one place that ever finds out.
        deviceAlertRegistry(bridge).reportPairingOffer(store.id, true);
      })
      .catch((error) => {
        if (isCurrent()) onError(error);
      })
      .finally(controller.settled(() => setBusy(false)));
  };
  // An offer the agent is holding, and a call already sent, are both a pairing
  // that has to be ended where it was begun. A phrase typed into the accept
  // form is only typing: it can be asked about and typed again.
  useSheetGuard(
    busy || offer
      ? { verdict: 'refuse', reason: 'Finish or cancel the pairing first.' }
      : phrase
        ? {
            verdict: 'prompt',
            title: 'Discard pairing phrase?',
            body: 'The pairing phrase typed here has not been submitted.',
            confirm: 'Discard',
            onConfirm: () => {
              setPhrase('');
              onClose();
            },
          }
        : null,
    mode === 'accept' && !busy,
  );
  return (
    <DeviceSheetFrame
      title="Pair another device"
      onClose={() => {
        if (busy) return;
        setOffer(null);
        setPhrase('');
        onClose();
      }}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Close
          </Button>
          <span className="spacer" />
          {onBack ? (
            <Button disabled={busy} onClick={onBack}>
              Back
            </Button>
          ) : null}
          {mode === 'offer' ? (
            <Button
              variant="primary"
              disabled={!offer || busy || eligibility.disabled}
              onClick={() =>
                act(
                  () => finishPairing(bridge, store),
                  'Device paired successfully.',
                  () =>
                    deviceAlertRegistry(bridge).reportPairingOffer(
                      store.id,
                      false,
                    ),
                )
              }
            >
              Finish
            </Button>
          ) : (
            <>
              <Button
                title={eligibility.title}
                disabled={busy || eligibility.disabled || !target}
                onClick={() =>
                  act(
                    () => acceptPairing(bridge, store.server, target, 'resume'),
                    'Device paired successfully.',
                  )
                }
              >
                Resume acceptance
              </Button>
              <Button
                variant="primary"
                title={eligibility.title}
                disabled={
                  busy || eligibility.disabled || !target || !device || !phrase
                }
                onClick={() =>
                  act(
                    () =>
                      acceptPairing(bridge, store.server, target, {
                        device,
                        phrase,
                      }),
                    'Device paired successfully.',
                  )
                }
              >
                Accept
              </Button>
            </>
          )}
        </>
      }
    >
      {/* The agent holds at most one offer per account and reports neither
          when it was made nor when it expires, so the band says what is true:
          an offer is open, and resuming shows the same phrase again. */}
      {resumed && offer ? (
        <Band label="A pairing is waiting on this device">
          The phrase below is the one already issued.
        </Band>
      ) : null}
      {mode === 'offer' ? (
        <ol className="pair-steps">
          <li>
            <b>On this device</b>
            {offer ? (
              <>
                <p>
                  Type this phrase on the other device. Use Resume offer to show
                  it again while pairing is pending.
                </p>
                <CopyBox text={offer.phrase} onCopy={onCopy}>
                  <span className="mono">{offer.phrase}</span>
                </CopyBox>
              </>
            ) : (
              <>
                <p>Get a pairing phrase.</p>
                <span className="steprow">
                  <Button
                    variant="primary"
                    title={eligibility.title}
                    disabled={busy || eligibility.disabled}
                    onClick={() =>
                      revealOffer(() => bridge.startDevicePairing(store.id))
                    }
                  >
                    Start
                  </Button>
                  <Button
                    title={eligibility.title}
                    disabled={busy || eligibility.disabled}
                    onClick={() =>
                      revealOffer(
                        () => bridge.resumeDevicePairingOffer(store.id),
                        true,
                      )
                    }
                  >
                    Resume offer
                  </Button>
                </span>
              </>
            )}
          </li>
          <li className={offer ? undefined : 'off'}>
            <b>On the other device</b>
            <p>
              Open FOKS there, choose Add a device › Pair this device with
              another account, and type the phrase. Then click Finish here.
            </p>
          </li>
        </ol>
      ) : (
        <ol className="pair-steps">
          <li>
            <b>On the other device</b>
            <p>
              Open FOKS there, choose Add a device › Pair another device, and
              start or resume a pairing to show its phrase.
            </p>
          </li>
          <li>
            <b>On this device</b>
          </li>
        </ol>
      )}
      {mode === 'accept' ? (
        <Inset>
          <Field label="Account alias" value={target} onChange={setTarget} />
          <Field label="Device name" value={device} onChange={setDevice} />
          <Field
            label="Pairing phrase"
            value={phrase}
            onChange={setPhrase}
            type="password"
          />
        </Inset>
      ) : null}
    </DeviceSheetFrame>
  );
}

export function RecoverSheet({
  bridge,
  store,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
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
            title: 'Discard paper key phrase?',
            body: 'The paper key phrase typed here has not been submitted.',
            confirm: 'Discard',
            onConfirm: () => {
              setPhrase('');
              onClose();
            },
          }
        : null,
  );
  return (
    <DeviceSheetFrame
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
      <p>Enter a paper key phrase for this account to add this device to it.</p>
      <Inset>
        <Field label="Local alias" value={target} onChange={setTarget} />
        <Field label="Device name" value={device} onChange={setDevice} />
        <InsetRow label="Paper key phrase">
          <textarea
            value={phrase}
            onChange={(event) => setPhrase(event.target.value)}
          />
        </InsetRow>
      </Inset>
    </DeviceSheetFrame>
  );
}

export function EnrollSheet({
  bridge,
  store,
  card,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  card?: { serial: number };
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
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
  const suggestedName = card ? `YubiKey ${card.serial}` : 'YubiKey';
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
    <DeviceSheetFrame
      title="Create a YubiKey account"
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
              !pin ||
              !puk ||
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
          <b>YubiKey {card.serial}</b> is connected. Choose an alias for this
          key below.
        </p>
      ) : (
        <Band label="Connect a YubiKey">
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
        <Field label="Card PIN" value={pin} onChange={setPin} type="password" />
        <Field
          label="Unlock code"
          value={puk}
          onChange={setPuk}
          type="password"
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
              min={1}
              max={255}
              value={pinAttempts}
              onChange={(event) => setPinAttempts(Number(event.target.value))}
            />
          </InsetRow>
          <InsetRow label="PUK tries">
            <input
              type="number"
              min={1}
              max={255}
              value={pukAttempts}
              onChange={(event) => setPukAttempts(Number(event.target.value))}
            />
          </InsetRow>
        </Inset>
      </details>
    </DeviceSheetFrame>
  );
}

export function ProvisionSheet({
  bridge,
  store,
  cards,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  cards: { serial: number }[];
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const access = useWorkflowAccess();
  const workflowTarget = { profile: store.server, account: store.account };
  const operation = 'yubi-provision';
  const eligibility = access.props(operation, workflowTarget);
  const controller = useDeviceOperation(
    JSON.stringify([store?.id, operation, workflowTarget]),
  );
  const [targetAlias, setTargetAlias] = useState('');
  const [deviceName, setDeviceName] = useState(
    cards[0] ? `YubiKey ${cards[0].serial}` : 'YubiKey',
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
    pin &&
    puk,
  );
  return (
    <DeviceSheetFrame
      title="Connect a YubiKey"
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
        <Band label="Connect a YubiKey">
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
                  YubiKey {card.serial}
                </option>
              ))}
            </select>
          </InsetRow>
        ) : null}
        <Field label="Card PIN" value={pin} onChange={setPin} type="password" />
        <Field
          label="Unlock code"
          value={puk}
          onChange={setPuk}
          type="password"
        />
      </Inset>
    </DeviceSheetFrame>
  );
}

export function YubiActionSheet({
  bridge,
  store,
  profile,
  action,
  alias,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store?: AccountStore;
  profile?: string;
  action: SimpleYubiAction;
  alias: string;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
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
  const valid = Boolean(
    alias &&
    (!needsPin || action === 'resume-rotation' || pin) &&
    (!needsOther || other) &&
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
    <DeviceSheetFrame
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
          <InsetRow label={firstLabel}>
            <input
              type="password"
              value={pin}
              onChange={(event) => setPin(event.target.value)}
            />
          </InsetRow>
        ) : null}
        {needsOther ? (
          <InsetRow label={otherLabel}>
            <input
              type="password"
              value={other}
              onChange={(event) => setOther(event.target.value)}
            />
          </InsetRow>
        ) : null}
        {needsConfirmation ? (
          <Field
            label="Confirm"
            value={confirmation}
            onChange={setConfirmation}
            type="password"
          />
        ) : null}
      </Inset>
    </DeviceSheetFrame>
  );
}

export function RevokeSheet({
  bridge,
  store,
  alias,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  alias: string;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
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
    <DeviceSheetFrame
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
    </DeviceSheetFrame>
  );
}

export function RevokeBackupSheet({
  bridge,
  store,
  backup,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  backup: BackupEnrollment;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const access = useWorkflowAccess();
  const workflowTarget = { profile: store.server, account: store.account };
  const eligibility = access.props('backup-revoke', workflowTarget);
  const controller = useDeviceOperation(
    JSON.stringify([store.id, workflowTarget, backup.backupId]),
  );
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  // Revoking a paper key rotates every account key it could read.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the revocation to finish.' }
      : null,
  );
  return (
    <DeviceSheetFrame
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
                    revokePaperKey(bridge, access, store, backup, confirmation),
                  onDone,
                  onError,
                )
                .finally(controller.settled(() => setBusy(false)));
            }}
          >
            Revoke paper key
          </Button>
        </>
      }
    >
      <p>
        This paper key can no longer recover the account, and the account keys
        are rotated. Type its name to confirm.
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
    </DeviceSheetFrame>
  );
}

export function RemoveDeviceSheet({
  bridge,
  store,
  device,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  device: AccountDevice;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const access = useWorkflowAccess();
  const workflowTarget = { profile: store.server, account: store.account };
  const eligibility = access.props('device-remove', workflowTarget);
  const controller = useDeviceOperation(
    JSON.stringify([store.id, workflowTarget, device.id]),
  );
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  const expected = device.name ?? device.id;
  // Removing a device is a write the agent has already been given.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the removal to finish.' }
      : null,
  );
  return (
    <DeviceSheetFrame
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
            disabled={confirmation !== expected || busy || eligibility.disabled}
            onClick={() => {
              setBusy(true);
              // The display cache can outlive the native catalog's retained
              // device list. Re-read before a destructive action so native
              // target/current-device validation uses fresh records.
              void controller
                .run(
                  () => removeDevice(bridge, access, store, device.id),
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
    </DeviceSheetFrame>
  );
}

export function PassphraseSheet({
  bridge,
  store,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  onClose: () => void;
  onDone: (message: string) => void | Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const access = useWorkflowAccess();
  const workflowTarget = { profile: store.server, account: store.account };
  const eligibility = access.props('passphrase', workflowTarget);
  const controller = useDeviceOperation(
    JSON.stringify([store.id, workflowTarget]),
  );
  // The server accepts enrollment and rotation under mutually exclusive
  // conditions, so the mode is read from it rather than chosen by the reader.
  // `reload` re-reads it after a lost race.
  const [status, setStatus] = useState<PassphraseStatus | null>(null);
  const [unreadable, setUnreadable] = useState(false);
  const [reload, setReload] = useState(0);
  const [loading, setLoading] = useState(true);
  const [current, setCurrent] = useState('');
  const [currentRejected, setCurrentRejected] = useState(false);
  const [rateLimited, setRateLimited] = useState(false);
  const [passphrase, setPassphrase] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    setLoading(true);
    setUnreadable(false);
    void controller
      .read(
        () => accountPassphraseStatus(bridge, access, store),
        async (value) => {
          setStatus(value);
          setCurrentRejected(false);
          setRateLimited(false);
        },
        (error) => {
          // Neither operation can be offered without knowing which one the
          // server will accept, so the sheet says so rather than guessing.
          setUnreadable(true);
          onError(error);
        },
      )
      .finally(controller.settled(() => setLoading(false)));
    // The controller and access objects are rebuilt every render; the read is
    // keyed by the account it reads and by an explicit re-read request.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [store.id, reload]);
  const mode: PassphraseMode | null =
    status === null ? null : status.configured ? 'change' : 'set';
  // A change is authorized by this device, so the current passphrase is a
  // confirmation step rather than the authorization. It is always required.
  const checking = mode === 'change';
  const submit = (): void => {
    if (mode === null || loading || unreadable || busy || rateLimited) return;
    const offered = checking ? current : null;
    const secret = passphrase;
    const repeated = confirmation;
    setBusy(true);
    setCurrentRejected(false);
    setRateLimited(false);
    const task = () =>
      accountPassphrase(bridge, access, store, mode, offered, secret, repeated);
    const complete = async (
      report: Awaited<ReturnType<typeof accountPassphrase>>,
    ): Promise<void> => {
      setCurrent('');
      setPassphrase('');
      setConfirmation('');
      await onDone(
        mode === 'set'
          ? `Passphrase set (generation ${report.generation}).`
          : `Passphrase changed (generation ${report.generation}).`,
      );
    };
    const failed = (error: unknown): void => {
      const { code } = normalizeCommandError(error);
      // The check ran before the change, so a rejection belongs on the field
      // that carried it and leaves the rest of the form intact.
      if (code === 'current-passphrase-rejected') setCurrentRejected(true);
      // The passphrase moved underneath this sheet, on this device or
      // another. Whichever operation applies now is decided by a fresh read,
      // not by this form.
      if (code === 'conflict') setReload((generation) => generation + 1);
      // The check is never dropped, so a refused check holds the change
      // back until the server will run it again. This code also covers a
      // server that is merely busy, so editing the field clears it rather
      // than stranding a reader whose passphrase was never wrong.
      if (code === 'rate-limited' && checking) setRateLimited(true);
      onError(error);
    };
    void controller
      .run(task, complete, failed)
      .finally(controller.settled(() => setBusy(false)));
  };
  // A passphrase that is being set or changed is a write in flight; one that
  // has only been typed is worth a question, since it was typed twice.
  useSheetGuard(
    busy
      ? {
          verdict: 'refuse',
          reason: 'Wait for the passphrase update to finish.',
        }
      : current || passphrase || confirmation
        ? {
            verdict: 'prompt',
            title: 'Discard passphrase?',
            body: 'The passphrase typed here has not been submitted.',
            confirm: 'Discard',
            onConfirm: () => {
              setCurrent('');
              setPassphrase('');
              setConfirmation('');
              onClose();
            },
          }
        : null,
  );
  const incomplete =
    !passphrase || passphrase !== confirmation || (checking && !current);
  return (
    <DeviceSheetFrame
      title={
        mode === null
          ? 'Account passphrase'
          : mode === 'set'
            ? 'Set passphrase'
            : 'Change passphrase'
      }
      onClose={() => {
        if (busy) return;
        setCurrent('');
        setPassphrase('');
        setConfirmation('');
        onClose();
      }}
      footer={
        <>
          <span className="spacer" />
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            title={eligibility.title}
            disabled={
              eligibility.disabled ||
              mode === null ||
              loading ||
              unreadable ||
              incomplete ||
              busy ||
              rateLimited
            }
            onClick={submit}
          >
            {busy
              ? mode === 'set'
                ? 'Setting…'
                : 'Changing…'
              : mode === 'set'
                ? 'Set'
                : 'Change'}
          </Button>
        </>
      }
    >
      {unreadable ? (
        <Band
          label="Passphrase state unavailable"
          severity="crit"
          live
          action={
            <Button
              size="sm"
              onClick={() => setReload((generation) => generation + 1)}
            >
              Retry
            </Button>
          }
        >
          This account's passphrase state could not be read from its server.
          Setting and changing a passphrase are accepted under different
          conditions, so neither is offered until it is known which applies.
        </Band>
      ) : loading || mode === null ? (
        <p className="hint">Reading this account's passphrase state…</p>
      ) : (
        <>
          <p>
            {mode === 'set'
              ? 'This account has no passphrase yet.'
              : 'Replaces your current passphrase.'}
          </p>
          {checking && rateLimited ? (
            <Band label="Too many incorrect attempts" severity="crit" live>
              Passphrase checks for this account are temporarily rate-limited.
              Your passphrase has not been changed. Try again later.
            </Band>
          ) : null}
          <Inset>
            {checking ? (
              <Field
                label="Current"
                value={current}
                onChange={(value) => {
                  setCurrent(value);
                  setCurrentRejected(false);
                  setRateLimited(false);
                }}
                type="password"
                hint={currentRejected ? 'Incorrect passphrase.' : undefined}
              />
            ) : null}
            <Field
              label={mode === 'set' ? 'Passphrase' : 'New'}
              value={passphrase}
              onChange={setPassphrase}
              type="password"
            />
            <Field
              label="Confirm"
              value={confirmation}
              onChange={setConfirmation}
              type="password"
            />
          </Inset>
          {checking ? (
            <p className="hint">
              Your current passphrase is checked before the change is submitted.
              Repeated wrong attempts temporarily rate-limit this account.
            </p>
          ) : null}
        </>
      )}
    </DeviceSheetFrame>
  );
}
