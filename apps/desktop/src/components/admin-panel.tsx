import { useEffect, useRef, useState } from 'react';
import type { Bridge } from '../bridge';
import { normalizeCommandError } from '../bridge';
export function AdminPanel({
  bridge,
  profile,
  account,
}: {
  bridge: Bridge;
  profile: string;
  account: string;
}) {
  const [destination, setDestination] = useState('');
  const [pin, setPin] = useState('');
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');
  const owner = `${profile}/${account}`;
  const active = useRef(owner);
  useEffect(() => {
    active.current = owner;
    setDestination('');
    setPin('');
    setMessage('');
    setError('');
    setBusy(false);
    return () => {
      active.current = '';
    };
  }, [owner]);
  const run = async (configure: boolean) => {
    if (busy) return;
    setBusy(true);
    setError('');
    const supplied = pin || null;
    setPin('');
    try {
      if (configure)
        await bridge.configureWebAdmin(profile, account, destination);
      else await bridge.openWebAdmin(profile, account, supplied);
      if (active.current === owner)
        setMessage(
          configure
            ? 'Admin destination saved.'
            : 'Host administration opened in a private window.',
        );
    } catch (e) {
      if (active.current === owner) setError(normalizeCommandError(e).message);
    } finally {
      if (active.current === owner) setBusy(false);
    }
  };
  return (
    <section
      className="pcard"
      aria-label={`Host administration for ${account}`}
    >
      <h3>Host administration · {account}</h3>
      <p>
        For hosts that offer a web administration panel. Enter the HTTPS address
        supplied by your host operator. The panel opens in a private window and
        closes when you lock FOKS.
      </p>
      <label>
        Admin HTTPS address{' '}
        <input
          type="url"
          value={destination}
          maxLength={2048}
          disabled={busy}
          placeholder="https://admin.example/"
          onChange={(e) => setDestination(e.target.value)}
        />
      </label>
      <button disabled={busy || !destination} onClick={() => void run(true)}>
        Save admin destination
      </button>
      <label>
        Admin security key PIN, if needed{' '}
        <input
          type="password"
          autoComplete="off"
          value={pin}
          maxLength={32}
          disabled={busy}
          onChange={(e) => setPin(e.target.value)}
        />
      </label>
      <button disabled={busy} onClick={() => void run(false)}>
        Open host administration
      </button>
      {message && <p role="status">{message}</p>}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
