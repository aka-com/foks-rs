import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { normalizeCommandError, type Bridge } from '../bridge';
import { useDeviceCache } from '../device-cache';
import { INVITATION_ACTIVITY } from '../invitation-activity';
import type { InvitationActivityDetail } from '../invitation-activity';
import type { InvitationAction, InvitationRow } from '../invitation-contract';
import {
  forgetInvitationLabel,
  invitationLabel,
  relativeTime,
} from '../invitation-labels';
import type { InvitationGrantRole } from '../invitation-labels';
import {
  grantRoleName,
  hardwareUnlockRequested,
  invitationRow,
  invitationRows,
  nativeRole,
  operationStateText,
  settleInvitationWrite,
  unfinishedOperation,
} from '../invitation-writes';
import type { TeamStore } from '../model';
import { serverDisplayLabel } from '../model';
import type { JoinServer } from './invitation-panel';
import { useMetadataRepository } from '../query-hooks';
import {
  Button,
  Chip,
  Icon,
  Inset,
  InsetRow,
  Notice,
  SectionLabel,
} from './index';

export function MembershipRequests({
  bridge,
  team,
  serverLabel,
  servers,
  onComplete,
  onInvite,
  onActivity,
}: {
  bridge: Bridge;
  team: TeamStore;
  serverLabel: string;
  servers: readonly JoinServer[];
  onComplete: () => Promise<void> | void;
  onInvite: () => void;
  onActivity: () => void;
}): ReactNode {
  const profile = team.server;
  const account = team.account;
  const devices = useDeviceCache();
  const queries = useMetadataRepository(bridge, devices?.repository);
  const [rows, setRows] = useState<InvitationRow[]>([]);
  const [operations, setOperations] = useState<InvitationRow[]>([]);
  const [roles, setRoles] = useState<Record<string, InvitationGrantRole>>({});
  const [remoteProfiles, setRemoteProfiles] = useState<Record<string, string>>(
    {},
  );
  const [loaded, setLoaded] = useState(false);
  const [truncated, setTruncated] = useState(false);
  const loadGeneration = useRef(0);
  const [busy, setBusy] = useState(false);
  const [pin, setPin] = useState('');
  const [needPin, setNeedPin] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);

  const load = useCallback(
    async (report: boolean, secret: string | null = null): Promise<void> => {
      const generation = ++loadGeneration.current;
      const current = () =>
        live.current && generation === loadGeneration.current;
      const failed = (failure: unknown) => {
        if (!current()) return;
        if (hardwareUnlockRequested(failure)) {
          setNeedPin(true);
          setError(
            'Enter the PIN for your security key, then refresh requests.',
          );
        } else if (report) setError(normalizeCommandError(failure).message);
      };
      await Promise.all([
        bridge
          .invitation(
            profile,
            account,
            { action: 'inbox', team_alias: team.alias },
            secret,
          )
          .then((inbox) => {
            if (!current()) return;
            setRows(invitationRows(inbox).filter((row) => row.request_id));
            setTruncated(
              !Array.isArray(inbox) && inbox.possibly_truncated === true,
            );
            setLoaded(true);
            setPin('');
            setNeedPin(false);
          })
          .catch(failed),
        bridge
          .invitation(profile, account, { action: 'list' }, null)
          .then((list) => {
            if (!current()) return;
            setOperations(
              invitationRows(list).filter(
                (row) =>
                  row.team_id === team.team_id_hex && unfinishedOperation(row),
              ),
            );
          })
          .catch(failed),
      ]);
    },
    [bridge, profile, account, team.alias, team.team_id_hex],
  );

  useEffect(() => {
    void load(false);
  }, [load]);

  useEffect(() => {
    const refresh = (event: Event) => {
      const scope = (event as CustomEvent<InvitationActivityDetail>).detail;
      if (scope && (scope.profile !== profile || scope.account !== account))
        return;
      void load(false);
    };
    window.addEventListener(INVITATION_ACTIVITY, refresh);
    return () => window.removeEventListener(INVITATION_ACTIVITY, refresh);
  }, [load, profile, account]);

  const call = (action: InvitationAction) =>
    bridge
      .invitation(profile, account, action, pin || null)
      .then(invitationRow);

  const run = async (
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
      if (write) settleInvitationWrite(queries, profile, account);
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

  const approve = (row: InvitationRow) =>
    run(async () => {
      const id = row.request_id;
      if (!id) return;
      const role = nativeRole(row.remote ? 'member' : (roles[id] ?? 'member'));
      const remote = remoteProfileOf(row);
      const current = await call(
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
        setRows((all) => all.filter((r) => r.request_id !== id));
        await onComplete();
      }
    });

  const reject = (row: InvitationRow) =>
    run(async () => {
      const id = row.request_id;
      if (!id) return;
      await call({ action: 'reject', team_alias: team.alias, request_id: id });
      if (!live.current) return;
      setRows((all) => all.filter((r) => r.request_id !== id));
    });

  const verify = (row: InvitationRow) =>
    run(async () => {
      const id = row.request_id;
      const remote = remoteProfileOf(row);
      if (!id || !remote) return;
      const current = await call({
        action: 'inspect-remote',
        remote_profile: remote,
        team_alias: team.alias,
        request_id: id,
      });
      if (!live.current) return;
      setRows((all) =>
        all.map((r) => (r.request_id === id ? { ...r, ...current } : r)),
      );
    }, false);

  const finish = (row: InvitationRow) =>
    run(async () => {
      if (!row.operation_id) return;
      const current = await call({
        action: 'attempt',
        operation_id: row.operation_id,
      });
      if (!live.current || !hardwareGate(current)) return;
      setOperations((all) =>
        all
          .map((r) => (r.operation_id === row.operation_id ? current : r))
          .filter(unfinishedOperation),
      );
    });

  const check = (row: InvitationRow) =>
    run(async () => {
      if (!row.operation_id) return;
      const current = await call({
        action: 'status',
        operation_id: row.operation_id,
      });
      if (!live.current || !hardwareGate(current)) return;
      setOperations((all) =>
        all
          .map((r) => (r.operation_id === row.operation_id ? current : r))
          .filter(unfinishedOperation),
      );
    }, false);

  const discard = (row: InvitationRow) =>
    run(async () => {
      if (!row.operation_id) return;
      await call({ action: 'cancel', operation_id: row.operation_id });
      forgetInvitationLabel(profile, account, row.operation_id);
      if (!live.current) return;
      setOperations((all) =>
        all.filter((r) => r.operation_id !== row.operation_id),
      );
    });

  const requestRow = (row: InvitationRow): ReactNode => {
    const id = row.request_id!;
    const name = row.username ?? 'Unknown user';
    const remote = remoteProfileOf(row);
    const isTeam = row.joiner_kind === 'team';
    const where = row.remote
      ? remote
        ? `${isTeam ? 'Team on' : 'on'} ${remote}`
        : 'on another server'
      : `on ${serverLabel}`;
    const role = row.remote ? 'member' : (roles[id] ?? 'member');
    return (
      <article key={id} className="op" aria-label={`Request from ${name}`}>
        <p>
          <b>{name}</b>
          {isTeam ? <Chip>Team</Chip> : null}
          {row.remote ? <Chip>Remote</Chip> : null}
        </p>
        <p className="fn">
          {where}
          {row.time ? ` · ${relativeTime(row.time)}` : ''}
        </p>
        {row.remote ? (
          <Inset className="form">
            <InsetRow label={`Server for ${name}`}>
              <select
                value={remote ?? ''}
                disabled={busy}
                onChange={(event) => {
                  const selected = event.target.value;
                  setRemoteProfiles((all) => ({ ...all, [id]: selected }));
                  setRows((all) =>
                    all.map((r) =>
                      r.request_id === id ? { ...r, verified: false } : r,
                    ),
                  );
                }}
              >
                <option value="">Choose the requester’s server</option>
                {servers
                  .filter((server) => server.profileName !== profile)
                  .map((server) => (
                    <option key={server.profileName} value={server.profileName}>
                      {serverDisplayLabel(server)} ({server.configuredEndpoint})
                    </option>
                  ))}
              </select>
            </InsetRow>
          </Inset>
        ) : null}
        {row.remote && !remote ? (
          <Notice severity="crit" title="Cannot verify">
            <p>
              Choose the requester’s server above, then verify. If it is not
              listed, add it in Settings first.
            </p>
          </Notice>
        ) : !row.verified ? (
          <Notice severity="crit" title="Cannot verify this request">
            <p>
              {row.error ??
                `The requester’s identity does not match what ${row.remote && remote ? remote : serverLabel} reports.`}{' '}
              {row.remote
                ? 'Verify again. If it still does not match, reject it.'
                : 'Refresh. If it still does not match, reject it.'}
            </p>
          </Notice>
        ) : null}
        <div className="btns">
          {row.verified ? (
            <>
              <select
                className="role-select"
                aria-label={`Role for ${name}`}
                value={role}
                disabled={busy || Boolean(row.remote)}
                title={
                  row.remote
                    ? 'Teams from other servers join as Member.'
                    : undefined
                }
                onChange={(e) =>
                  setRoles((all) => ({
                    ...all,
                    [id]: e.target.value as InvitationGrantRole,
                  }))
                }
              >
                <option value="member">Member</option>
                {row.remote ? null : <option value="admin">Admin</option>}
                {row.remote ? null : <option value="owner">Owner</option>}
              </select>
              <Button
                size="sm"
                variant="primary"
                disabled={busy || (Boolean(row.remote) && !remote)}
                onClick={() => void approve(row)}
              >
                Approve
              </Button>
            </>
          ) : null}
          {row.remote ? (
            <Button
              size="sm"
              icon="shield"
              disabled={busy || !remote}
              onClick={() => void verify(row)}
            >
              {row.verified ? 'Verify again' : 'Verify'}
            </Button>
          ) : null}
          <Button
            size="sm"
            danger
            disabled={busy}
            onClick={() => void reject(row)}
          >
            Reject
          </Button>
        </div>
      </article>
    );
  };

  const operationRow = (row: InvitationRow): ReactNode => {
    const note = row.operation_id
      ? invitationLabel(profile, account, row.operation_id)
      : undefined;
    return (
      <article key={row.operation_id} className="op">
        <p>
          <b>{note?.label || 'Untitled invitation'}</b>
        </p>
        <p className="fn">
          {operationStateText(row.state)}
          {note ? ` · ${relativeTime(note.created)}` : ''}
          {note ? ` · ${grantRoleName(note.role)} on approval` : ''}
        </p>
        <div className="btns">
          {row.state === 'prepared' ? (
            <>
              <Button
                size="sm"
                variant="primary"
                disabled={busy}
                onClick={() => void finish(row)}
              >
                Finish creating
              </Button>
              <Button
                size="sm"
                disabled={busy}
                onClick={() => void discard(row)}
              >
                Discard
              </Button>
            </>
          ) : (
            <Button
              size="sm"
              variant="primary"
              disabled={busy}
              onClick={() => void check(row)}
            >
              Check status
            </Button>
          )}
        </div>
      </article>
    );
  };

  return (
    <section className="pcard requests" aria-label="Membership requests">
      {error ? (
        <p role="alert" className="crit">
          {error}
        </p>
      ) : null}
      {needPin ? (
        <Inset className="form">
          <InsetRow label="Security key PIN">
            <input
              type="password"
              autoComplete="off"
              maxLength={32}
              value={pin}
              disabled={busy}
              onChange={(e) => setPin(e.target.value)}
            />
          </InsetRow>
        </Inset>
      ) : null}
      <SectionLabel
        action={
          <Button
            size="sm"
            icon="refresh"
            disabled={busy}
            onClick={() => void run(() => load(true, pin || null), false)}
          >
            Refresh
          </Button>
        }
      >
        Membership requests
      </SectionLabel>
      {truncated ? (
        <p role="status">
          Showing {rows.length} {rows.length === 1 ? 'request' : 'requests'};
          more may be pending. Process these requests and refresh to see more.
        </p>
      ) : null}
      {rows.length ? (
        rows.map(requestRow)
      ) : (
        <div className="callout">
          <span className="kico neutral">
            <Icon name="door" />
          </span>
          <span className="t">
            <b>No requests yet.</b>
            Requests appear here when someone uses an invitation.
          </span>
          <Button icon="door" onClick={onInvite}>
            Invite new user…
          </Button>
        </div>
      )}
      <SectionLabel
        action={
          <button type="button" className="lnk" onClick={onActivity}>
            See all
          </button>
        }
      >
        Invitations you created
      </SectionLabel>
      {operations.length ? (
        operations.map(operationRow)
      ) : (
        <div className="callout">
          <span className="kico neutral">
            <Icon name="check" />
          </span>
          <span className="t">
            <b>{loaded ? 'No in-progress invitations.' : 'Loading…'}</b>
          </span>
        </div>
      )}
    </section>
  );
}
