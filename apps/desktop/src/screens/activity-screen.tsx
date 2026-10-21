/**
 * Activity: what waits on the reader across every account on this Mac, as a
 * list beside the selected entry's details.
 *
 * "Waiting for you" holds membership requests: the inbox of every team this
 * Mac can manage whose request count is above zero. They are read and acted
 * on through `useMembershipInbox`, the same hook and commands as the team
 * page's Requests tab, so an approval here is the approval there.
 *
 * "Needs attention" holds the notices `activityNotices` derives from the
 * snapshot: a team whose setup never finished, an inactive federated
 * admission, a server that is unverified or whose check-in lapsed. It also
 * lists a team with unfinished membership work, from the counts its page's
 * reads left in the shared repository (`useCachedUnfinishedCounts`); a team
 * not read since unlock is not listed. Each notice's action is the one the
 * place that owns it runs (Finish Setup and Restore Access through the
 * team's operation controller, the team's Unfinished activity sheet, the
 * server's own page for trust).
 *
 * Synchronization status is not repeated here; the sidebar's status line
 * owns it.
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import type { Bridge } from '../bridge';
import {
  Band,
  Button,
  Chip,
  EmptyState,
  Icon,
  PropertyGroup,
  PropertyRow,
  StoreMark,
} from '../components';
import { useMembershipInbox } from '../components/membership-inbox';
import type { MembershipInbox } from '../components/membership-inbox';
import {
  InboxPin,
  RequestActions,
  RequestChecks,
  RequesterMark,
} from '../components/membership-requests';
import { activityNoticeText, activityNotices } from '../activity';
import type { ActivityNotice } from '../activity';
import type { InvitationRow } from '../invitation-contract';
import type { Location } from '../location';
import {
  federatedTeamName,
  parseRole,
  partiesOf,
  roleName,
  serverDisplayLabel,
  serverDisplayLabelForStore,
  serverOf,
} from '../model';
import type { AgentSnapshot, TeamStore } from '../model';
import type { MutationFailureHandler } from '../mutation-recovery';
import {
  useCachedUnfinishedCounts,
  useTeamRequestCounts,
} from '../operation-queries';
import { InvitationActivitySheet } from '../components/invitation-activity-sheet';
import { unfinishedActivityText } from '../components/unfinished-activity';
import { DetailsColumn } from '../shell/page-slots';
import { useToast } from '/kit/toasts';
import { useGroupOperationController } from './groups/operation-controller';
import { accessCopy } from './store-access';

type Entry =
  | {
      kind: 'request';
      id: string;
      team: TeamStore;
      row: InvitationRow;
      title: string;
      detail: string;
    }
  | {
      kind: 'notice';
      id: string;
      notice: ActivityNotice;
      title: string;
      detail: string;
    };

export interface ActivityScreenProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  /** The shell's search field, which filters the list. */
  query: string;
  onNavigate: (location: Location) => void;
  onApplied: (message: string, profile?: string) => Promise<void>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
}

/** The account a team belongs to, and the role it holds there. */
function actingAs(snapshot: AgentSnapshot, team: TeamStore): string {
  const username =
    snapshot.accounts.find(
      (candidate) =>
        candidate.alias === team.account && candidate.server === team.server,
    )?.username ?? team.account;
  const mine = partiesOf(snapshot, team.id).find(
    (party) => party.label === 'you',
  );
  const role = mine ? parseRole(mine.destination_role) : null;
  return role ? `${username} · ${roleName(role)}` : username;
}

function TeamValue({
  snapshot,
  team,
}: {
  snapshot: AgentSnapshot;
  team: TeamStore;
}): ReactNode {
  return (
    <>
      <StoreMark snapshot={snapshot} store={team} />
      {team.name}
    </>
  );
}

