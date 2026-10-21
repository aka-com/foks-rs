import { CollectionStatus } from './collection-status';
import type { CollectionReadiness } from '../model';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import { INVITATION_ACTIVITY } from '../invitation-activity';
import type { InvitationActivityDetail } from '../invitation-activity';
import type { InvitationRow } from '../invitation-contract';
import {
  forgetInvitationLabel,
  invitationLabel,
  relativeTime,
} from '../invitation-labels';
import type { InvitationGrantRole } from '../invitation-labels';
import {
  grantRoleName,
  invitationRows,
  operationStateText,
  unfinishedOperation,
} from '../invitation-writes';
import type { TeamStore } from '../model';
import { hue, serverDisplayLabel } from '../model';
import type { JoinServer } from './invitation-panel';
import { useMembershipInbox } from './membership-inbox';
import type { MembershipInbox } from './membership-inbox';
import { Button, Chip, Icon, Notice, SectionLabel } from './index';

/** Up to two initials for a requester: "fixture-joiner" is FJ. */
export function requesterInitials(name: string): string {
  return name
    .split(/[._\-\s@]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part.slice(0, 1).toUpperCase())
    .join('');
}

/** The requester's mark: initials on the name's hue. */
export function RequesterMark({
  name,
  large = false,
}: {
  name: string;
  large?: boolean;
}): ReactNode {
  return (
    <span
      className={large ? 'member-mark lg' : 'member-mark'}
      style={{ background: hue(name) }}
      aria-hidden="true"
    >
      {requesterInitials(name)}
    </span>
  );
}

/** Where a request comes from, as its row's caption reads it. */
export function requestOrigin(
  inbox: MembershipInbox,
  row: InvitationRow,
  serverLabel: string,
): string {
  const remote = inbox.remoteProfileOf(row);
  const isTeam = row.joiner_kind === 'team';
  return row.remote
    ? remote
      ? `${isTeam ? 'Team on' : 'on'} ${remote}`
      : 'on another server'
    : `on ${serverLabel}`;
}

/**
 * The parts of a request that can need the reader before Approve: the
 * requester's server for a remote request, and the notice when the
 * requester could not be verified. Shared by the Requests tab and Activity.
 */
