/** Sets or changes the account passphrase from Settings › Account. */
import {
  accountPassphrase,
  accountPassphraseStatus,
} from '../credential-workflow';
import { useDeviceOperation } from '../use-device-operation';
import { useWorkflowAccess } from '../../../workflow-context';
import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { useSheetGuard } from '../../../navigation-guard';
import type { PassphraseStatus } from '../../../bridge';
import { normalizeCommandError } from '../../../bridge';
import { Band, Button, Field, Inset, SheetDialog } from '../../../components';
import type { AccountStore } from '../../../model';
import { useShellServices } from '../../../shell-services';
import type { PassphraseMode } from '../credential-actions';

export function PassphraseSheet({
  store,
  onClose,
  onDone,
  onError,
}: {
  store: AccountStore;
  onClose: () => void;
  onDone: (message: string) => void | Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const { bridge } = useShellServices();
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
    <SheetDialog
      width="wide"
      icon="settings"
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
        <Inset className="passphrase-loading">
          <div
            role="status"
            aria-label="Reading this account's passphrase state"
          >
            <span className="spin" aria-hidden="true" />
          </div>
        </Inset>
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
                placeholder="Enter current passphrase"
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
              placeholder={
                mode === 'set' ? 'Enter a passphrase' : 'Enter new passphrase'
              }
              value={passphrase}
              onChange={setPassphrase}
              type="password"
            />
            <Field
              label="Confirm"
              placeholder="Repeat passphrase"
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
    </SheetDialog>
  );
}
