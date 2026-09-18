import { useEffect, useRef, useState } from 'react';
import { useWorkflowAccess } from '../workflow-context';
import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import type { RenameAction, RenameProgress } from '../rename-contract';
import { normalizeCommandError } from '../bridge';
import { Button, Inset, InsetRow, PanelSheet } from './index';
import type { PanelPresentation } from './index';

export function RenamePanel({
  bridge,
  profile,
  account,
  presentation,
  onComplete,
}: {
  bridge: Bridge;
  profile: string;
  account: string;
  presentation: PanelPresentation;
  onComplete: () => void | Promise<void>;
}): ReactNode {
  const access = useWorkflowAccess();
  const target = { profile, account };
  const eligibility = access.props('account-rename', target);
  const [name, setName] = useState('');
  const [pin, setPin] = useState('');
  const [rows, setRows] = useState<RenameProgress[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const owner = `${profile}/${account}`;
  const active = useRef(owner);
  useEffect(() => {
    active.current = owner;
    setRows([]);
    setPin('');
    setName('');
    setBusy(false);
    setError(null);
    return () => {
      active.current = '';
    };
  }, [owner]);
  const run = async (action: RenameAction | null) => {
    setBusy(true);
    setError(null);
    const suppliedPin = pin || null;
    setPin('');
    try {
      const needsHardware =
        action &&
        'operation_id' in action &&
        'pin' in action &&
        rows.some(
          (row) =>
            row.operation_id === action.operation_id && row.hardware_required,
        );
      access.require('account-rename', {
        ...target,
        ...(needsHardware && !suppliedPin
          ? { hardware: 'needed' as const }
          : {}),
      });
      const result = await bridge.renameAccount(
        profile,
        account,
        action && 'pin' in action ? { ...action, pin: suppliedPin } : action,
      );
      if (active.current !== owner) return;
      if (
        result.some(
          (p) =>
            p.account_alias !== account ||
            (action &&
              'operation_id' in action &&
              p.operation_id !== action.operation_id),
        )
      )
        throw new Error('Rename belongs to a different account.');
      setRows(result);
      if (result.some((p) => p.state === 'complete')) await onComplete();
    } catch (e) {
      if (active.current === owner) setError(normalizeCommandError(e).message);
    } finally {
      if (active.current === owner) setBusy(false);
    }
  };
  return (
    <PanelSheet
      presentation={presentation}
      busy={busy}
      footer={
        <>
          <Button disabled={busy} onClick={presentation.onClose}>
            Cancel
          </Button>
          <Button
            disabled={busy || eligibility.disabled}
            title={eligibility.title}
            onClick={() => void run(null)}
          >
            Show pending changes
          </Button>
          <Button
            variant="primary"
            title={eligibility.title}
            disabled={busy || eligibility.disabled || !name}
            onClick={() =>
              void run({ action: 'prepare', username: name, pin: null })
            }
          >
            Change username
          </Button>
        </>
      }
    >
      <p>Changes the username on the server.</p>
      <Inset className="form">
        <InsetRow label="Username">
          <input
            value={name}
            maxLength={256}
            disabled={busy || eligibility.disabled}
            title={eligibility.title}
            onChange={(e) => setName(e.target.value)}
          />
        </InsetRow>
        <InsetRow label="Security key PIN (enrolled keys only)">
          <input
            type="password"
            autoComplete="off"
            value={pin}
            maxLength={32}
            disabled={busy || eligibility.disabled}
            title={eligibility.title}
            onChange={(e) => setPin(e.target.value)}
          />
        </InsetRow>
      </Inset>
      {error && (
        <p role="alert" className="crit">
          {error}
        </p>
      )}
      {rows.map((p) => (
        <div key={p.operation_id} className="op">
          <p role="status">
            {p.target ? `Requested: ${p.target}. ` : ''}
            {p.current_username ? `Current: ${p.current_username}. ` : ''}
            {p.state}
          </p>
          <p>
            Operation: <code>{p.operation_id}</code>
          </p>
          {p.hardware_required && (
            <p>Enter this account’s security key PIN to continue.</p>
          )}
          {p.state === 'submission-unknown' && (
            <p>
              The server may have processed the rename. Check the operation
              status before retrying to prevent conflicting updates.
            </p>
          )}
          <div className="btns">
            {p.state === 'prepared' && (
              <>
                <Button
                  variant="primary"
                  disabled={
                    busy ||
                    eligibility.disabled ||
                    (p.hardware_required && !pin)
                  }
                  title={
                    eligibility.title ??
                    (p.hardware_required && !pin
                      ? 'Enter the security key PIN to continue.'
                      : undefined)
                  }
                  onClick={() =>
                    void run({
                      action: 'attempt',
                      operation_id: p.operation_id,
                      pin: null,
                    })
                  }
                >
                  Confirm
                </Button>
                <Button
                  disabled={busy || eligibility.disabled}
                  title={eligibility.title}
                  onClick={() =>
                    void run({ action: 'cancel', operation_id: p.operation_id })
                  }
                >
                  Cancel
                </Button>
              </>
            )}
            {!['complete', 'rejected'].includes(p.state) && (
              <Button
                disabled={
                  busy || eligibility.disabled || (p.hardware_required && !pin)
                }
                title={
                  eligibility.title ??
                  (p.hardware_required && !pin
                    ? 'Enter the security key PIN to continue.'
                    : undefined)
                }
                onClick={() =>
                  void run({
                    action: 'status',
                    operation_id: p.operation_id,
                    pin: null,
                  })
                }
              >
                Check status
              </Button>
            )}
          </div>
        </div>
      ))}
    </PanelSheet>
  );
}
