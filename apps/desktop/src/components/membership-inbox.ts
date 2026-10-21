/**
 * Membership requests for one or more teams: the rows the agent's `inbox`
 * action returns, and the approve, reject and verify actions on them. The
 * team page's Requests tab and the Activity page both read requests through
 * this hook, so approving in either place sends the same command, settles the
 * same shared rows (`settleInvitationWrite`) and refreshes the other through
 * the invitation activity event.
 *
 * Inbox rows can carry tokens, so they live only in this hook's state and are
 * dropped with the component; only counts enter the shared repository.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { normalizeCommandError } from '../bridge';
import type { Bridge } from '../bridge';
import { useDeviceCache } from '../device-cache';
import { INVITATION_ACTIVITY } from '../invitation-activity';
import type { InvitationActivityDetail } from '../invitation-activity';
import type { InvitationAction, InvitationRow } from '../invitation-contract';
import type { InvitationGrantRole } from '../invitation-labels';
import {
  hardwareUnlockRequested,
  invitationRow,
  invitationRows,
  nativeRole,
  settleInvitationWrite,
} from '../invitation-writes';
import type { CollectionReadiness, StoreRef, TeamStore } from '../model';
import { useMetadataRepository } from '../query-hooks';

/** One team's requests as last read. */
export interface InboxEntry {
  rows: InvitationRow[];
  state: CollectionReadiness;
  /** The server said more requests may be pending than it returned. */
  truncated: boolean;
}

const LOADING: InboxEntry = { rows: [], state: 'loading', truncated: false };

export interface MembershipInbox {
  entry: (team: TeamStore) => InboxEntry;
  /** Reads one team's inbox again, with the security key PIN when given. */
  load: (team: TeamStore, secret?: string | null) => Promise<void>;
  /**
   * Runs one action with the shared busy state and error handling. A write
   * settles the team's shared invitation rows afterwards.
   */
  run: (
    team: TeamStore,
    work: () => Promise<void>,
    write?: boolean,
  ) => Promise<void>;
  /** Sends one invitation action as the team's account, with the PIN. */
  call: (team: TeamStore, action: InvitationAction) => Promise<InvitationRow>;
  /** Asks for the PIN when the row says the key requires it. */
  hardwareGate: (row: InvitationRow) => boolean;
  /** Records a read failure in the shared error line. */
  fail: (failure: unknown) => void;
  approve: (team: TeamStore, row: InvitationRow) => Promise<void>;
  reject: (team: TeamStore, row: InvitationRow) => Promise<void>;
  verify: (team: TeamStore, row: InvitationRow) => Promise<void>;
  /** The role a request will be approved at; remote teams join as Member. */
  roleOf: (row: InvitationRow) => InvitationGrantRole;
  setRole: (row: InvitationRow, role: InvitationGrantRole) => void;
  /** The server a remote request's requester is checked against. */
  remoteProfileOf: (row: InvitationRow) => string | undefined;
  setRemoteProfile: (row: InvitationRow, profile: string) => void;
  busy: boolean;
  error: string | null;
  needPin: boolean;
  pin: string;
  setPin: (pin: string) => void;
}