function ServerValue({
  snapshot,
  team,
}: {
  snapshot: AgentSnapshot;
  team: TeamStore;
}): ReactNode {
  const server = serverOf(snapshot, team.id);
  return (
    <>
      {serverDisplayLabelForStore(snapshot, team)}
      {server && serverDisplayLabel(server) !== server.configuredEndpoint ? (
        <span className="dim">{server.configuredEndpoint}</span>
      ) : null}
    </>
  );
}

function RequestDetail({
  snapshot,
  inbox,
  team,
  row,
  onNavigate,
}: {
  snapshot: AgentSnapshot;
  inbox: MembershipInbox;
  team: TeamStore;
  row: InvitationRow;
  onNavigate: (location: Location) => void;
}): ReactNode {
  const name = row.username ?? 'Unknown user';
  const serverLabel = serverDisplayLabelForStore(snapshot, team);
  return (
    <>
      <div className="hero">
        <RequesterMark name={name} large />
        <div className="t">
          <h2>{name}</h2>
          <p>Membership request · {team.name}</p>
        </div>
      </div>
      {inbox.error ? (
        <p role="alert" className="crit">
          {inbox.error}
        </p>
      ) : null}
      <InboxPin inbox={inbox} />
      <PropertyGroup>
        <PropertyRow label="Requester">
          {name}
          {row.joiner_kind === 'team' ? <Chip>Team</Chip> : null}
          {row.verified ? (
            <Chip tone="ok">
              <Icon name="check" />
              Verified
            </Chip>
          ) : null}
        </PropertyRow>
        <PropertyRow
          label="Team"
          actions={
            <Button
              onClick={() =>
                onNavigate({
                  kind: 'group-settings',
                  ref: team.id,
                  tab: 'requests',
                })
              }
            >
              Open Team
            </Button>
          }
        >
          <TeamValue snapshot={snapshot} team={team} />
        </PropertyRow>
        <PropertyRow label="Server">
          <ServerValue snapshot={snapshot} team={team} />
        </PropertyRow>
        <PropertyRow label="You act as">{actingAs(snapshot, team)}</PropertyRow>
      </PropertyGroup>
      {row.remote || !row.verified ? (
        <div className="request-checks">
          <RequestChecks
            inbox={inbox}
            team={team}
            row={row}
            servers={snapshot.servers}
            serverLabel={serverLabel}
          />
        </div>
      ) : null}
      <PropertyGroup>
        <PropertyRow
          label="Admit as"
          className="admit"
          actions={<RequestActions inbox={inbox} team={team} row={row} />}
        />
      </PropertyGroup>
    </>
  );
}

/**
 * A notice about one team, acted on through that team's own operation
 * controller: the same Finish Setup and Restore Access the team page runs.
 */
