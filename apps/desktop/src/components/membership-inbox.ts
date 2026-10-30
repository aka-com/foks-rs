/**
 * Loads membership requests for one or more teams via the agent's `inbox`
 * action and provides approve, reject, and verify actions. Both the team
 * Requests tab and the Activity screen use this hook, so approving a request in
 * either view runs the same command, updates shared invitation state
 * (`settleInvitationWrite`), and refreshes the other view via the invitation
 * activity event.
 *
 * Because inbox rows can contain invitation tokens, they are kept in component
 * state and discarded on unmount; only request counts are stored in the shared
 * metadata repository.
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

/** Last loaded membership requests for a single team. */
export interface InboxEntry {
  rows: InvitationRow[];
  state: CollectionReadiness;
  /** True when the server indicates additional pending requests were omitted. */
  truncated: boolean;
}

const LOADING: InboxEntry = { rows: [], state: 'loading', truncated: false };

export interface MembershipInbox {
  entry: (team: TeamStore) => InboxEntry;
  /** Reloads a team's inbox, optionally passing a security key PIN. */
  load: (team: TeamStore, secret?: string | null) => Promise<void>;
  /**
   * Runs an inbox action with shared busy state and error handling. When
   * `write` is true, reconciles the team's shared invitation rows afterward.
   */
  run: (
    team: TeamStore,
    work: () => Promise<void>,
    write?: boolean,
  ) => Promise<void>;
  /** Executes an invitation action for the team's account using the current PIN. */
  call: (team: TeamStore, action: InvitationAction) => Promise<InvitationRow>;
  /** Prompts for a security key PIN if `row.hardware_required` is set. */
  hardwareGate: (row: InvitationRow) => boolean;
  /** Formats a read failure into the shared error message state. */
  fail: (failure: unknown) => void;
  approve: (team: TeamStore, row: InvitationRow) => Promise<void>;
  reject: (team: TeamStore, row: InvitationRow) => Promise<void>;
  verify: (team: TeamStore, row: InvitationRow) => Promise<void>;
  /** Returns the selected approval role for a request (remote teams always join as Member). */
  roleOf: (row: InvitationRow) => InvitationGrantRole;
  setRole: (row: InvitationRow, role: InvitationGrantRole) => void;
  /** Returns the server profile used to verify a remote requester. */
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
    /** Called after a request is approved and removed from the inbox list. */
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

  // Reload all team inboxes when the team list changes or when an invitation
  // activity event affects one of their accounts.
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
      // Clear verification status when switching to a different remote server.
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
