/** Shared state and bridge operations for SSO sign-in and sign-up. */

import { useEffect, useRef, useState } from 'react';
import type { Bridge } from '../../bridge';
import { normalizeCommandError } from '../../bridge';
import type { SsoAction, SsoProgress } from '../../sso-contract';
import { useWorkflowAccess } from '../../workflow-context';

export interface SsoFlowOptions {
  bridge: Bridge;
  profile: string;
  account: string;
  /** True for an existing account signing in; false for a sign-up. */
  login: boolean;
  deviceName?: string;
  invite?: string;
  disabled?: boolean;
  onComplete: () => void | Promise<void>;
  /** Saved operation to load instead of starting a new operation. */
  initialOperationId?: string;
  initialHardware?: boolean;
  onProgress?: (progress: SsoProgress, hardware: boolean) => void;
  /** Optional wrapper for the final sign-up request. */
  executeSignup?: (
    action: SsoAction,
    operation: () => Promise<SsoProgress>,
  ) => Promise<SsoProgress>;
  /** Disable starting and canceling operations. */
  resumeOnly?: boolean;
}

/** UI phase derived from the current SSO progress state. */
export type SsoPhase = 'start' | 'browser' | 'verified' | 'done';

const BROWSER_STATES: ReadonlySet<SsoProgress['state']> = new Set([
  'prepared',
  'waiting',
  'provider-unavailable',
]);
const VERIFIED_STATES: ReadonlySet<SsoProgress['state']> = new Set([
  'ready',
  'submitting',
  'submission-unknown',
  'hardware-verification-required',
]);

export function ssoPhase(progress: SsoProgress | null): SsoPhase {
  if (!progress || progress.accountStatus) return 'start';
  if (BROWSER_STATES.has(progress.state)) return 'browser';
  if (VERIFIED_STATES.has(progress.state)) return 'verified';
  if (progress.state === 'complete' || progress.state === 'service-unavailable')
    return 'done';
  // Terminal failure and cancellation states return to the initial phase.
  return 'start';
}

export function useSsoFlow({
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
}: SsoFlowOptions) {
  const access = useWorkflowAccess();
  const authorizedSetup = !login && executeSignup !== undefined;
  const [hardware, setHardware] = useState(initialHardware);
  const workflow = login
    ? 'sso-login'
    : hardware
      ? 'sso-yubi-signup'
      : 'sso-signup';
  const target = { profile, account };
  const eligibility = authorizedSetup
    ? { disabled: false, title: undefined }
    : access.props(workflow, target);
  const preflight = useRef(() => {});
  preflight.current = () => {
    if (!authorizedSetup) access.require(workflow, target);
  };
  const [progress, setProgress] = useState<SsoProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pin, setPin] = useState('');
  const [serial, setSerial] = useState('');
  // Tracks whether this component opened the browser for the current
  // operation so the UI can advance from Open to Check.
  const [opened, setOpened] = useState(false);
  const owner = `${profile}/${account}/${login}`;
  const active = useRef(owner);
  const complete = useRef(onComplete);
  const reportProgress = useRef(onProgress);
  reportProgress.current = onProgress;
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
    void Promise.resolve()
      .then(() => {
        preflight.current();
        return bridge.sso(profile, account, {
          action: 'status',
          operation_id: initialOperationId,
        });
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
        reportProgress.current?.(p, initialHardware);
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
  }, [bridge, profile, account, initialOperationId, login, initialHardware]);

  const run = async (action: SsoAction): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      preflight.current();
      if (
        login &&
        progress?.state === 'hardware-verification-required' &&
        action.action === 'finish-login' &&
        !action.pin
      )
        access.require('sso-login', { ...target, hardware: 'needed' });
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
      if (
        action.action === 'begin' ||
        action.action === 'begin-yubi-signup' ||
        action.action === 'cancel'
      )
        setOpened(false);
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
    busy ||
    disabled ||
    eligibility.disabled ||
    !account ||
    (!login && !deviceName.trim());
  const operation = progress && !progress.accountStatus ? progress : null;
  const finishable =
    operation !== null &&
    [
      'ready',
      'submitting',
      'submission-unknown',
      'hardware-verification-required',
    ].includes(operation.state);
  const pollable =
    operation !== null &&
    ['waiting', 'provider-unavailable'].includes(operation.state);
  const cancellable =
    !resumeOnly &&
    operation !== null &&
    ['prepared', 'waiting', 'ready', 'denied', 'provider-unavailable'].includes(
      operation.state,
    );
  const serialValid = /^[1-9][0-9]{0,9}$/.test(serial);
  const beginDisabled =
    blocked || resumeOnly || (!login && hardware && (!serialValid || !pin));
  const hardwareRequired = progress?.state === 'hardware-verification-required';
  const browserAvailable = Boolean(operation?.browserAvailable);

  const begin = (): void => {
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
  };
  const finish = (): void => {
    if (!operation?.operationId) return;
    const action: SsoAction =
      !login && hardware
        ? {
            action: 'finish-yubi-signup',
            operation_id: operation.operationId,
            pin,
          }
        : login
          ? {
              action: 'finish-login',
              operation_id: operation.operationId,
              pin: pin || null,
            }
          : {
              action: 'finish-signup',
              operation_id: operation.operationId,
              device_name: deviceName,
              invite,
              passphrase: null,
            };
    setPin('');
    void run(action);
  };
  const withOperation = (action: 'poll' | 'cancel' | 'status') => (): void => {
    if (!operation?.operationId) return;
    void run({ action, operation_id: operation.operationId });
  };
  const checkLinkage = (): void => {
    const action: SsoAction = { action: 'account-status', pin: pin || null };
    setPin('');
    void run(action);
  };
  const openBrowser = (): void => {
    if (!operation?.operationId) return;
    const id = operation.operationId;
    void Promise.resolve()
      .then(() => {
        preflight.current();
        return bridge.openSsoBrowser(profile, account, id);
      })
      .then(() => {
        if (active.current === owner) setOpened(true);
      })
      .catch((e) => {
        if (active.current === owner)
          setError(normalizeCommandError(e).message);
      });
  };
  const chooseHardware = (next: boolean): void => {
    setHardware(next);
    if (!next) setPin('');
  };

  return {
    progress,
    phase: ssoPhase(progress),
    busy,
    error,
    eligibility,
    blocked,
    hardware,
    chooseHardware,
    hardwareRequired,
    pin,
    setPin,
    serial,
    setSerial,
    serialValid,
    opened,
    browserAvailable,
    finishable,
    pollable,
    cancellable,
    beginDisabled,
    begin,
    finish,
    poll: withOperation('poll'),
    cancel: withOperation('cancel'),
    status: withOperation('status'),
    checkLinkage,
    openBrowser,
  };
}

export type SsoFlow = ReturnType<typeof useSsoFlow>;
