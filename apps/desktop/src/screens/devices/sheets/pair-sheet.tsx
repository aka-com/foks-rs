/**
 * Pairs another device with this account by showing a pairing phrase, or this
 * device with another account by typing the phrase that device shows.
 */
import { useTabSheetState } from '../../../navigation-guard';
import { queuedDeviceWork } from '../operation-controller';
import { useDeviceOperation } from '../use-device-operation';
import { acceptPairing, finishPairing } from '../pairing-workflow';
import { useWorkflowAccess } from '../../../workflow-context';
import { useState } from 'react';
import type { ReactNode } from 'react';
import { useConcealOnInactive } from '../../../use-conceal-on-inactive';
import { useSheetGuard } from '../../../navigation-guard';
import type { PairingOffer } from '../../../bridge';
import {
  Band,
  Button,
  CopyBox,
  Field,
  Inset,
  SheetDialog,
} from '../../../components';
import type { AccountStore } from '../../../model';
import { deviceAlertRegistry } from '../../../data/device-alert';
import { useShellServices } from '../../../shell-services';

export function PairSheet({
  store,
  initialMode,
  onCopy,
  onBack,
  onClose,
  onDone,
  onError,
}: {
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
  const { bridge } = useShellServices();
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
    <SheetDialog
      width="wide"
      icon="settings"
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
    </SheetDialog>
  );
}