function TeamNoticeDetail({
  snapshot,
  bridge,
  notice,
  text,
  onNavigate,
  onApplied,
  onError,
  onMutationError,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  notice: Extract<
    ActivityNotice,
    { kind: 'setup-incomplete' | 'federation-inactive' }
  >;
  text: { title: string; detail: string };
  onNavigate: (location: Location) => void;
  onApplied: (message: string, profile?: string) => Promise<void>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
}): ReactNode {
  const toasts = useToast();
  const team = notice.store;
  const controller = useGroupOperationController({
    bridge,
    store: team,
    enabled: false,
    onSnapshotApplied: onApplied,
    onSnapshotMutationError: onMutationError,
    onRefreshError: (error) => {
      toasts.show('Change applied. Refresh pending.');
      onError(error);
    },
    onReadError: onError,
  });
  const serverName = serverDisplayLabelForStore(snapshot, team);
  const copy =
    notice.kind === 'setup-incomplete'
      ? accessCopy('setup-incomplete', team, serverName)
      : null;
  const operation =
    notice.kind === 'federation-inactive'
      ? notice.entry.operation_id_hex
      : undefined;
  return (
    <>
      <div className="hero">
        <span className="bigtile warn" aria-hidden="true">
          <Icon name="warning" />
        </span>
        <div className="t">
          <h2>{text.title}</h2>
          <p>{text.detail}</p>
        </div>
      </div>
      <p className="note">
        {notice.kind === 'federation-inactive'
          ? `${federatedTeamName(snapshot, notice.entry)}’s membership in ${team.name} is inactive. ${federatedTeamName(snapshot, notice.entry)} members cannot access ${team.name} until it is restored.`
          : copy?.detail}
      </p>
      <PropertyGroup>
        <PropertyRow
          label="Team"
          actions={
            <Button
              onClick={() =>
                onNavigate({
                  kind: 'group-settings',
                  ref: team.id,
                  tab: 'people',
                })
              }
            >
              Open Team
            </Button>
          }
        >
          <TeamValue snapshot={snapshot} team={team} />
        </PropertyRow>
        <PropertyRow label="Server">
          <ServerValue snapshot={snapshot} team={team} />
        </PropertyRow>
        <PropertyRow
          label=""
          actions={
            notice.kind === 'setup-incomplete' ? (
              <Button
                variant="primary"
                disabled={copy?.actionDisabled}
                onClick={controller.resumeCreation}
              >
                Finish Setup
              </Button>
            ) : operation ? (
              <Button
                variant="primary"
                onClick={() => controller.resumeFederatedMemberAdd(operation)}
              >
                Restore Access
              </Button>
            ) : null
          }
        />
      </PropertyGroup>
    </>
  );
}

/**
 * A team's unfinished membership work: the count its Unfinished activity
 * band shows, and that band's sheet, opened here over the team's own
 * operation controller so resuming an operation is the same write.
 */
function UnfinishedDetail({
  snapshot,
  bridge,
  notice,
  text,
  onNavigate,
  onApplied,
  onError,
  onMutationError,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  notice: Extract<ActivityNotice, { kind: 'unfinished' }>;
  text: { title: string; detail: string };
  onNavigate: (location: Location) => void;
  onApplied: (message: string, profile?: string) => Promise<void>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
}): ReactNode {
  const toasts = useToast();
  const team = notice.store;
  const [reviewing, setReviewing] = useState(false);
  // The journal this entry was counted from is already cached; reading it
  // here keeps the sheet's membership rows current while it is open.
  const controller = useGroupOperationController({
    bridge,
    store: team,
    enabled: true,
    onSnapshotApplied: onApplied,
    onSnapshotMutationError: onMutationError,
    onRefreshError: (error) => {
      toasts.show('Change applied. Refresh pending.');
      onError(error);
    },
    onReadError: onError,
  });
  return (
    <>
      <div className="hero">
        <span className="bigtile warn" aria-hidden="true">
          <Icon name="warning" />
        </span>
        <div className="t">
          <h2>{text.title}</h2>
          <p>{text.detail}</p>
        </div>
      </div>
      <p className="note">
        {unfinishedActivityText(notice.count, controller.operations.length)}
      </p>
      <PropertyGroup>
        <PropertyRow
          label="Team"
          actions={
            <Button
              onClick={() =>
                onNavigate({
                  kind: 'group-settings',
                  ref: team.id,
                  tab: 'people',
                })
              }
            >
              Open Team
            </Button>
          }
        >
          <TeamValue snapshot={snapshot} team={team} />
        </PropertyRow>
        <PropertyRow label="Server">
          <ServerValue snapshot={snapshot} team={team} />
        </PropertyRow>
        <PropertyRow
          label=""
          actions={
            <Button variant="primary" onClick={() => setReviewing(true)}>
              Review…
            </Button>
          }
        />
      </PropertyGroup>
      {reviewing ? (
        <InvitationActivitySheet
          bridge={bridge}
          team={team}
          membership={{
            operations: controller.operations,
            onResume: controller.resumeMembership,
          }}
          onClose={() => setReviewing(false)}
          onComplete={() => onApplied('Team requests updated', team.server)}
        />
      ) : null}
    </>
  );
}