export function RequestChecks({
  inbox,
  team,
  row,
  servers,
  serverLabel,
}: {
  inbox: MembershipInbox;
  team: TeamStore;
  row: InvitationRow;
  servers: readonly JoinServer[];
  serverLabel: string;
}): ReactNode {
  const name = row.username ?? 'Unknown user';
  const remote = inbox.remoteProfileOf(row);
  return (
    <>
      {row.remote ? (
        <label className="request-server">
          <span className="k">Server for {name}</span>
          <select
            value={remote ?? ''}
            disabled={inbox.busy}
            onChange={(event) =>
              inbox.setRemoteProfile(row, event.target.value)
            }
          >
            <option value="">Choose the requester’s server</option>
            {servers
              .filter((server) => server.profileName !== team.server)
              .map((server) => (
                <option key={server.profileName} value={server.profileName}>
                  {serverDisplayLabel(server)} ({server.configuredEndpoint})
                </option>
              ))}
          </select>
        </label>
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
    </>
  );
}

/**
 * Admit-as, Reject, Approve and (for a remote request) Verify: the same
 * controls, calling the same actions, on the Requests tab and in Activity.
 */
export function RequestActions({
  inbox,
  team,
  row,
}: {
  inbox: MembershipInbox;
  team: TeamStore;
  row: InvitationRow;
}): ReactNode {
  const name = row.username ?? 'Unknown user';
  const remote = inbox.remoteProfileOf(row);
  return (
    <>
      {row.verified ? (
        <select
          className="role-select"
          aria-label={`Role for ${name}`}
          value={inbox.roleOf(row)}
          disabled={inbox.busy || Boolean(row.remote)}
          title={
            row.remote ? 'Teams from other servers join as Member.' : undefined
          }
          onChange={(e) =>
            inbox.setRole(row, e.target.value as InvitationGrantRole)
          }
        >
          <option value="member">Member</option>
          {row.remote ? null : <option value="admin">Admin</option>}
          {row.remote ? null : <option value="owner">Owner</option>}
        </select>
      ) : null}
      {row.remote ? (
        <Button
          icon="shield"
          disabled={inbox.busy || !remote}
          onClick={() => void inbox.verify(team, row)}
        >
          {row.verified ? 'Verify Again' : 'Verify'}
        </Button>
      ) : null}
      <Button
        disabled={inbox.busy}
        onClick={() => void inbox.reject(team, row)}
      >
        Reject
      </Button>
      {row.verified ? (
        <Button
          variant="primary"
          disabled={inbox.busy || (Boolean(row.remote) && !remote)}
          onClick={() => void inbox.approve(team, row)}
        >
          Approve
        </Button>
      ) : null}
    </>
  );
}

/** The security key PIN field, when an action asked for it. */
export function InboxPin({ inbox }: { inbox: MembershipInbox }): ReactNode {
  if (!inbox.needPin) return null;
  return (
    <div className="group-card request-pin">
      <label className="set-row">
        <span className="k">Security key PIN</span>
        <span className="v">
          <input
            type="password"
            autoComplete="off"
            maxLength={32}
            value={inbox.pin}
            disabled={inbox.busy}
            onChange={(e) => inbox.setPin(e.target.value)}
          />
        </span>
      </label>
    </div>
  );
}

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
  const [teams] = useState(() => [team]);
  const inbox = useMembershipInbox(bridge, teams, {
    onApproved: () => onComplete(),
  });
  const { rows, state: requestState, truncated } = inbox.entry(team);
  const [operations, setOperations] = useState<InvitationRow[]>([]);
  const [operationState, setOperationState] =
    useState<CollectionReadiness>('loading');
  const loadGeneration = useRef(0);
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  const { fail } = inbox;

  // The invitations this account created for the team, read beside the
  // requests; the requests themselves are the shared inbox's.
  const loadOperations = useCallback(async (): Promise<void> => {
    const generation = ++loadGeneration.current;
    setOperationState('loading');
    try {
      const list = await bridge.invitation(
        profile,
        account,
        { action: 'list' },
        null,
      );
      if (!live.current || generation !== loadGeneration.current) return;
      setOperationState('ready');
      setOperations(
        invitationRows(list).filter(
          (row) => row.team_id === team.team_id_hex && unfinishedOperation(row),
        ),
      );
    } catch (failure) {
      if (!live.current || generation !== loadGeneration.current) return;
      setOperationState('unavailable');
      fail(failure);
    }
  }, [bridge, profile, account, team.team_id_hex, fail]);

  useEffect(() => {
    void loadOperations();
  }, [loadOperations]);

  useEffect(() => {
    const refresh = (event: Event) => {
      const scope = (event as CustomEvent<InvitationActivityDetail>).detail;
      if (scope && (scope.profile !== profile || scope.account !== account))
        return;
      void loadOperations();
    };
    window.addEventListener(INVITATION_ACTIVITY, refresh);
    return () => window.removeEventListener(INVITATION_ACTIVITY, refresh);
  }, [loadOperations, profile, account]);

  const finish = (row: InvitationRow) =>
    inbox.run(team, async () => {
      if (!row.operation_id) return;
      const current = await inbox.call(team, {
        action: 'attempt',
        operation_id: row.operation_id,
      });
      if (!live.current || !inbox.hardwareGate(current)) return;
      setOperations((all) =>
        all
          .map((r) => (r.operation_id === row.operation_id ? current : r))
          .filter(unfinishedOperation),
      );
    });

  const check = (row: InvitationRow) =>
    inbox.run(
      team,
      async () => {
        if (!row.operation_id) return;
        const current = await inbox.call(team, {
          action: 'status',
          operation_id: row.operation_id,
        });
        if (!live.current || !inbox.hardwareGate(current)) return;
        setOperations((all) =>
          all
            .map((r) => (r.operation_id === row.operation_id ? current : r))
            .filter(unfinishedOperation),
        );
      },
      false,
    );

  const discard = (row: InvitationRow) =>
    inbox.run(team, async () => {
      if (!row.operation_id) return;
      await inbox.call(team, {
        action: 'cancel',
        operation_id: row.operation_id,
      });
      forgetInvitationLabel(profile, account, row.operation_id);
      if (!live.current) return;
      setOperations((all) =>
        all.filter((r) => r.operation_id !== row.operation_id),
      );
    });

  const requestRow = (row: InvitationRow): ReactNode => {
    const id = row.request_id!;
    const name = row.username ?? 'Unknown user';
    const isTeam = row.joiner_kind === 'team';
    return (
      <article
        key={id}
        className="set-row request-row"
        aria-label={`Request from ${name}`}
      >
        <RequesterMark name={name} />
        <span className="t">
          <b>
            <span>{name}</span>
            {isTeam ? <Chip>Team</Chip> : null}
            {row.remote ? <Chip>Remote</Chip> : null}
          </b>
          <small>
            {requestOrigin(inbox, row, serverLabel)}
            {row.verified ? ' · verified' : ''}
            {row.time ? ` · ${relativeTime(row.time)}` : ''}
          </small>
        </span>
        <span className="a">
          <RequestActions inbox={inbox} team={team} row={row} />
        </span>
        {row.remote || !row.verified ? (
          <div className="request-checks">
            <RequestChecks
              inbox={inbox}
              team={team}
              row={row}
              servers={servers}
              serverLabel={serverLabel}
            />
          </div>
        ) : null}
      </article>
    );
  };

  const operationRow = (row: InvitationRow): ReactNode => {
    const note = row.operation_id
      ? invitationLabel(profile, account, row.operation_id)
      : undefined;
    return (
      <article key={row.operation_id} className="set-row request-row">
        <span className="member-tile" aria-hidden="true">
          <Icon name="door" />
        </span>
        <span className="t">
          <b>
            <span>{note?.label || 'Untitled invitation'}</span>
          </b>
          <small>
            {operationStateText(row.state)}
            {note ? ` · ${relativeTime(note.created)}` : ''}
            {note ? ` · ${grantRoleName(note.role)} on approval` : ''}
          </small>
        </span>
        <span className="a">
          {row.state === 'prepared' ? (
            <>
              <Button disabled={inbox.busy} onClick={() => void discard(row)}>
                Discard
              </Button>
              <Button
                variant="primary"
                disabled={inbox.busy}
                onClick={() => void finish(row)}
              >
                Finish Creating
              </Button>
            </>
          ) : (
            <Button
              variant="primary"
              disabled={inbox.busy}
              onClick={() => void check(row)}
            >
              Check Status
            </Button>
          )}
        </span>
      </article>
    );
  };

  return (
    <section className="requests" aria-label="Membership requests">
      {inbox.error ? (
        <p role="alert" className="crit">
          {inbox.error}
        </p>
      ) : null}
      <InboxPin inbox={inbox} />
      <SectionLabel
        action={
          <Button
            icon="refresh"
            disabled={inbox.busy}
            onClick={() =>
              void inbox.run(
                team,
                () =>
                  Promise.all([
                    inbox.load(team, inbox.pin || null),
                    loadOperations(),
                  ]).then(() => undefined),
                false,
              )
            }
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
      <CollectionStatus state={requestState} label="requests" />
      {rows.length ? (
        <div className="group-card">{rows.map(requestRow)}</div>
      ) : requestState === 'ready' ? (
        <div className="group-card">
          <div className="set-row">
            <span className="member-tile" aria-hidden="true">
              <Icon name="door" />
            </span>
            <span className="t">
              <b>No requests yet.</b>
              <small>
                Requests appear here when someone uses an invitation.
              </small>
            </span>
            <span className="a">
              <Button icon="door" onClick={onInvite}>
                Invite New User…
              </Button>
            </span>
          </div>
        </div>
      ) : null}
      <SectionLabel
        action={
          <button type="button" className="lnk" onClick={onActivity}>
            See All
          </button>
        }
      >
        Invitations you created
      </SectionLabel>
      <CollectionStatus state={operationState} label="invitations" />
      {operations.length ? (
        <div className="group-card">{operations.map(operationRow)}</div>
      ) : operationState === 'ready' ? (
        <div className="group-card">
          <div className="set-row">
            <span className="member-tile" aria-hidden="true">
              <Icon name="check" />
            </span>
            <span className="t">
              <b>No in-progress invitations.</b>
            </span>
          </div>
        </div>
      ) : null}
    </section>
  );
}
