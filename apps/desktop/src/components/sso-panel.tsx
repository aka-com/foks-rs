import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import { normalizeCommandError } from '../bridge';
import type { SsoAction, SsoProgress } from '../sso-contract';
import { Button } from './index';
const messages: Record<SsoProgress['state'], string> = {
  'device-only': 'This host has not enabled organization sign-in.',
  'link-needed':
    'Link your existing account to your organization. Device access remains available during migration.',
  'locked-out':
    'Organization sign-in is enforced. Link this account with its owner device to restore access.',
  linked: 'This account is linked to your organization.',
  'not-eligible': 'This account is not eligible for existing-account linkage.',
  prepared:
    'Authentication was interrupted before the browser was opened. Cancel and begin again.',
  waiting: 'Complete sign-in in your browser, then check for completion.',
  ready:
    'Browser sign-in is verified. Continue to finish this account operation.',
  submitting:
    'The signed operation is being reconciled. Resume this flow to check its outcome.',
  complete: 'The signed account operation was accepted.',
  cancelled: 'This browser flow was cancelled.',
  expired: 'This browser flow expired. Begin again.',
  'submission-unknown':
    'The signed request may have been accepted. Its outcome is still unknown. A new login does not retry the interrupted operation.',
  rejected: 'Authentication could not be verified.',
  denied: 'Your identity provider declined sign-in.',
  'provider-unavailable':
    'Your identity provider is unavailable. Check again when it recovers.',
  'reauthentication-required': 'Service access is unavailable. Sign in again.',
  'service-unavailable':
    'The signed operation was accepted, but service access could not be verified. Check your connection and refresh status.',
  'hardware-verification-required':
    'Unlock the enrolled security key to verify service access.',
};
interface Props {
  bridge: Bridge;
  profile: string;
  account: string;
  login: boolean;
  deviceName?: string;
  invite?: string;
  disabled?: boolean;
  onComplete: () => void | Promise<void>;
  initialOperationId?: string;
  initialHardware?: boolean;
  onProgress?: (progress: SsoProgress, hardware: boolean) => void;
  executeSignup?: (
    action: SsoAction,
    operation: () => Promise<SsoProgress>,
  ) => Promise<SsoProgress>;
  resumeOnly?: boolean;
}
export function SsoPanel({
  bridge,
  profile,
  account,
  login,
  deviceName = '',
  invite = '',
  disabled = false,
  onComplete,
  initialOperationId,
  initialHardware = false,
  onProgress,
  executeSignup,
  resumeOnly = false,
}: Props) {
  const [progress, setProgress] = useState<SsoProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pin, setPin] = useState('');
  const [hardware, setHardware] = useState(initialHardware);
  const [serial, setSerial] = useState('');
  const owner = `${profile}/${account}/${login}`;
  const active = useRef(owner);
  const complete = useRef(onComplete);
  useEffect(() => {
    complete.current = onComplete;
  }, [onComplete]);
  useEffect(() => {
    active.current = owner;
    return () => {
      active.current = '';
    };
  }, [owner]);
  useEffect(() => {
    if (!initialOperationId) return;
    let alive = true;
    setBusy(true);
    void bridge
      .sso(profile, account, {
        action: 'status',
        operation_id: initialOperationId,
      })
      .then((p) => {
        if (!alive) return;
        if (
          p.accountAlias !== account ||
          p.operationId !== initialOperationId ||
          (!login && p.purpose !== 'signup')
        )
          throw new Error('Authentication belongs to a different account.');
        setProgress(p);
        if (
          !login &&
          (p.state === 'complete' || p.state === 'service-unavailable')
        )
          return complete.current();
      })
      .catch((e) => {
        if (alive) setError(normalizeCommandError(e).message);
      })
      .finally(() => {
        if (alive) setBusy(false);
      });
    return () => {
      alive = false;
    };
  }, [bridge, profile, account, initialOperationId, login]);
  const run = async (action: SsoAction) => {
    setBusy(true);
    setError(null);
    try {
      const submit =
        action.action === 'finish-signup' ||
        action.action === 'finish-yubi-signup';
      const p = await (submit && executeSignup
        ? executeSignup(action, () => bridge.sso(profile, account, action))
        : bridge.sso(profile, account, action));
      if (active.current !== owner) return;
      if (
        p.accountAlias !== account ||
        (p.purpose !== 'signup') !== login ||
        (action.action !== 'begin' &&
          action.action !== 'begin-yubi-signup' &&
          action.action !== 'account-status' &&
          p.operationId !== action.operation_id)
      )
        throw new Error('Authentication belongs to a different account.');
      setProgress(p);
      onProgress?.(p, hardware);
      if (p.state === 'complete' && p.serviceAccess) {
        setPin('');
        await onComplete();
      }
    } catch (e) {
      if (active.current === owner) setError(normalizeCommandError(e).message);
    } finally {
      if (active.current === owner) setBusy(false);
    }
  };
  const blocked =
    busy || disabled || !account || (!login && !deviceName.trim());
  const finishable =
    progress !== null &&
    !progress.accountStatus &&
    [
      'ready',
      'submitting',
      'submission-unknown',
      'hardware-verification-required',
    ].includes(progress.state);
  const pollable =
    progress !== null &&
    !progress.accountStatus &&
    ['waiting', 'provider-unavailable'].includes(progress.state);
  const cancellable =
    !resumeOnly &&
    progress !== null &&
    !progress.accountStatus &&
    ['prepared', 'waiting', 'ready', 'denied', 'provider-unavailable'].includes(
      progress.state,
    );
  const beginDisabled =
    blocked ||
    resumeOnly ||
    (!login && hardware && (!/^[1-9][0-9]{0,9}$/.test(serial) || !pin));
  // Priority order for the primary action: completion, browser authentication,
  // then initiation.
  const primary: 'finish' | 'browser' | 'begin' = finishable
    ? 'finish'
    : progress?.browserAvailable
      ? 'browser'
      : 'begin';
  const pinField = (label: string): ReactNode => (
    <div className="local-field-card">
      <label className="local-field-row">
        <span>{label}</span>
        <input
          type="password"
          autoComplete="off"
          value={pin}
          onChange={(e) => setPin(e.target.value)}
        />
      </label>
    </div>
  );
  return (
    <section className="pcard" aria-label="Organization sign-in">
      <h3>
        {login ? 'Organization sign-in' : 'Sign up with your organization'}
      </h3>
      <p>
        {login
          ? 'Restore access using your identity provider and this device’s account key.'
          : 'Your identity provider supplies your username and email. Choose a local account label and device name above.'}
      </p>
      {progress && (
        <p role="status">
          {progress.serviceAccess && !progress.accountStatus
            ? 'Account authentication and service access verified.'
            : messages[progress.state]}
        </p>
      )}
      {error && (
        <p role="alert" className="crit">
          {error}
        </p>
      )}
      {login && pinField('Security key PIN (for an enrolled key)')}
      {!login && (
        <label className="check-row">
          <input
            type="checkbox"
            checked={hardware}
            disabled={busy || Boolean(progress)}
            onChange={(e) => setHardware(e.target.checked)}
          />
          Create keys on a security key
        </label>
      )}
      {!login && hardware && (
        <>
          <div className="local-field-card">
            <label className="local-field-row">
              <span>Card serial</span>
              <input
                value={serial}
                inputMode="numeric"
                onChange={(e) => setSerial(e.target.value)}
              />
            </label>
            <label className="local-field-row">
              <span>Security key PIN</span>
              <input
                type="password"
                autoComplete="off"
                value={pin}
                onChange={(e) => setPin(e.target.value)}
              />
            </label>
          </div>
          <p>
            Uses signing slot 130 and encryption slot 131. Existing keys in
            these slots are checked before preparation.
          </p>
        </>
      )}
      {finishable &&
        login &&
        pinField('Security key PIN (only for an enrolled key)')}
      <div className="btns">
        {login && (
          <Button
            disabled={blocked}
            onClick={() => {
              const action: SsoAction = {
                action: 'account-status',
                pin: pin || null,
              };
              setPin('');
              void run(action);
            }}
          >
            Check account linkage
          </Button>
        )}
        {!resumeOnly && (
          <Button
            variant={primary === 'begin' ? 'primary' : 'plain'}
            disabled={beginDisabled}
            onClick={() => {
              const action: SsoAction =
                !login && hardware
                  ? {
                      action: 'begin-yubi-signup',
                      card_serial: Number(serial),
                      signing_slot: 130,
                      pq_slot: 131,
                      pin,
                      device_name: deviceName,
                      invite,
                    }
                  : {
                      action: 'begin',
                      purpose: !login
                        ? 'signup'
                        : (progress?.purpose ?? 'reauthenticate'),
                      pin: login ? pin || null : null,
                    };
              setPin('');
              void run(action);
            }}
          >
            {progress?.purpose === 'link-existing'
              ? 'Link existing account'
              : progress
                ? 'Begin or resume sign-in'
                : 'Continue with organization'}
          </Button>
        )}
        {progress?.browserAvailable && (
          <Button
            variant={primary === 'browser' ? 'primary' : 'plain'}
            disabled={blocked}
            onClick={() =>
              void bridge
                .openSsoBrowser(profile, account, progress.operationId!)
                .catch((e) => setError(normalizeCommandError(e).message))
            }
          >
            Open sign-in browser
          </Button>
        )}
        {pollable && (
          <Button
            disabled={blocked}
            onClick={() =>
              void run({ action: 'poll', operation_id: progress.operationId! })
            }
          >
            Check sign-in
          </Button>
        )}
        {finishable && (
          <Button
            variant="primary"
            disabled={blocked}
            onClick={() => {
              const action: SsoAction =
                !login && hardware
                  ? {
                      action: 'finish-yubi-signup',
                      operation_id: progress.operationId!,
                      pin,
                    }
                  : login
                    ? {
                        action: 'finish-login',
                        operation_id: progress.operationId!,
                        pin: pin || null,
                      }
                    : {
                        action: 'finish-signup',
                        operation_id: progress.operationId!,
                        device_name: deviceName,
                        invite,
                        passphrase: null,
                      };
              setPin('');
              void run(action);
            }}
          >
            Finish sign-in
          </Button>
        )}
        {cancellable && (
          <Button
            disabled={blocked}
            onClick={() =>
              void run({
                action: 'cancel',
                operation_id: progress.operationId!,
              })
            }
          >
            Cancel sign-in
          </Button>
        )}
        {progress && !progress.accountStatus && (
          <Button
            disabled={blocked}
            onClick={() =>
              void run({
                action: 'status',
                operation_id: progress.operationId!,
              })
            }
          >
            Refresh status
          </Button>
        )}
      </div>
    </section>
  );
}
