/** Prepare a username change for confirmation on the account page. */

import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useWorkflowAccess } from '../workflow-context';
import type { Bridge } from '../bridge';
import type { RenameProgress } from '../rename-contract';
import { checkRenameRows } from '../rename-status';
import { normalizeCommandError } from '../bridge';
import { useSheetGuard } from '../navigation-guard';
import { Button, Inset, InsetRow, PanelSheet } from './index';
import type { PanelPresentation } from './index';

export function RenamePanel({
  bridge,
  profile,
  account,
  presentation,
  onPrepared,
}: {
  bridge: Bridge;
  profile: string;
  account: string;
  presentation: PanelPresentation;
  /** Store the preparation response and close the dialog. */
  onPrepared: (rows: RenameProgress[]) => void | Promise<void>;
}): ReactNode {
  const access = useWorkflowAccess();
  const target = { profile, account };
  const eligibility = access.props('account-rename', target);
  const [name, setName] = useState('');
  const [pin, setPin] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generation = useRef(0);
  useEffect(() => {
    generation.current += 1;
    setName('');
    setPin('');
    setBusy(false);
    setError(null);
    return () => {
      generation.current += 1;
    };
  }, [profile, account]);
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the rename to be prepared.' }
      : name
        ? {
            verdict: 'prompt',
            title: 'Discard username change?',
            body: 'The new username has not been prepared.',
            confirm: 'Discard',
            onConfirm: presentation.onClose,
          }
        : null,
  );
  const prepare = async () => {
    if (!name || busy) return;
    const version = generation.current;
    setBusy(true);
    setError(null);
    const suppliedPin = pin || null;
    setPin('');
    try {
      access.require('account-rename', target);
      const rows = await bridge.renameAccount(profile, account, {
        action: 'prepare',
        username: name,
        pin: suppliedPin,
      });
      if (generation.current !== version) return;
      checkRenameRows(rows, account, null);
      await onPrepared(rows);
    } catch (e) {
      if (generation.current !== version) return;
      setError(normalizeCommandError(e).message);
      setBusy(false);
    }
  };
  const disabled = busy || eligibility.disabled;
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
            variant="primary"
            busy={busy}
            title={eligibility.title}
            disabled={disabled || !name}
            onClick={() => void prepare()}
          >
            {busy ? 'Preparing…' : 'Prepare rename'}
          </Button>
        </>
      }
    >
      <p>
        Prepares a new username for this account. Nothing changes on the server
        until you confirm the rename on the account page.
      </p>
      <Inset className="form">
        <InsetRow label="Username">
          <input
            value={name}
            maxLength={256}
            disabled={disabled}
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
            disabled={disabled}
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
    </PanelSheet>
  );
}