export function useMembershipInbox(
  bridge: Bridge,
  teams: readonly TeamStore[],
  {
    onApproved,
  }: {
    /** After a request was approved and its row removed. */
    onApproved?: (team: TeamStore) => Promise<void> | void;
  } = {},
): MembershipInbox {
  const devices = useDeviceCache();
  const queries = useMetadataRepository(bridge, devices?.repository);
  const [entries, setEntries] = useState<Record<StoreRef, InboxEntry>>({});
  const [roles, setRoles] = useState<Record<string, InvitationGrantRole>>({});
  const [remoteProfiles, setRemoteProfiles] = useState<Record<string, string>>(
    {},
  );
  const [busy, setBusy] = useState(false);
  const [pin, setPin] = useState('');
  const [needPin, setNeedPin] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generations = useRef(new Map<StoreRef, number>());
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  const approvedRef = useRef(onApproved);
  approvedRef.current = onApproved;

  const update = (team: StoreRef, next: (old: InboxEntry) => InboxEntry) =>
    setEntries((all) => ({ ...all, [team]: next(all[team] ?? LOADING) }));

  const fail = useCallback((failure: unknown) => {
    if (!live.current) return;
    if (hardwareUnlockRequested(failure)) {
      setNeedPin(true);
      setError('Enter the PIN for your security key, then refresh requests.');
    } else setError(normalizeCommandError(failure).message);
  }, []);

  const load = useCallback(
    async (team: TeamStore, secret: string | null = null): Promise<void> => {
      const generation = (generations.current.get(team.id) ?? 0) + 1;
      generations.current.set(team.id, generation);
      const current = () =>
        live.current && generations.current.get(team.id) === generation;
      update(team.id, (old) => ({ ...old, state: 'loading' }));
      setError(null);
      try {
        const inbox = await bridge.invitation(
          team.server,
          team.account,
          { action: 'inbox', team_alias: team.alias },
          secret,
        );
        if (!current()) return;
        update(team.id, () => ({
          rows: invitationRows(inbox).filter((row) => row.request_id),
          truncated: !Array.isArray(inbox) && inbox.possibly_truncated === true,
          state: 'ready',
        }));
        setPin('');
        setNeedPin(false);
      } catch (failure) {
        if (!current()) return;
        update(team.id, (old) => ({ ...old, state: 'unavailable' }));
        fail(failure);
      }
    },
    [bridge, fail],
  );

  // Read every team when the set of teams changes, and again whenever an
  // invitation action anywhere changes one of their accounts' requests.
  const signature = teams.map((team) => team.id).join('|');
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const stable = useMemo(() => teams, [signature]);
  useEffect(() => {
    for (const team of stable) void load(team);
  }, [stable, load]);
  useEffect(() => {
    const refresh = (event: Event) => {
      const scope = (event as CustomEvent<InvitationActivityDetail>).detail;
      for (const team of stable) {
        if (
          scope &&
          (scope.profile !== team.server || scope.account !== team.account)
        )
          continue;
        void load(team);
      }
    };
    window.addEventListener(INVITATION_ACTIVITY, refresh);
    return () => window.removeEventListener(INVITATION_ACTIVITY, refresh);
  }, [stable, load]);

  const call = (team: TeamStore, action: InvitationAction) =>
    bridge
      .invitation(team.server, team.account, action, pin || null)
      .then(invitationRow);

  const run = async (
    team: TeamStore,
    work: () => Promise<void>,
    write = true,
  ): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      await work();
    } catch (failure) {
      if (!live.current) return;
      if (hardwareUnlockRequested(failure)) {
        setNeedPin(true);
        setError('Enter the PIN for your security key to continue.');
      } else setError(normalizeCommandError(failure).message);
    } finally {
      if (live.current) setBusy(false);
      if (write) settleInvitationWrite(queries, team.server, team.account);
    }
  };

  const hardwareGate = (current: InvitationRow): boolean => {
    if (current.hardware_required) {
      setNeedPin(true);
      setError('Enter the PIN for your security key to continue.');
      return false;
    }
    setPin('');
    setNeedPin(false);
    return true;
  };

  const remoteProfileOf = (row: InvitationRow) =>
    remoteProfiles[row.request_id ?? ''] ??
    row.source_profile ??
    row.remote_profile;

  const roleOf = (row: InvitationRow): InvitationGrantRole =>
    row.remote ? 'member' : (roles[row.request_id ?? ''] ?? 'member');

  const drop = (team: TeamStore, id: string) =>
    update(team.id, (old) => ({
      ...old,
      rows: old.rows.filter((r) => r.request_id !== id),
    }));

  const approve = (team: TeamStore, row: InvitationRow) =>
    run(team, async () => {
      const id = row.request_id;
      if (!id) return;
      const role = nativeRole(roleOf(row));
      const remote = remoteProfileOf(row);
      const current = await call(
        team,
        row.remote && remote
          ? {
              action: 'approve-remote',
              remote_profile: remote,
              team_alias: team.alias,
              request_id: id,
              role,
            }
          : { action: 'approve', team_alias: team.alias, request_id: id, role },
      );
      if (!live.current || !hardwareGate(current)) return;
      if (current.state === 'complete') {
        drop(team, id);
        await approvedRef.current?.(team);
      }
    });

  const reject = (team: TeamStore, row: InvitationRow) =>
    run(team, async () => {
      const id = row.request_id;
      if (!id) return;
      await call(team, {
        action: 'reject',
        team_alias: team.alias,
        request_id: id,
      });
      if (!live.current) return;
      drop(team, id);
    });

  const verify = (team: TeamStore, row: InvitationRow) =>
    run(
      team,
      async () => {
        const id = row.request_id;
        const remote = remoteProfileOf(row);
        if (!id || !remote) return;
        const current = await call(team, {
          action: 'inspect-remote',
          remote_profile: remote,
          team_alias: team.alias,
          request_id: id,
        });
        if (!live.current) return;
        update(team.id, (old) => ({
          ...old,
          rows: old.rows.map((r) =>
            r.request_id === id ? { ...r, ...current } : r,
          ),
        }));
      },
      false,
    );

  return {
    entry: (team) => entries[team.id] ?? LOADING,
    load,
    run,
    call,
    hardwareGate,
    fail,
    approve,
    reject,
    verify,
    roleOf,
    setRole: (row, role) =>
      setRoles((all) => ({ ...all, [row.request_id ?? '']: role })),
    remoteProfileOf,
    setRemoteProfile: (row, profile) => {
      const id = row.request_id ?? '';
      setRemoteProfiles((all) => ({ ...all, [id]: profile }));
      // A different server has not vouched for this requester yet.
      setEntries((all) =>
        Object.fromEntries(
          Object.entries(all).map(([key, value]) => [
            key,
            {
              ...value,
              rows: value.rows.map((r) =>
                r.request_id === id ? { ...r, verified: false } : r,
              ),
            },
          ]),
        ),
      );
    },
    busy,
    error,
    needPin,
    pin,
    setPin,
  };
}
