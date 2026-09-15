/**
 * The sheets that act on one account's devices, keys and passphrase.
 *
 * They are shared: Devices opens the pairing, paper-key, YubiKey and removal
 * sheets, and Settings opens the passphrase and card-credential ones. Each is
 * the sheet that pane opened before the three tabs were split apart, with the
 * same commands behind it.
 */

import { useState } from 'react';
import type { ReactNode } from 'react';
import { enqueueProfileWork } from '../bridge';
import type {
  AccountDevice,
  BackupEnrollment,
  Bridge,
  PairingOffer,
  YubiCommand,
} from '../bridge';
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
  SegmentedControl,
  SheetDialog,
} from '../components';
import type { AccountStore } from '../model';

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
 * What "Add a device or paper key" leads to. Pairing is two directions, and
 * each is its own card: this Mac hands out a phrase, or types the one the
 * other Mac is showing.
 */
type AddChoice = 'pair' | 'pair-accept' | 'phrase' | 'provision';

/** What the passphrase sheet does with what is typed into it. */
export type PassphraseMode = 'set' | 'change' | 'verify';

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

/**
 * The chooser the Devices page's primary action opens: the ways this account
 * gains a key, each leading to the sheet that already did it.
 */
export function AddDeviceSheet({
  subtitle,
  onChoose,
  onClose,
}: {
  subtitle: string;
  onChoose: (choice: AddChoice) => void;
  onClose: () => void;
}): ReactNode {
  const [choice, setChoice] = useState<AddChoice>('pair');
  return (
    <DeviceSheetFrame
      title="Add a device, paper key, or security key"
      subtitle={subtitle}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" onClick={() => onChoose(choice)}>
            Continue
          </Button>
        </>
      }
    >
      <p>
        Every device holds its own key. Nothing here copies a key between
        devices.
      </p>
      <RadioGroup label="What to add">
        <RadioCard
          icon="laptop"
          title="Pair another Mac"
          detail="Get a pairing phrase here and type it on the Mac you are adding."
          selected={choice === 'pair'}
          onSelect={() => setChoice('pair')}
        />
        <RadioCard
          icon="laptop"
          title="Enter a pairing phrase"
          detail="Add this Mac to the account by entering the pairing phrase displayed on your other device."
          selected={choice === 'pair-accept'}
          onSelect={() => setChoice('pair-accept')}
        />
        <RadioCard
          icon="file"
          title="Paper key"
          detail="A phrase you write down and can recover this account with."
          selected={choice === 'phrase'}
          onSelect={() => setChoice('phrase')}
        />
        <RadioCard
          icon="key"
          title="Security key"
          detail="Add the connected YubiKey as an authorized device for this account."
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
  subtitle,
  children,
  footer,
  onClose,
  danger = false,
  dismissible = true,
}: {
  title: string;
  subtitle: string;
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
      subtitle={subtitle}
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
  username,
  server,
  seedPhrase,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  profile: string;
  accountAlias: string;
  username: string;
  server: string;
  seedPhrase?: string;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [phrase, setPhrase] = useState<string | null>(() => seedPhrase ?? null);
  const [alias, setAlias] = useState('paper-backup');
  const [written, setWritten] = useState(false);
  const [busy, setBusy] = useState(false);
  const words = phrase?.split(/\s+/) ?? [];
  // Leaving the sheet with a phrase on screen is the same act as Cancel, by
  // whichever of the three ways out it is done: the phrase is discarded, not
  // committed. `prepare_owner_backup` wrote nothing, so there is nothing left
  // to clean up, and a revealed phrase is not a reason to trap the reader.
  const discard = (): void => {
    setPhrase(null);
    setWritten(false);
    onClose();
  };
  return (
    <DeviceSheetFrame
      title={phrase ? 'Save paper key' : 'Create paper key'}
      subtitle={`Generate a paper key for ${username} on ${server}`}
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
                disabled={!written || busy}
                onClick={() => {
                  setBusy(true);
                  const once = phrase;
                  setPhrase(null);
                  void bridge
                    .commitOwnerBackup(profile, accountAlias, alias, once)
                    .then(onDone)
                    .catch(onError)
                    .finally(() => setBusy(false));
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
                disabled={!alias.trim() || busy}
                onClick={() => {
                  setBusy(true);
                  void bridge
                    .prepareOwnerBackup(profile, accountAlias, alias.trim())
                    .then((result) => setPhrase(result.phrase))
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
            These {words.length} words are the key. Write them down now. This
            Mac shows them once and does not copy them; nothing else stores
            them.
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
  subtitle,
  initialMode,
  onCopy,
  onBack,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  /** Who this Mac pairs as, and where. */
  subtitle: string;
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
  const [offer, setOffer] = useState<PairingOffer | null>(null);
  /** Whether the phrase on screen came from an offer the agent still held. */
  const [resumed, setResumed] = useState(false);
  const [target, setTarget] = useState(store.account);
  const [device, setDevice] = useState('This Mac');
  const [phrase, setPhrase] = useState('');
  const [busy, setBusy] = useState(false);
  const queued = <T,>(task: () => Promise<T>): Promise<T> =>
    enqueueProfileWork(bridge, store.server, task);
  const act = (task: () => Promise<unknown>, message: string): void => {
    setOffer(null);
    setPhrase('');
    setBusy(true);
    void queued(task)
      .then(() => onDone(message))
      .catch(onError)
      .finally(() => setBusy(false));
  };
  const revealOffer = (
    task: () => Promise<PairingOffer>,
    held = false,
  ): void => {
    setOffer(null);
    setResumed(held);
    setBusy(true);
    void queued(task)
      .then((next) => {
        if (next.accountAlias !== store.account)
          throw new Error('pairing offer returned a different account.');
        setOffer(next);
      })
      .catch(onError)
      .finally(() => setBusy(false));
  };
  return (
    <DeviceSheetFrame
      title="Set up another Mac"
      subtitle={subtitle}
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
              disabled={!offer || busy}
              onClick={() =>
                act(async () => {
                  const result = await bridge.finishDevicePairing(store.id);
                  if (result.alias !== store.account)
                    throw new Error(
                      'finish_device_pairing returned a different account.',
                    );
                  return result;
                }, 'Device paired successfully.')
              }
            >
              Finish
            </Button>
          ) : (
            <>
              <Button
                disabled={busy || !target}
                onClick={() =>
                  act(async () => {
                    const result = await bridge.resumeDevicePairingAcceptance(
                      store.server,
                      target,
                    );
                    if (result.alias !== target)
                      throw new Error(
                        'resume_device_pairing_acceptance returned a different account.',
                      );
                    return result;
                  }, 'Device paired successfully.')
                }
              >
                Resume acceptance
              </Button>
              <Button
                variant="primary"
                disabled={busy || !target || !device || !phrase}
                onClick={() =>
                  act(async () => {
                    const result = await bridge.acceptDevicePairing(
                      store.server,
                      target,
                      device,
                      phrase,
                    );
                    if (result.alias !== target)
                      throw new Error(
                        'accept_device_pairing returned a different account.',
                      );
                    return result;
                  }, 'Device paired successfully.')
                }
              >
                Accept
              </Button>
            </>
          )}
        </>
      }
    >
      <p>
        {mode === 'offer'
          ? 'Complete step 1 here and step 2 on the other Mac, then return here to finish setup.'
          : 'Do step 1 on the other Mac and step 2 here.'}
      </p>
      {/* The agent holds at most one offer per account and reports neither
          when it was made nor when it expires, so the band says what is true:
          an offer is open, and resuming shows the same phrase again. */}
      {resumed && offer ? (
        <Band label="A pairing is waiting on this Mac">
          {subtitle} · the agent still holds this offer, and the phrase below is
          the same one it issued.
        </Band>
      ) : null}
      {mode === 'offer' ? (
        <ol className="pair-steps">
          <li>
            <b>On this Mac</b>
            {offer ? (
              <>
                <p>
                  Type this pairing phrase on the other Mac. It is shown once
                  and is never stored on this Mac.
                </p>
                <CopyBox text={offer.phrase} onCopy={onCopy}>
                  <span className="mono">{offer.phrase}</span>
                </CopyBox>
              </>
            ) : (
              <>
                <p>Get a pairing phrase for {subtitle}.</p>
                <span className="steprow">
                  <Button
                    variant="primary"
                    disabled={busy}
                    onClick={() =>
                      revealOffer(() => bridge.startDevicePairing(store.id))
                    }
                  >
                    Start
                  </Button>
                  <Button
                    disabled={busy}
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
            <b>On the other Mac</b>
            <p>
              Open FOKS there, choose Add a device or paper key › Enter a
              pairing phrase, and type the phrase. Then finish here: Finish
              writes the new device into the account and reloads the
              authenticated device list.
            </p>
          </li>
        </ol>
      ) : (
        <ol className="pair-steps">
          <li>
            <b>On the other Mac</b>
            <p>
              Open FOKS there, choose Add a device or paper key › Pair another
              Mac, and start a pairing. It shows a phrase once.
            </p>
          </li>
          <li>
            <b>On this Mac</b>
            <p>
              Type that phrase here. Resume acceptance continues an acceptance
              this Mac already began.
            </p>
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
  const [target, setTarget] = useState(store.account);
  const [device, setDevice] = useState('This Mac');
  const [phrase, setPhrase] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <DeviceSheetFrame
      title="Recover on this Mac"
      subtitle="Use your paper key"
      onClose={() => {
        setPhrase('');
        onClose();
      }}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabled={!target || !device || !phrase || busy}
            onClick={() => {
              const once = phrase;
              setPhrase('');
              setBusy(true);
              void enqueueProfileWork(bridge, store.server, () =>
                bridge.recoverOwnerAccount(store.server, target, once, device),
              )
                .then(onDone)
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Recover
          </Button>
        </>
      }
    >
      <p>
        Recovery adds this Mac as an authorized device. If interrupted, you can
        resume recovery from People’s Needs attention list using the same
        phrase.
      </p>
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
  const [alias, setAlias] = useState('work-key');
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
  const slot = (value: string): number | null =>
    /^0x[0-9a-fA-F]{2}$/.test(value)
      ? Number.parseInt(value.slice(2), 16)
      : null;
  const signing = slot(signingSlot);
  const pq = slot(pqSlot);
  const validAttempts =
    Number.isInteger(pinAttempts) &&
    pinAttempts > 0 &&
    pinAttempts <= 255 &&
    Number.isInteger(pukAttempts) &&
    pukAttempts > 0 &&
    pukAttempts <= 255;
  return (
    <DeviceSheetFrame
      title="Create a YubiKey account"
      subtitle={`Create a new account on ${store.server}`}
      onClose={() => {
        clear();
        onClose();
      }}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabled={
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
              const command: YubiCommand = {
                command: 'create_yubi_account',
                args: {
                  profile: store.server,
                  alias: alias.trim(),
                  username: username.trim(),
                  deviceName: shownName.trim(),
                  email: '',
                  invite,
                  cardSerial: card?.serial ?? 0,
                  signingSlot: signing,
                  pqSlot: pq,
                  pin,
                  puk,
                  pinAttempts,
                  pukAttempts,
                },
              };
              clear();
              setBusy(true);
              void bridge
                .runYubi(command)
                .then(onDone)
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Prepare card and create account
          </Button>
        </>
      }
    >
      <p>Before you continue:</p>
      <ol className="sheet-steps">
        <li>
          <b>Credentials are written in a single operation.</b> If setup fails,
          the card's security applet must be reset, erasing existing card data.
        </li>
        <li>
          <b>The card must use default factory settings.</b> Custom-managed
          cards are not supported.
        </li>
        <li>
          <b>Save your unlock code securely.</b> It cannot be recovered if lost.
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
        <Field label="Alias" value={alias} onChange={setAlias} />
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
          <small>Optional server invitation code.</small>
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
      <p className="hint">
        PIN, unlock code and invite go only to the local agent and are cleared
        when submitted.
      </p>
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
  const [targetAlias, setTargetAlias] = useState('new-key');
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
  const valid = Boolean(
    cards.some((card) => card.serial === serial) &&
    targetAlias.trim() &&
    deviceName.trim() &&
    pin &&
    puk,
  );
  return (
    <DeviceSheetFrame
      title="Provision a YubiKey device"
      subtitle={`Add a security key to ${store.account}`}
      onClose={() => {
        clear();
        onClose();
      }}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabled={!valid || busy}
            onClick={() => {
              const command: YubiCommand = {
                command: 'provision_yubi_device',
                args: {
                  accountStoreId: store.id,
                  targetAlias: targetAlias.trim(),
                  deviceName: deviceName.trim(),
                  cardSerial: serial,
                  signingSlot: 0x82,
                  pqSlot: 0x83,
                  pin,
                  puk,
                  pinAttempts: 3,
                  pukAttempts: 3,
                },
              };
              clear();
              setBusy(true);
              void bridge
                .runYubi(command)
                .then(onDone)
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Provision card
          </Button>
        </>
      }
    >
      <p>Enter a new alias and select a connected card.</p>
      {cards.length ? null : (
        <Band label="Connect a YubiKey">
          No security key is currently detected.
        </Band>
      )}
      <Inset>
        <Field
          label="Key alias"
          value={targetAlias}
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
  action,
  alias,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  action: SimpleYubiAction;
  alias: string;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
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
  const submit = (): void => {
    let command: YubiCommand;
    switch (action) {
      case 'sync':
        command = {
          command: 'sync_yubi_account',
          args: { profile: store.server, alias, pin, withFederation: true },
        };
        break;
      case 'pin-status':
        command = {
          command: 'yubi_pin_status',
          args: { profile: store.server, alias },
        };
        break;
      case 'change-pin':
        command = {
          command: 'change_yubi_pin',
          args: { profile: store.server, alias, oldPin: pin, newPin: other },
        };
        break;
      case 'set-passphrase':
      case 'change-passphrase':
        command = {
          command:
            action === 'set-passphrase'
              ? 'set_yubi_passphrase'
              : 'change_yubi_passphrase',
          args: {
            profile: store.server,
            alias,
            pin,
            passphrase: other,
            confirmation,
          },
        };
        break;
      case 'verify-passphrase':
        command = {
          command: 'verify_yubi_passphrase',
          args: { profile: store.server, alias, pin, passphrase: other },
        };
        break;
      case 'unblock':
        command = {
          command: 'unblock_yubi_pin',
          args: { profile: store.server, alias, puk: pin, newPin: other },
        };
        break;
      case 'change-puk':
        command = {
          command: 'change_yubi_puk',
          args: { profile: store.server, alias, oldPuk: pin, newPuk: other },
        };
        break;
      case 'recover-management':
        command = {
          command: 'recover_yubi_management_key',
          args: { accountStoreId: store.id, yubiAlias: alias },
        };
        break;
      case 'recover-subkey':
        command = {
          command: 'recover_yubi_subkey',
          args: { profile: store.server, alias, pin },
        };
        break;
      case 'resume-enrollment':
        command = {
          command: 'resume_yubi_account',
          args: { profile: store.server, alias, pin },
        };
        break;
      case 'resume-rotation':
        command = {
          command: 'resume_yubi_management_key',
          args: { profile: store.server, alias, ...(pin ? { pin } : {}) },
        };
        break;
      case 'rotate':
        command = {
          command: 'rotate_yubi_management_key',
          args: { profile: store.server, alias, pin },
        };
        break;
    }
    setPin('');
    setOther('');
    setConfirmation('');
    setBusy(true);
    void bridge
      .runYubi(command)
      .then(onDone)
      .catch(onError)
      .finally(() => setBusy(false));
  };
  return (
    <DeviceSheetFrame
      title={YUBI_ACTION_LABELS[action]}
      subtitle={alias || 'Choose an enrolled key alias'}
      onClose={() => {
        setPin('');
        setOther('');
        setConfirmation('');
        onClose();
      }}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" disabled={!valid || busy} onClick={submit}>
            Continue
          </Button>
        </>
      }
    >
      <p>
        The values below go only to the local agent and are cleared when
        submitted.
      </p>
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
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <DeviceSheetFrame
      title={`Revoke ${alias}?`}
      subtitle="Disconnects this key and updates account security"
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
            disabled={confirmation !== alias || busy}
            onClick={() => {
              setBusy(true);
              void bridge
                .runYubi({
                  command: 'revoke_yubi_device',
                  args: {
                    accountStoreId: store.id,
                    yubiAlias: alias,
                    confirmation,
                  },
                })
                .then((result) => {
                  if (
                    result.alias !== alias ||
                    result.removedLocalCredential !== true
                  )
                    throw new Error(
                      'revoke_yubi_device returned a different enrollment.',
                    );
                  return onDone();
                })
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Revoke {alias}
          </Button>
        </>
      }
    >
      <p>
        This YubiKey will immediately lose access to your account. Any data
        previously cached on devices using this key will remain until cleared.
        Enter the key alias to confirm revocation.
      </p>
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
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <DeviceSheetFrame
      title={`Revoke ${backup.backupAlias}?`}
      subtitle={`${store.account} · ${backup.backupId}`}
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
            disabled={confirmation !== backup.backupAlias || busy}
            onClick={() => {
              setBusy(true);
              void bridge
                .revokeOwnerBackup(store.id, backup, confirmation)
                .then((revoked) => {
                  if (
                    revoked.backupAlias !== backup.backupAlias ||
                    revoked.backupId !== backup.backupId
                  )
                    throw new Error(
                      'revoke_owner_backup returned a different enrollment.',
                    );
                  return onDone();
                })
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Revoke paper key
          </Button>
        </>
      }
    >
      <p>
        This paper key will immediately lose future recovery access. Revocation
        also rotates every account key it could read. Enter the paper key name
        to confirm.
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
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  const expected = device.name ?? device.id;
  return (
    <DeviceSheetFrame
      title={`Remove ${device.name ?? 'device'}?`}
      subtitle={`${store.account} · ${device.id}`}
      onClose={onClose}
      danger
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="danger"
            disabled={confirmation !== expected || busy}
            onClick={() => {
              setBusy(true);
              void bridge
                .removeAccountDevice(store.id, device.id)
                .then((removed) => {
                  if (removed.deviceId !== device.id)
                    throw new Error(
                      'remove_account_device returned a different device.',
                    );
                  return onDone();
                })
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Remove device
          </Button>
        </>
      }
    >
      <p>
        This device loses future access to the account. Any data previously
        downloaded to this device will remain until removed; change any
        sensitive secrets if the device is not under your control.
      </p>
      <p className="fn">
        You can only remove other devices here. A security key is revoked under
        Security keys, and this Mac is removed by Reset this Mac in Settings.
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
  subtitle,
  initialMode,
  onClose,
  onDone,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  subtitle: string;
  initialMode: PassphraseMode;
  onClose: () => void;
  onDone: (message: string) => void;
  onError: (error: unknown) => void;
}): ReactNode {
  const [mode, setMode] = useState<PassphraseMode>(initialMode);
  const [passphrase, setPassphrase] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  const submit = (): void => {
    const secret = passphrase;
    const repeated = confirmation;
    setBusy(true);
    const task =
      mode === 'set'
        ? bridge.setAccountPassphrase(store.id, secret, repeated)
        : mode === 'change'
          ? bridge.changeAccountPassphrase(store.id, secret, repeated)
          : bridge.verifyAccountPassphrase(store.id, secret);
    void task
      .then((report) => {
        setPassphrase('');
        setConfirmation('');
        onDone(
          mode === 'verify'
            ? `Passphrase verified (generation ${report.generation})`
            : 'Passphrase updated successfully.',
        );
      })
      .catch(onError)
      .finally(() => setBusy(false));
  };
  return (
    <DeviceSheetFrame
      title="Account passphrase"
      subtitle={subtitle}
      onClose={() => {
        if (busy) return;
        setPassphrase('');
        setConfirmation('');
        onClose();
      }}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={
              !passphrase ||
              (mode !== 'verify' && passphrase !== confirmation) ||
              busy
            }
            onClick={submit}
          >
            {mode === 'set'
              ? 'Set passphrase'
              : mode === 'change'
                ? 'Change passphrase'
                : 'Verify passphrase'}
          </Button>
        </>
      }
    >
      <SegmentedControl
        label="Passphrase action"
        value={mode}
        onChange={(next) => {
          setConfirmation('');
          setMode(next);
        }}
        items={[
          { id: 'set', label: 'Set' },
          { id: 'change', label: 'Change' },
          { id: 'verify', label: 'Verify' },
        ]}
      />
      <Inset>
        <Field
          label="Passphrase"
          value={passphrase}
          onChange={setPassphrase}
          type="password"
        />
        {mode === 'verify' ? null : (
          <Field
            label="Confirm"
            value={confirmation}
            onChange={setConfirmation}
            type="password"
          />
        )}
      </Inset>
      {mode === 'verify' ? (
        <p className="hint">
          Verify tests the passphrase with the server login challenge.
        </p>
      ) : null}
    </DeviceSheetFrame>
  );
}
