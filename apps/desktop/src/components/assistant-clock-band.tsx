/** The agent's pause on MCP writes for one account, and the time confirmation that lifts it. */

import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import type { AdapterClock } from '../adapter-clock-contract';
import { normalizeCommandError, type Bridge } from '../bridge';
import { useWorkflowAccess } from '../workflow-context';
import { Band } from './notice';
import { Button } from './button';

/**
 * Drawn only while the agent reports the account's adapter clock as paused:
 * its stored anchor disagrees with this computer's clock beyond what elapsed
 * time explains. Confirming re-anchors the clock at the computer's current
 * time; MCP tools cannot do this themselves. A failed status read draws
 * nothing, because the band is advisory and writes report their own refusal.
 */
export function AssistantClockBand({
  bridge,
  profile,
  account,
}: {
  bridge: Bridge;
  profile: string;
  account: string;
}): ReactNode {
  const access = useWorkflowAccess();
  const target = { profile, account };
  const [clock, setClock] = useState<AdapterClock | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let current = true;
    setClock(null);
    setError(null);
    bridge.adapterClock(profile, account, false).then(
      (next) => {
        if (current) setClock(next);
      },
      () => undefined,
    );
    return () => {
      current = false;
    };
  }, [bridge, profile, account]);
  if (clock?.admission !== 'paused') return null;
  const confirm = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      access.require('assistant-clock', target);
      const next = await bridge.adapterClock(profile, account, true);
      setClock(next);
      if (next.admission === 'paused')
        setError(
          'Writes are still paused. Correct the system clock, then confirm again.',
        );
    } catch (failure) {
      setError(normalizeCommandError(failure).message);
      // A confirmation can take effect before its reply is lost; reread.
      await bridge.adapterClock(profile, account, false).then(
        (next) => setClock(next),
        () => undefined,
      );
    } finally {
      setBusy(false);
    }
  };
  const eligibility = access.props('assistant-clock', target);
  return (
    <Band
      label="Assistant writes paused: confirm this computer’s time."
      action={
        <Button
          size="sm"
          title={eligibility.title}
          disabled={busy || eligibility.disabled}
          onClick={() => void confirm()}
        >
          Confirm time
        </Button>
      }
    >
      MCP assistants cannot write to this account until you confirm that this
      computer’s clock is correct. It read{' '}
      {new Date(clock.localTime * 1_000).toLocaleString()} when checked; Confirm
      time uses its current reading. If the clock is wrong, correct the system
      clock first.
      {error ? (
        <>
          {' '}
          <span role="alert">{error}</span>
        </>
      ) : null}
    </Band>
  );
}
