import {
  groupDetailReadiness,
  combineReadiness,
  itemsReadiness,
} from '../model';
import { CollectionStatus } from '../components/collection-status';
import { useEffect, useRef, type ReactNode } from 'react';
import {
  Button,
  Chip,
  Mark,
  PropertyGroup,
  PropertyRow,
  StoreMark,
} from '../components';
import { inPopup } from '/kit/overlay-primitives';
import type { Bridge } from '../bridge';
import type { Location, GroupSettingsTab } from '../location';
import {
  groupDetailFailure,
  hue,
  isMachine,
  partiesOf,
  partyName,
  parseRole,
  plural,
  roleChipLabel,
  roleName,
  roleRank,
  serverDisplayLabelForStore,
  shortId,
  storeDescription,
  storeOperationAvailability,
} from '../model';
import type { AgentSnapshot, Party, TeamStore } from '../model';
import { useSidebarInbox } from '../chat/inbox-provider';
import { channelTitle, listChannels } from '../chat/presentation';
import { federatedTeamMembers, memberCountOf } from './team-members';
import { itemCountOf } from './group-tabs';
import { manageReason } from './group-model';
import { AccountMark } from './account-switcher';

/** A member row, as the details column's Sharing roster draws one. */
function MemberRow({ party }: { party: Party }): ReactNode {
  const name = partyName(party);
  const role = parseRole(party.destination_role);
  const team = party.party_kind !== 'user' && !isMachine(party);
  return (
    <div className="party">
      {party.party_kind === 'user' ? (
        <AccountMark name={name} />
      ) : (
        <Mark hue={hue(name)} size="md">
          {name.slice(0, 1).toUpperCase()}
        </Mark>
      )}
      <span className="nm">
        <span className="t">{name}</span>
        {party.label === 'you' ? <Chip tone="you">you</Chip> : null}
        {isMachine(party) ? <Chip>Machine</Chip> : null}
        {team ? <Chip>Team</Chip> : null}
      </span>
      <Chip>{role ? roleChipLabel(role) : 'Role unavailable'}</Chip>
    </div>
  );
}

/**
 * A read-only summary of a team, drawn in the Files details column (opened
 * from a folder's or store's Team Info) with the column's own parts: the
 * hero, property rows with their actions at the trailing edge, and the
 * Sharing roster's member rows. Membership changes keep their existing
 * guarded pages.
 */
