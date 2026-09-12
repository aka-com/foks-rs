import { useEffect, useRef, useState } from 'react';
import type { Bridge } from '../bridge';
import type { RenameAction, RenameProgress } from '../rename-contract';
import { normalizeCommandError } from '../bridge';

export function RenamePanel({
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
    <section className="pcard" aria-label={`Change username for ${account}`}>
      <h3>Change username · {account}</h3>
      <p>
        The local label for this account remains {account}. Enter a name, then
        confirm the change.
      </p>
      <label>
        New username{' '}
        <input
          value={name}
          maxLength={256}
          disabled={busy}
          onChange={(e) => setName(e.target.value)}
        />
      </label>
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
        disabled={busy || !name}
        onClick={() =>
          void run({ action: 'prepare', username: name, pin: null })
        }
      >
        Prepare rename
      </button>
      <button disabled={busy} onClick={() => void run(null)}>
        Recover rename operations
      </button>
      {error && <p role="alert">{error}</p>}
      {rows.map((p) => (
        <div key={p.operation_id}>
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
                Confirm rename
              </button>
              <button
                disabled={busy}
                onClick={() =>
                  void run({ action: 'cancel', operation_id: p.operation_id })
                }
              >
                Cancel rename
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
              Check original operation
            </button>
          )}
        </div>
      ))}
    </section>
  );
}
