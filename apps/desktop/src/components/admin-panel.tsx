import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import { normalizeCommandError } from '../bridge';
import { Button, Inset, InsetRow, PanelSheet } from './index';
import type { PanelPresentation } from './index';

export function AdminPanel({
  bridge,
  profile,
  account,
  presentation,
}: {
  bridge: Bridge;
  profile: string;
  account: string;
  presentation: PanelPresentation;
}): ReactNode {
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
            ? 'Address saved.'
            : 'Admin panel opened in a private window.',
        );
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
            disabled={busy || !destination}
            onClick={() => void run(true)}
          >
            Save address
          </Button>
          <Button
            variant="primary"
            disabled={busy}
            onClick={() => void run(false)}
          >
            Open admin panel
          </Button>
        </>
      }
    >
      <p>
        Opens the host’s web admin panel in a private window that closes when
        FOKS locks.
      </p>
      <Inset className="form">
        <InsetRow label="Admin panel address">
          <input
            type="url"
            value={destination}
            maxLength={2048}
            disabled={busy}
            placeholder="Address from the host operator"
            onChange={(e) => setDestination(e.target.value)}
          />
        </InsetRow>
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
      </Inset>
      {message && <p role="status">{message}</p>}
      {error && (
        <p role="alert" className="crit">
          {error}
        </p>
      )}
    </PanelSheet>
  );
}
