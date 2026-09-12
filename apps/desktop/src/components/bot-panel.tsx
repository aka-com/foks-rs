import { useEffect, useRef, useState } from 'react';
import type { Bridge } from '../bridge';
import { normalizeCommandError } from '../bridge';
import type { BotAction, BotEnrollment } from '../bot-contract';
export function BotPanel({
  bridge,
  profile,
  account,
  onComplete,
}: {
  bridge: Bridge;
  profile: string;
  account: string;
  onComplete: () => void | Promise<void>;
}) {
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
  return (
    <section className="pcard" aria-label={`Bot credentials for ${account}`}>
      <h3>Bot credentials · {account}</h3>
      <p>
        Choose a role, confirm enrollment, then export the token once to a
        private file. Keep that file to load the bot after an agent restart.
      </p>
      <label>
        Bot role{' '}
        <select
          value={role}
          disabled={busy}
          onChange={(e) => setRole(e.target.value as typeof role)}
        >
          <option value="member">Member</option>
          <option value="admin">Admin</option>
          <option value="owner">Owner</option>
        </select>
      </label>
      {role === 'member' && (
        <label>
          Visibility{' '}
          <input
            type="number"
            min={-32768}
            max={32767}
            value={visibility}
            onChange={(e) => setVisibility(Number(e.target.value))}
          />
        </label>
      )}
      <label>
        Security key PIN, if needed{' '}
        <input
          type="password"
          autoComplete="off"
          value={pin}
          maxLength={32}
          disabled={busy}
          onChange={(e) => setPin(e.target.value)}
        />
      </label>
      <button
        disabled={
          busy ||
          !Number.isInteger(visibility) ||
          visibility < -32768 ||
          visibility > 32767
        }
        onClick={() =>
          void run({
            action: 'prepare',
            role: role === 'member' ? { member: { visibility } } : role,
            pin: null,
          })
        }
      >
        Prepare bot enrollment
      </button>
      <button disabled={busy} onClick={() => void run({ action: 'list' })}>
        Recover bot enrollments
      </button>
      {rows.map((p) => (
        <div key={p.operation_id}>
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
          {p.state === 'prepared' && (
            <>
              <button
                disabled={busy}
                onClick={() =>
                  void run({
                    action: 'attempt',
                    operation_id: p.operation_id,
                    pin: null,
                  })
                }
              >
                Confirm bot enrollment
              </button>
              <button
                disabled={busy}
                onClick={() =>
                  void run({ action: 'cancel', operation_id: p.operation_id })
                }
              >
                Cancel enrollment
              </button>
            </>
          )}
          {!['complete', 'rejected'].includes(p.state) && (
            <button
              disabled={busy}
              onClick={() =>
                void run({
                  action: 'status',
                  operation_id: p.operation_id,
                  pin: null,
                })
              }
            >
              Check original enrollment
            </button>
          )}
          {p.export_available && (
            <button
              disabled={busy}
              onClick={() =>
                void run({
                  action: 'export-file',
                  operation_id: p.operation_id,
                })
              }
            >
              Export token once
            </button>
          )}
          {p.state === 'complete' && (
            <button
              disabled={busy}
              onClick={() => {
                setTarget(p.device_id);
                setRevokeConfirmed(true);
              }}
            >
              Review bot revocation
            </button>
          )}
        </div>
      ))}
      <label>
        Bot account label{' '}
        <input
          value={alias}
          maxLength={64}
          disabled={busy}
          onChange={(e) => setAlias(e.target.value)}
        />
      </label>
      <button
        disabled={busy || !/^[a-zA-Z0-9_-]{1,64}$/.test(alias)}
        onClick={() => void run({ action: 'load-file' }, alias)}
      >
        Load bot from private file
      </button>
      <button disabled={busy} onClick={() => void run({ action: 'unload' })}>
        Unload selected bot
      </button>
      <label>
        Bot credential to revoke{' '}
        <input
          value={target}
          maxLength={66}
          disabled={busy}
          onChange={(e) => {
            setTarget(e.target.value);
            setRevokeConfirmed(false);
          }}
        />
      </label>
      {revokeConfirmed ? (
        <>
          <p>
            Revoke credential {target}? It will lose access to new keys and
            requests.
          </p>
          <button
            disabled={busy}
            onClick={() => {
              setRevokeConfirmed(false);
              void run({ action: 'revoke', device_id: target, pin: null });
            }}
          >
            Confirm bot revocation
          </button>
        </>
      ) : (
        <button
          disabled={busy || !/^13[0-9a-f]{64}$/.test(target)}
          onClick={() => setRevokeConfirmed(true)}
        >
          Review revocation
        </button>
      )}
      {message && <p role="status">{message}</p>}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