export function TeamInfoPanel({
  snapshot,
  store,
  bridge,
  requestCount,
  onNavigate,
  onClose,
  onError,
  accessNow = () => Date.now() / 1000,
}: {
  snapshot: AgentSnapshot;
  store: TeamStore;
  bridge: Bridge;
  requestCount?: import('../operation-queries').TeamRequestCount;
  onNavigate: (location: Location) => void;
  onClose: (restoreFocus?: boolean) => void;
  onError: (error: unknown) => void;
  accessNow?: () => number;
}): ReactNode {
  const panel = useRef<HTMLElement>(null);
  const inbox = useSidebarInbox();
  useEffect(() => {
    panel.current?.focus({ preventScroll: true });
  }, [store.id]);
  useEffect(() => {
    const dismiss = (event: PointerEvent) => {
      const target = event.target;
      if (
        event.button !== 0 ||
        !(target instanceof Element) ||
        panel.current?.contains(target) ||
        target.closest('[role="dialog"], [role="alertdialog"]') ||
        inPopup(target)
      )
        return;
      onClose(false);
    };
    document.addEventListener('pointerdown', dismiss);
    return () => document.removeEventListener('pointerdown', dismiss);
  }, [onClose]);
  const options = { nowSeconds: accessNow() };
  const readable = storeOperationAvailability(
    snapshot,
    store,
    'teams',
    options,
  ).available;
  const rosterFailure = groupDetailFailure(snapshot, store.id, 'roster');
  const federationFailure = groupDetailFailure(
    snapshot,
    store.id,
    'federation',
  );
  const fileReadiness = itemsReadiness(snapshot, store.id);
  const rosterReadiness = groupDetailReadiness(snapshot, store.id, 'roster');
  const memberReadiness = combineReadiness([
    rosterReadiness,
    groupDetailReadiness(snapshot, store.id, 'federation'),
  ]);
  const parties =
    readable && !rosterFailure
      ? [...partiesOf(snapshot, store.id)].sort(
          (a, b) => roleRank(b.destination_role) - roleRank(a.destination_role),
        )
      : [];
  const mine = parties.find((party) => party.label === 'you');
  const role = mine && parseRole(mine.destination_role);
  const owners = parties.filter(
    (party) => parseRole(party.destination_role)?.kind === 'owner',
  );
  const account = snapshot.accounts.find(
    (candidate) =>
      candidate.alias === store.account && candidate.server === store.server,
  );
  const federated = federatedTeamMembers(snapshot, store);
  const memberRows = [
    ...parties.filter((party) => party.party_kind === 'user'),
    ...(federationFailure
      ? []
      : [...federated.unmatched, ...federated.ambiguous]),
  ];
  const entry = inbox.get(store.id);
  const chatReadable = storeOperationAvailability(
    snapshot,
    store,
    'chat',
    options,
  ).available;
  const channels =
    chatReadable && entry?.data
      ? listChannels(entry.data.channels, entry.data.conversations)
      : undefined;
  const navigate = (location: Location) => {
    onClose(false);
    onNavigate(location);
  };
  const teamTab = (tab: GroupSettingsTab) =>
    navigate({ kind: 'group-settings', ref: store.id, tab });
  return (
    <aside
      ref={panel}
      className="details team-info"
      aria-label="Team info"
      tabIndex={-1}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && !event.defaultPrevented) {
          event.preventDefault();
          event.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="bar" data-tauri-drag-region="">
        <b className="bar-title">Team Info</b>
        <span className="sp" />
        <Button
          variant="quiet"
          icon="close"
          aria-label="Close team info"
          onClick={() => onClose()}
        />
      </div>
      <div className="scroll">
        <div className="hero">
          <StoreMark snapshot={snapshot} store={store} size="lg" />
          <div className="t">
            <h2 title={store.name}>{store.name}</h2>
            <p>
              {store.team_kind === 'adhoc' ? 'Shared folder' : 'Team'} ·{' '}
              {serverDisplayLabelForStore(snapshot, store)}
            </p>
          </div>
          <div className="hero-acts">
            <Button onClick={() => teamTab('settings')}>Team Settings</Button>
          </div>
        </div>
        {!readable ? (
          <p className="note" role="status">
            {storeDescription(snapshot, store, { operation: 'teams' })}
          </p>
        ) : null}
        <PropertyGroup>
          <PropertyRow label="Server">
            {serverDisplayLabelForStore(snapshot, store)}
          </PropertyRow>
          <PropertyRow label="Your account">
            <span>{account?.username ?? store.account}</span>
            {role ? <Chip>{roleName(role)}</Chip> : null}
          </PropertyRow>
          {readable && !rosterFailure ? (
            <PropertyRow label={owners.length === 1 ? 'Owner' : 'Owners'}>
              {owners.length
                ? owners.map(partyName).join(', ')
                : rosterReadiness === 'loading'
                  ? 'Loading owner…'
                  : rosterReadiness === 'unavailable'
                    ? 'Owner unavailable'
                    : 'No owner designated'}
            </PropertyRow>
          ) : null}
          {readable ? (
            <PropertyRow label="Joining">
              {store.team_kind === 'adhoc'
                ? 'Fixed membership'
                : 'Invite only. An administrator adds members or approves requests.'}
            </PropertyRow>
          ) : null}
          <PropertyRow
            label="Team ID"
            mono
            actions={
              <Button
                size="sm"
                aria-label="Copy Team ID"
                title="Copy Team ID"
                onClick={() => {
                  void bridge.copyText(store.team_id_hex).catch(onError);
                }}
              >
                Copy
              </Button>
            }
          >
            <span title={store.team_id_hex}>{shortId(store.team_id_hex)}</span>
          </PropertyRow>
        </PropertyGroup>
        {readable ? (
          <>
            <PropertyGroup
              className="titled"
              label={
                <>
                  <h3>
                    Members
                    {!rosterFailure &&
                    !federationFailure &&
                    memberReadiness === 'ready'
                      ? ` · ${memberCountOf(snapshot, store)}`
                      : ''}
                  </h3>
                  <Button
                    size="sm"
                    aria-label="Manage Members"
                    onClick={() => teamTab('people')}
                  >
                    Manage…
                  </Button>
                </>
              }
            >
              {!rosterFailure && !federationFailure ? (
                <CollectionStatus state={memberReadiness} label="members" />
              ) : null}
              {rosterFailure ? (
                <p className="note" role="alert">
                  {rosterFailure.message}
                </p>
              ) : (
                <div className="roster">
                  {memberRows.map((party) => (
                    <MemberRow key={party.party_id_hex} party={party} />
                  ))}
                  {!federationFailure &&
                    federated.entries.map((entry) => {
                      const memberRole = parseRole(entry.destination);
                      return (
                        <div
                          className="party"
                          key={`${entry.remote_host_id_hex}:${entry.remote_team_id_hex}`}
                        >
                          <Mark hue={hue(entry.remote_team_alias)} size="md">
                            {entry.remote_team_alias.slice(0, 1).toUpperCase()}
                          </Mark>
                          <span className="nm">
                            <span className="t">{entry.remote_team_alias}</span>
                            <Chip>Team on {entry.remote_profile}</Chip>
                          </span>
                          <Chip>
                            {!entry.active
                              ? 'Inactive'
                              : memberRole
                                ? roleChipLabel(memberRole)
                                : 'Role unavailable'}
                          </Chip>
                        </div>
                      );
                    })}
                </div>
              )}
              {!rosterFailure &&
              !memberRows.length &&
              !federated.entries.length &&
              !federationFailure &&
              memberReadiness === 'ready' ? (
                <p className="note">No members yet.</p>
              ) : null}
              {federationFailure &&
              federationFailure.message !== rosterFailure?.message ? (
                <p className="note" role="alert">
                  {federationFailure.message}
                </p>
              ) : null}
            </PropertyGroup>
            <PropertyGroup
              className="titled"
              label={
                <>
                  <h3>Channels{channels ? ` · ${channels.length}` : ''}</h3>
                  <Button
                    size="sm"
                    aria-label="View Channels"
                    onClick={() => teamTab('channels')}
                  >
                    View…
                  </Button>
                </>
              }
            >
              {store.team_kind === 'adhoc' ? (
                <p className="note">Chat is not available in shared folders.</p>
              ) : !chatReadable ? (
                <p className="note">
                  {storeDescription(snapshot, store, { operation: 'chat' })}
                </p>
              ) : (
                <>
                  {entry?.error || entry?.note ? (
                    <p className="note" role="status">
                      {entry.error || entry.note}
                    </p>
                  ) : null}
                  {!channels ? (
                    <p className="note">
                      {entry?.state === 'blocked' ||
                      entry?.state === 'unavailable'
                        ? 'Channels unavailable.'
                        : 'Loading channels…'}
                    </p>
                  ) : channels.length ? (
                    channels.map(({ channel }) => (
                      <PropertyRow
                        key={channel.id}
                        label={
                          <button
                            type="button"
                            className="link"
                            onClick={() =>
                              navigate({
                                kind: 'chat',
                                ref: store.id,
                                channel: channel.id,
                              })
                            }
                          >
                            {channelTitle(channel)}
                          </button>
                        }
                      >
                        <span className="muted">
                          {channel.admin
                            ? 'Admins and owners'
                            : channel.description}
                        </span>
                      </PropertyRow>
                    ))
                  ) : (
                    <p className="note">No channels yet.</p>
                  )}
                </>
              )}
            </PropertyGroup>
            <PropertyGroup>
              <PropertyRow
                label="Files"
                actions={
                  <Button
                    size="sm"
                    aria-label="Open Team Files"
                    onClick={() => navigate({ kind: 'store', ref: store.id })}
                  >
                    Open
                  </Button>
                }
              >
                {fileReadiness === 'loading'
                  ? 'Loading files…'
                  : storeOperationAvailability(
                        snapshot,
                        store,
                        'vault',
                        options,
                      ).available
                    ? plural(itemCountOf(snapshot, store), 'item')
                    : 'Files unavailable.'}
              </PropertyRow>
              {store.team_kind === 'named' &&
              manageReason(snapshot, store, 'roster') === undefined ? (
                <PropertyRow
                  label="Requests"
                  actions={
                    <Button
                      size="sm"
                      aria-label="Review Requests"
                      onClick={() => teamTab('requests')}
                    >
                      Review…
                    </Button>
                  }
                >
                  {requestCount === undefined
                    ? 'Request count unavailable.'
                    : `${requestCount} pending ${requestCount === 1 ? 'request' : 'requests'}`}
                </PropertyRow>
              ) : null}
            </PropertyGroup>
          </>
        ) : null}
      </div>
    </aside>
  );
}