function ServerNoticeDetail({
  notice,
  text,
  onNavigate,
}: {
  notice: Extract<ActivityNotice, { kind: 'server' }>;
  text: { title: string; detail: string };
  onNavigate: (location: Location) => void;
}): ReactNode {
  const lapsed = notice.reason === 'check-in-expired';
  return (
    <>
      <div className="hero">
        <span
          className={lapsed ? 'bigtile bad' : 'bigtile warn'}
          aria-hidden="true"
        >
          <Icon name={lapsed ? 'lock' : 'warning'} />
        </span>
        <div className="t">
          <h2>{text.title}</h2>
          <p>{text.detail}</p>
        </div>
      </div>
      <p className="note">
        {lapsed
          ? 'This server’s compatibility check-in expired. Stores on it are locked until a new check-in is read from its signed status.'
          : 'This server has not been verified. Check reads its signed status and pins its identity.'}
      </p>
      <PropertyGroup>
        <PropertyRow label="Server">
          {serverDisplayLabel(notice.server)}
          {serverDisplayLabel(notice.server) !==
          notice.server.configuredEndpoint ? (
            <span className="dim">{notice.server.configuredEndpoint}</span>
          ) : null}
        </PropertyRow>
        <PropertyRow
          label=""
          actions={
            <Button
              variant="primary"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'account',
                  profile: notice.server.profileName,
                })
              }
            >
              Open Server
            </Button>
          }
        />
      </PropertyGroup>
    </>
  );
}

