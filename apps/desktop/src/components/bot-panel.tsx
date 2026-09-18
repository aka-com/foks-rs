import { useEffect, useRef, useState } from 'react';
import { useWorkflowAccess } from '../workflow-context';
import type { WorkflowOperation } from '../model/workflow-availability';
import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import { normalizeCommandError } from '../bridge';
import type { BotAction, BotEnrollment } from '../bot-contract';
import { Button, Inset, InsetRow, PanelSheet, SegmentedControl } from './index';
import type { PanelPresentation } from './index';

type BotPane = 'enroll' | 'load' | 'revoke';

export function botWorkflow(action: BotAction['action']): WorkflowOperation {
  switch (action) {
    case 'list': return 'bot-list';
    case 'unload': return 'bot-unload';
    case 'load-file': return 'bot-load';
    case 'revoke': return 'bot-revoke';
    default: return 'bot-enroll';
  }
}

export function BotPanel({
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
  const eligibility = (action: BotAction['action'], selected = account) =>
    access.props(botWorkflow(action), { profile, account: selected });
  const [pane, setPane] = useState<BotPane>('enroll');
  const [role, setRole] = useState<'owner' | 'admin' | 'member'>('member');
  const [visibility, setVisibility] = useState(0);
  const [pin, setPin] = useState('');
  const [alias, setAlias] = useState('');
  const [target, setTarget] = useState('');
  const [revokeConfirmed, setRevokeConfirmed] = useState(false);
  const [rows, setRows] = useState<BotEnrollment[]>([]);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');
  const owner = `${profile}/${account}`;
  const active = useRef(owner);
  useEffect(() => {
    active.current = owner;
    setRows([]);
    setPin('');
    setAlias('');
    setTarget('');
    setRevokeConfirmed(false);
    setBusy(false);
    setMessage('');
    setError('');
    return () => {
      active.current = '';
    };
  }, [owner]);
  const run = async (action: BotAction, selected = account) => {
    if (busy) return;
    setBusy(true);
    setError('');
    const supplied = pin || null;
    setPin('');
    try {
      const needsHardware = 'operation_id' in action && 'pin' in action &&
        rows.some((row) => row.operation_id === action.operation_id && row.hardware_required);
      access.require(botWorkflow(action.action), {
        profile, account: selected,
        ...(needsHardware && !supplied ? { hardware: 'needed' as const } : {}),
      });
      const result = await bridge.botAccount(
        profile,
        selected,
        'pin' in action ? { ...action, pin: supplied } : action,
      );
      if (active.current !== owner) return;
      if (
        result.rows.some(
          (p) =>
            p.account_alias !== selected ||
            ('operation_id' in action &&
              p.operation_id !== action.operation_id),
        )
      )
        throw new Error('Bot enrollment belongs to a different account.');
      setRows(result.rows);
      setMessage(result.message);
      if (['load-file', 'unload', 'revoke'].includes(action.action))
        await onComplete();
    } catch (e) {
      if (active.current === owner) setError(normalizeCommandError(e).message);
    } finally {
      if (active.current === owner) setBusy(false);
    }
  };
  const visibilityValid =
    Number.isInteger(visibility) && visibility >= -32768 && visibility <= 32767;
  const pinField = (
    <InsetRow label="Security key PIN (enrolled keys only)">
      <input
        type="password"
        autoComplete="off"
        value={pin}
        maxLength={32}
        disabled={busy}
        onChange={(e) => setPin(e.target.value)}
      />
    </InsetRow>
  );
  const paneActions: ReactNode =
    pane === 'enroll' ? (
      <>
        <Button {...eligibility('list')} disabled={busy || eligibility('list').disabled} onClick={() => void run({ action: 'list' })}>
          Show pending enrollments
        </Button>
        <Button
          variant="primary"
          title={eligibility('prepare').title}
          disabled={busy || eligibility('prepare').disabled || !visibilityValid}
          onClick={() =>
            void run({
              action: 'prepare',
              role: role === 'member' ? { member: { visibility } } : role,
              pin: null,
            })
          }
        >
          Enroll bot
        </Button>
      </>
    ) : pane === 'load' ? (
      <>
        <Button {...eligibility('unload')} disabled={busy || eligibility('unload').disabled} onClick={() => void run({ action: 'unload' })}>
          Unload
        </Button>
        <Button
          variant="primary"
          title={eligibility('load-file', alias).title}
          disabled={busy || eligibility('load-file', alias).disabled || !/^[a-zA-Z0-9_-]{1,64}$/.test(alias)}
          onClick={() => void run({ action: 'load-file' }, alias)}
        >
          Load from file
        </Button>
      </>
    ) : revokeConfirmed ? (
      <Button
        variant="danger"
        title={eligibility('revoke').title}
        disabled={busy || eligibility('revoke').disabled}
        onClick={() => {
          setRevokeConfirmed(false);
          void run({ action: 'revoke', device_id: target, pin: null });
        }}
      >
        Confirm revocation
      </Button>
    ) : (
      <Button
        title={eligibility('revoke').title}
        disabled={busy || eligibility('revoke').disabled || !/^13[0-9a-f]{64}$/.test(target)}
        onClick={() => setRevokeConfirmed(true)}
      >
        Revoke
      </Button>
    );
  return (
    <PanelSheet
      presentation={presentation}
      busy={busy}
      footer={
        <>
          <Button disabled={busy} onClick={presentation.onClose}>
            Close
          </Button>
          {paneActions}
        </>
      }
    >
      <p>
        A bot is a device credential for automation. Export its token once and
        keep the file to reload it after a restart.
      </p>
      <SegmentedControl<BotPane>
        label="Bot account action"
        value={pane}
        onChange={setPane}
        items={[
          { id: 'enroll', label: 'Enroll' },
          { id: 'load', label: 'Load' },
          { id: 'revoke', label: 'Revoke' },
        ]}
      />
      {message && <p role="status">{message}</p>}
      {error && (
        <p role="alert" className="crit">
          {error}
        </p>
      )}
      {pane === 'enroll' && (
        <Inset className="form">
          <InsetRow label="Role">
            <select
              value={role}
              disabled={busy}
              onChange={(e) => setRole(e.target.value as typeof role)}
            >
              <option value="member">Member</option>
              <option value="admin">Admin</option>
              <option value="owner">Owner</option>
            </select>
          </InsetRow>
          {role === 'member' && (
            <InsetRow label="Member visibility level">
              <input
                type="number"
                min={-32768}
                max={32767}
                value={visibility}
                disabled={busy}
                onChange={(e) => setVisibility(Number(e.target.value))}
              />
            </InsetRow>
          )}
          {pinField}
        </Inset>
      )}
      {pane === 'load' && (
        <Inset className="form">
          <InsetRow label="Local label">
            <input
              value={alias}
              maxLength={64}
              disabled={busy}
              onChange={(e) => setAlias(e.target.value)}
            />
          </InsetRow>
        </Inset>
      )}
      {pane === 'revoke' && (
        <>
          <Inset className="form">
            <InsetRow label="Credential ID" valueClass="mono">
              <input
                className="mono"
                value={target}
                maxLength={66}
                disabled={busy}
                onChange={(e) => {
                  setTarget(e.target.value);
                  setRevokeConfirmed(false);
                }}
              />
            </InsetRow>
            {pinField}
          </Inset>
          {revokeConfirmed && (
            <p>
              Revoke <code>{target}</code>? The bot loses access to new keys and
              requests.
            </p>
          )}
        </>
      )}
      {rows.map((p) => (
        <div key={p.operation_id} className="op">
          <p role="status">
            {p.name} · {p.role} · {p.state}
          </p>
          <p>
            Credential: <code>{p.device_id}</code>
          </p>
          {p.hardware_required && (
            <p>Unlock this account’s security key to continue.</p>
          )}
          {p.state === 'submission-unknown' && (
            <p>
              The bot enrollment may have already succeeded on the server. Check
              the operation status before attempting to enroll again.
            </p>
          )}
          <div className="btns">
            {p.state === 'prepared' && (
              <>
                <Button
                  variant="primary"
                  title={eligibility('attempt').title ?? (p.hardware_required && !pin ? 'Enter the security key PIN to continue.' : undefined)}
                  disabled={busy || eligibility('attempt').disabled || (p.hardware_required && !pin)}
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
                  title={eligibility('cancel').title}
                  disabled={busy || eligibility('cancel').disabled}
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
                title={eligibility('status').title ?? (p.hardware_required && !pin ? 'Enter the security key PIN to continue.' : undefined)}
                disabled={busy || eligibility('status').disabled || (p.hardware_required && !pin)}
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
            {p.export_available && (
              <Button
                variant="primary"
                title={eligibility('export-file').title}
                disabled={busy || eligibility('export-file').disabled}
                onClick={() =>
                  void run({
                    action: 'export-file',
                    operation_id: p.operation_id,
                  })
                }
              >
                Export token
              </Button>
            )}
            {p.state === 'complete' && (
              <Button
                danger
                disabled={busy}
                onClick={() => {
                  setTarget(p.device_id);
                  setRevokeConfirmed(true);
                  setPane('revoke');
                }}
              >
                Revoke
              </Button>
            )}
          </div>
        </div>
      ))}
    </PanelSheet>
  );
}
