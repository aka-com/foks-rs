/**
 * Account-scoped rename state for Settings. Pending operations remain visible
 * after the preparation dialog closes. Listings omit target usernames, so
 * targets are retained from action responses until the account changes.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { Bridge } from './bridge';
import { normalizeCommandError } from './bridge';
import type { RenameAction, RenameProgress } from './rename-contract';

/** Terminal operation states. */
const FINISHED: readonly RenameProgress['state'][] = ['complete', 'rejected'];

/** First nonterminal operation. */
export function pendingRename(
  rows: readonly RenameProgress[],
): RenameProgress | undefined {
  return rows.find((row) => !FINISHED.includes(row.state));
}

/** Validate the response account and operation ID. */
export function checkRenameRows(
  rows: readonly RenameProgress[],
  account: string,
  action: RenameAction | null,
): void {
  const id = action && 'operation_id' in action ? action.operation_id : null;
  if (
    rows.some(
      (row) =>
        row.account_alias !== account ||
        (id !== null && row.operation_id !== id),
    )
  )
    throw new Error('Rename belongs to a different account.');
}

export interface RenameStatus {
  /** Current nonterminal operation. */
  pending: RenameProgress | undefined;
  /** The username the pending operation would set, when this app prepared it. */
  target: string | undefined;
  /** A status listing is in progress. */
  loading: boolean;
  /** True while an action against the pending operation runs. */
  busy: boolean;
  /** Latest listing or action error. */
  error: string | null;
  /** The last action failed without confirming the operation state. */
  needsCheck: boolean;
  /** Reads the list again. */
  refresh: () => Promise<void>;
  /** Run an action, returning undefined and setting error on failure. */
  act: (action: RenameAction) => Promise<RenameProgress | undefined>;
  /** Merge action results and invalidate older listings. */
  learn: (rows: readonly RenameProgress[]) => void;
}

export function useRenameStatus({
  bridge,
  profile,
  account,
  enabled,
}: {
  bridge: Bridge;
  profile: string | undefined;
  account: string | undefined;
  /** False while the listing cannot run, such as when access is stopped. */
  enabled: boolean;
}): RenameStatus {
  const owner = profile && account ? `${profile}/${account}` : '';
  const active = useRef(owner);
  const generation = useRef(0);
  const readId = useRef(0);
  const writing = useRef<symbol | null>(null);
  const [rows, setRows] = useState<RenameProgress[]>([]);
  const [targets, setTargets] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [uncertain, setUncertain] = useState<string | null>(null);

  const learn = useCallback((answered: readonly RenameProgress[]) => {
    generation.current += 1;
    setLoading(false);
    setTargets((known) => {
      const next = { ...known };
      for (const row of answered)
        if (row.target !== null) next[row.operation_id] = row.target;
      return next;
    });
    setRows((current) => {
      const ids = new Set(answered.map((row) => row.operation_id));
      return [
        ...answered,
        ...current.filter((row) => !ids.has(row.operation_id)),
      ];
    });
  }, []);

  const refresh = useCallback(async () => {
    if (!profile || !account || !enabled || writing.current) return;
    const scope = `${profile}/${account}`;
    const version = generation.current;
    const request = ++readId.current;
    const current = () =>
      active.current === scope &&
      generation.current === version &&
      readId.current === request;
    setLoading(true);
    try {
      const listed = await bridge.renameAccount(profile, account, null);
      if (!current()) return;
      checkRenameRows(listed, account, null);
      setRows(listed);
      setUncertain(null);
      setError(null);
    } catch (e) {
      if (current()) setError(normalizeCommandError(e).message);
    } finally {
      if (current()) setLoading(false);
    }
  }, [account, bridge, enabled, profile]);

  // Preserve pending state across access changes; reset on account changes.
  useEffect(() => {
    active.current = owner;
    generation.current += 1;
    writing.current = null;
    setUncertain(null);
    setLoading(false);
    setRows([]);
    setTargets({});
    setError(null);
    setBusy(false);
    return () => {
      active.current = '';
      generation.current += 1;
    };
  }, [owner]);
  useEffect(() => {
    void refresh();
  }, [refresh]);

  const act = useCallback(
    async (action: RenameAction) => {
      if (!profile || !account || !enabled || writing.current) return undefined;
      const version = ++generation.current;
      const operation = Symbol();
      writing.current = operation;
      setLoading(false);
      const scope = `${profile}/${account}`;
      setBusy(true);
      setError(null);
      try {
        const answered = await bridge.renameAccount(profile, account, action);
        if (active.current !== scope || generation.current !== version)
          return undefined;
        checkRenameRows(answered, account, action);
        setUncertain(null);
        learn(answered);
        return answered[0];
      } catch (e) {
        if (active.current === scope && generation.current === version) {
          setError(normalizeCommandError(e).message);
          if ('operation_id' in action) setUncertain(action.operation_id);
        }
        return undefined;
      } finally {
        if (writing.current === operation) {
          writing.current = null;
          setBusy(false);
        }
      }
    },
    [account, bridge, enabled, learn, profile],
  );

  const pending = pendingRename(rows);
  return {
    pending,
    target: pending ? targets[pending.operation_id] : undefined,
    loading,
    busy,
    error,
    needsCheck: pending !== undefined && uncertain === pending.operation_id,
    refresh,
    act,
    learn,
  };
}