export function ActivityScreen({
  snapshot,
  bridge,
  query,
  onNavigate,
  onApplied,
  onError,
  onMutationError,
}: ActivityScreenProps): ReactNode {
  // The same shared request counts the sidebar and the Teams list read; a
  // team's inbox is read only once it has something in it.
  const counts = useTeamRequestCounts(bridge, snapshot, onError);
  const requestTeams = useMemo(
    () =>
      snapshot.stores.filter(
        (store): store is TeamStore =>
          store.kind === 'team' &&
          store.team_kind === 'named' &&
          Boolean(counts.get(store.id)) &&
          counts.get(store.id) !== 0,
      ),
    [snapshot.stores, counts],
  );
  const teamStores = useMemo(
    () =>
      snapshot.stores.filter(
        (store): store is TeamStore => store.kind === 'team',
      ),
    [snapshot.stores],
  );
  const unfinished = useCachedUnfinishedCounts(bridge, teamStores);
  const inbox = useMembershipInbox(bridge, requestTeams, {
    onApproved: (team) => onApplied('Team requests updated', team.server),
  });
  const entries: Entry[] = [
    ...requestTeams.flatMap((team) =>
      inbox.entry(team).rows.map((row): Entry => ({
        kind: 'request',
        id: `request:${team.id}:${row.request_id}`,
        team,
        row,
        title: row.username ?? 'Unknown user',
        detail: `Wants to join ${team.name} · ${serverDisplayLabelForStore(snapshot, team)}`,
      })),
    ),
    ...activityNotices(snapshot, unfinished).map((notice): Entry => ({
      kind: 'notice',
      id: notice.id,
      notice,
      ...activityNoticeText(snapshot, notice),
    })),
  ];
  const needle = query.trim().toLowerCase();
  const visible = needle
    ? entries.filter((entry) =>
        `${entry.title} ${entry.detail}`.toLowerCase().includes(needle),
      )
    : entries;
  const loadingRequests = requestTeams.some(
    (team) => inbox.entry(team).state === 'loading',
  );
  const [chosen, setChosen] = useState<string | null>(null);
  // A resolved entry leaves the list; the selection moves to the first one
  // left rather than pointing at nothing.
  const selected =
    visible.find((entry) => entry.id === chosen) ?? visible[0] ?? null;
  const listRef = useRef<HTMLDivElement>(null);
  const focusRow = useRef(false);
  useEffect(() => {
    if (!focusRow.current) return;
    focusRow.current = false;
    listRef.current
      ?.querySelector<HTMLElement>('[role="option"][aria-selected="true"]')
      ?.focus();
  });
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    const index = visible.findIndex((entry) => entry.id === selected?.id);
    const next =
      visible[
        Math.min(
          visible.length - 1,
          Math.max(0, index + (event.key === 'ArrowDown' ? 1 : -1)),
        )
      ];
    if (!next) return;
    event.preventDefault();
    focusRow.current = true;
    setChosen(next.id);
  };
  const row = (entry: Entry): ReactNode => {
    const on = entry.id === selected?.id;
    return (
      <button
        key={entry.id}
        type="button"
        role="option"
        aria-selected={on}
        tabIndex={on ? 0 : -1}
        className={on ? 'row act sel' : 'row act'}
        onClick={() => setChosen(entry.id)}
      >
        <span
          className={
            entry.kind === 'notice' &&
            entry.notice.kind === 'server' &&
            entry.notice.reason === 'check-in-expired'
              ? 'act-dot bad'
              : 'act-dot'
          }
          aria-hidden="true"
        />
        <span className="name">
          <b>{entry.title}</b>
          <small>{entry.detail}</small>
        </span>
      </button>
    );
  };
  const requests = visible.filter((entry) => entry.kind === 'request');
  const notices = visible.filter((entry) => entry.kind === 'notice');
  return (
    <>
      <div className="activity-list">
        {visible.length || loadingRequests ? (
          <div
            ref={listRef}
            className="rows"
            role="listbox"
            aria-label="Activity"
            onKeyDown={onKeyDown}
          >
            {requests.length || loadingRequests ? (
              <>
                <div className="gsec" role="presentation">
                  Waiting for you
                </div>
                {requests.map(row)}
                {loadingRequests ? (
                  <p className="act-status" role="status">
                    Loading requests…
                  </p>
                ) : null}
              </>
            ) : null}
            {notices.length ? (
              <>
                <div className="gsec" role="presentation">
                  Needs attention
                </div>
                {notices.map(row)}
              </>
            ) : null}
          </div>
        ) : (
          <EmptyState
            inList
            title={needle ? 'No Results' : 'Nothing Waiting'}
            body={
              needle
                ? undefined
                : 'Membership requests and notices that need you appear here.'
            }
          />
        )}
        {inbox.error && selected?.kind !== 'request' ? (
          <Band severity="crit">{inbox.error}</Band>
        ) : null}
      </div>
      <DetailsColumn>
        <div className="bar" data-tauri-drag-region="" />
        {selected ? (
          <div className="scroll activity-detail" key={selected.id}>
            {selected.kind === 'request' ? (
              <RequestDetail
                snapshot={snapshot}
                inbox={inbox}
                team={selected.team}
                row={selected.row}
                onNavigate={onNavigate}
              />
            ) : selected.notice.kind === 'server' ? (
              <ServerNoticeDetail
                notice={selected.notice}
                text={selected}
                onNavigate={onNavigate}
              />
            ) : selected.notice.kind === 'unfinished' ? (
              <UnfinishedDetail
                snapshot={snapshot}
                bridge={bridge}
                notice={selected.notice}
                text={selected}
                onNavigate={onNavigate}
                onApplied={onApplied}
                onError={onError}
                onMutationError={onMutationError}
              />
            ) : (
              <TeamNoticeDetail
                snapshot={snapshot}
                bridge={bridge}
                notice={selected.notice}
                text={selected}
                onNavigate={onNavigate}
                onApplied={onApplied}
                onError={onError}
                onMutationError={onMutationError}
              />
            )}
          </div>
        ) : (
          <div className="empty">
            <b>No Selection</b>
          </div>
        )}
      </DetailsColumn>
    </>
  );
}
