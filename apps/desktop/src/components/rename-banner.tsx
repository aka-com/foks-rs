/** Pending rename actions and security-key authentication in Settings. */

import { useState } from 'react';
import type { ReactNode } from 'react';
import type { RenameAction, RenameProgress } from '../rename-contract';
import type { RenameStatus } from '../rename-status';
import { useWorkflowAccess } from '../workflow-context';
import { useSheetGuard } from '../navigation-guard';
import { normalizeCommandError } from '../bridge';
import { Band } from './notice';
import { Button } from './button';
import { Inset, InsetRow } from './inset';
import { PanelSheet } from './panel-sheet';
import type { PanelPresentation } from './panel-sheet';

/** Status text and available actions. */
function describe(
  row: RenameProgress,
  target: string | undefined,
  needsCheck: boolean,
): {
  severity: 'info' | 'warn';
  label: string;
  detail: string;
  /** Next operation action. */
  step: 'confirm' | 'check';
} {
  const what = target ? `Rename to ${target}` : 'A username change';
  if (needsCheck)
    return {
      severity: 'warn',
      label: `${what}: status unknown.`,
      detail: 'Check status before confirming or discarding this change.',
      step: 'check',
    };
  switch (row.state) {
    case 'prepared':
      return {
        severity: 'info',
        label: `${what} is waiting for your confirmation.`,
        detail: 'Nothing has changed on the server yet.',
        step: 'confirm',
      };
    case 'submission-unknown':
      return {
        severity: 'warn',
        label: `${what} may or may not have been applied.`,
        detail: 'Check status before starting another username change.',
        step: 'check',
      };
    case 'remote-verified':
      return {
        severity: 'info',
        label: `${what} was accepted by the server.`,
        detail: 'Check its status to finish updating this device.',
        step: 'check',
      };
    default:
      return {
        severity: 'info',
        label: `${what} is being sent to the server.`,
        detail: 'Check status to verify the result.',
        step: 'check',
      };
  }
}

export function RenameBanner({
  status,
  profile,
  account,
  onUnlock,
  onComplete,
}: {
  status: RenameStatus;
  profile: string;
  account: string;
  /** Open the PIN dialog for the requested action. */
  onUnlock: (action: RenameAction) => void;
  /** Refresh account data after completion. */
  onComplete: () => void | Promise<void>;
}): ReactNode {
  const access = useWorkflowAccess();
  const eligibility = access.props('account-rename', { profile, account });
  // Recheck access when the action starts.
  const [refused, setRefused] = useState<string | null>(null);
  const run = async (action: RenameAction) => {
    setRefused(null);
    try {
      access.require('account-rename', { profile, account });
    } catch (e) {
      setRefused(normalizeCommandError(e).message);
      return;
    }
    const row = await status.act(action);
    if (!row) return;
    if (row.hardware_required && action.action !== 'cancel') {
      onUnlock(action);
      return;
    }
    if (row.state === 'complete') await onComplete();
  };
  const error = refused ?? status.error;
  const pending = status.pending;
  if (!pending) {
    if (status.error === null) return null;
    return (
      <Band
        live
        severity="warn"
        label="Pending username changes unavailable"
        action={
          <Button size="sm" onClick={() => void status.refresh()}>
            Retry
          </Button>
        }
      >
        {status.error}
      </Band>
    );
  }
  const copy = describe(pending, status.target, status.needsCheck);
  const disabled = status.busy || eligibility.disabled;
  const step: RenameAction = {
    action: copy.step === 'confirm' ? 'attempt' : 'status',
    operation_id: pending.operation_id,
    pin: null,
  };
  return (
    <Band
      live
      severity={copy.severity}
      label={copy.label}
      title={eligibility.title ?? `Operation ${pending.operation_id}`}
      action={
        <>
          <Button
            size="sm"
            variant="primary"
            disabled={disabled}
            title={eligibility.title}
            onClick={() => void run(step)}
          >
            {copy.step === 'confirm' ? 'Confirm' : 'Check status'}
          </Button>
          {copy.step === 'confirm' ? (
            <Button
              size="sm"
              disabled={disabled}
              title={eligibility.title}
              onClick={() =>
                void run({
                  action: 'cancel',
                  operation_id: pending.operation_id,
                })
              }
            >
              Discard
            </Button>
          ) : null}
        </>
      }
    >
      {copy.detail}
      {error === null ? null : (
        <>
          {' '}
          <span role="alert">{error}</span>
        </>
      )}
    </Band>
  );
}

/** Retry an action with a PIN; clear the input before submitting. */
export function RenameUnlockSheet({
  profile,
  account,
  action,
  status,
  presentation,
  onDone,
}: {
  profile: string;
  account: string;
  action: RenameAction;
  status: RenameStatus;
  presentation: PanelPresentation;
  /** Close the dialog and refresh account data if complete. */
  onDone: (completed: boolean) => void | Promise<void>;
}): ReactNode {
  const access = useWorkflowAccess();
  const eligibility = access.props('account-rename', { profile, account });
  const [refused, setRefused] = useState<string | null>(null);
  const [pin, setPin] = useState('');
  useSheetGuard(
    status.busy
      ? {
          verdict: 'refuse',
          reason: 'Wait for the rename operation to finish.',
        }
      : null,
  );
  const submit = async () => {
    if (!pin || status.busy) return;
    setRefused(null);
    try {
      access.require('account-rename', { profile, account });
    } catch (error) {
      setRefused(normalizeCommandError(error).message);
      return;
    }
    const supplied = pin;
    setPin('');
    const next: RenameAction =
      action.action === 'cancel'
        ? action
        : action.action === 'prepare'
          ? { ...action, pin: supplied }
          : {
              action: status.needsCheck ? 'status' : action.action,
              operation_id: action.operation_id,
              pin: supplied,
            };
    const row = await status.act(next);
    if (!row) return;
    if (row.hardware_required) return;
    await onDone(row.state === 'complete');
  };
  return (
    <PanelSheet
      presentation={presentation}
      busy={status.busy}
      footer={
        <>
          <Button disabled={status.busy} onClick={presentation.onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={status.busy || eligibility.disabled || !pin}
            title={
              eligibility.title ??
              (!pin ? 'Enter the security key PIN to continue.' : undefined)
            }
            onClick={() => void submit()}
          >
            {action.action === 'attempt' && !status.needsCheck
              ? 'Confirm rename'
              : 'Check status'}
          </Button>
        </>
      }
    >
      <p>Enter the enrolled security key’s PIN to authorize this action.</p>
      <Inset className="form">
        <InsetRow label="Security key PIN">
          <input
            type="password"
            autoComplete="off"
            value={pin}
            maxLength={32}
            disabled={status.busy || eligibility.disabled}
            onChange={(e) => setPin(e.target.value)}
          />
        </InsetRow>
      </Inset>
      {(refused ?? status.error) && (
        <p role="alert" className="crit">
          {refused ?? status.error}
        </p>
      )}
    </PanelSheet>
  );
}
