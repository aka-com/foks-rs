import { useEffect, useRef, type ReactNode } from 'react';
import { Button } from '../components';
import type { Bridge } from '../bridge';
import type { Location, GroupSettingsTab } from '../location';
import {
  groupDetailFailure,
  isMachine,
  partiesOf,
  partyName,
  parseRole,
  plural,
  roleName,
  roleRank,
  serverName,
  shortId,
  storeDescription,
  storeOperationAvailability,
} from '../model';
import type { AgentSnapshot, TeamStore } from '../model';
import { useSidebarInbox } from '../chat/inbox-provider';
import { channelTitle, listChannels } from '../chat/presentation';
import { admittedGroups, memberCountOf } from './team-members';
import { itemCountOf } from './group-tabs';
import { manageReason } from './group-model';
import { GroupMark } from './group-mark';
import './team-info.css';

/** A read-only summary; membership changes keep their existing guarded pages. */
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
  requestCount?: number;
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
        target.closest('[role="dialog"], [role="menu"], [role="listbox"]')
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
  const admitted = admittedGroups(snapshot, store);
  const memberRows = [
    ...parties.filter((party) => party.party_kind === 'user'),
    ...(federationFailure
      ? []
      : [...admitted.unmatched, ...admitted.ambiguous]),
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
      className="team-info"
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
      <div className="team-info-head">
        <b>Team info</b>
        <Button
          variant="quiet"
          icon="close"
          aria-label="Close team info"
          onClick={() => onClose()}
        />
      </div>
      <section>
        <div className="team-info-identity">
          <GroupMark store={store} />
          <div>
            <h2>{store.name}</h2>
            <p>{store.team_kind === 'adhoc' ? 'Shared folder' : 'Team'}</p>
          </div>
        </div>
        <dl>
          <dt>Server</dt>
          <dd>{serverName(snapshot, store)}</dd>
          <dt>Your account</dt>
          <dd>
            {account?.username ?? store.account}
            {role ? ` · ${roleName(role)}` : ''}
          </dd>
          {readable && !rosterFailure ? (
            <>
              <dt>{owners.length === 1 ? 'Owner' : 'Owners'}</dt>
              <dd>
                {owners.length
                  ? owners.map(partyName).join(', ')
                  : 'No owner designated'}
              </dd>
            </>
          ) : null}
        </dl>
        {!readable ? (
          <p role="status">
            {storeDescription(snapshot, store, { operation: 'teams' })}
          </p>
        ) : (
          <p className="team-info-quiet">
            {store.team_kind === 'adhoc'
              ? 'An ad-hoc team has fixed membership.'
              : 'Invite only. An administrator adds members or approves requests.'}
          </p>
        )}
      </section>
      {readable ? (
        <>
          <section>
            <h3>
              Members
              {!rosterFailure && !federationFailure
                ? ` · ${memberCountOf(snapshot, store)}`
                : ''}
            </h3>
            {rosterFailure ? (
              <p role="alert">{rosterFailure.message}</p>
            ) : (
              <>
                {memberRows.map((party) => {
                  const memberRole = parseRole(party.destination_role);
                  return (
                    <div className="team-info-member" key={party.party_id_hex}>
                      <span>
                        {partyName(party)}
                        {party.label === 'you' ? ' · You' : ''}
                        {isMachine(party) ? (
                          <small>Machine</small>
                        ) : party.party_kind !== 'user' ? (
                          <small>Team</small>
                        ) : null}
                      </span>
                      <small>
                        {memberRole ? roleName(memberRole) : 'Role unavailable'}
                      </small>
                    </div>
                  );
                })}
                {!federationFailure &&
                  admitted.entries.map((entry) => {
                    const memberRole = parseRole(entry.destination);
                    return (
                      <div
                        className="team-info-member"
                        key={`${entry.remote_host_id_hex}:${entry.remote_team_id_hex}`}
                      >
                        <span>
                          {entry.remote_team_alias}
                          <small>Team on {entry.remote_profile}</small>
                        </span>
                        <small>
                          {entry.active
                            ? memberRole
                              ? roleName(memberRole)
                              : 'Role unavailable'
                            : 'Inactive'}
                        </small>
                      </div>
                    );
                  })}
                {!memberRows.length &&
                !admitted.entries.length &&
                !federationFailure ? (
                  <p>No members yet.</p>
                ) : null}
              </>
            )}
            {federationFailure &&
            federationFailure.message !== rosterFailure?.message ? (
              <p role="alert">{federationFailure.message}</p>
            ) : null}
            <Button icon="users" onClick={() => teamTab('people')}>
              Manage members
            </Button>
          </section>
          <section>
            <h3>Channels{channels ? ` · ${channels.length}` : ''}</h3>
            {store.team_kind === 'adhoc' ? (
              <p>Chat is not available in shared folders.</p>
            ) : !chatReadable ? (
              <p>{storeDescription(snapshot, store, { operation: 'chat' })}</p>
            ) : (
              <>
                {entry?.error || entry?.note ? (
                  <p role="status">{entry.error || entry.note}</p>
                ) : null}
                {!channels ? (
                  <p>
                    {entry?.state === 'blocked' ||
                    entry?.state === 'unavailable'
                      ? 'Channels unavailable.'
                      : 'Loading channels…'}
                  </p>
                ) : channels.length ? (
                  channels.map(({ channel }) => (
                    <button
                      className="team-info-channel"
                      type="button"
                      key={channel.id}
                      onClick={() =>
                        navigate({
                          kind: 'chat',
                          ref: store.id,
                          channel: channel.id,
                        })
                      }
                    >
                      {channelTitle(channel)}
                      <small>
                        {channel.admin
                          ? 'Admins and owners'
                          : channel.description}
                      </small>
                    </button>
                  ))
                ) : (
                  <p>No channels yet.</p>
                )}
              </>
            )}
            <Button onClick={() => teamTab('channels')}>View channels</Button>
          </section>
          <section>
            <h3>Files</h3>
            <p>
              {storeOperationAvailability(snapshot, store, 'vault', options)
                .available
                ? plural(itemCountOf(snapshot, store), 'item')
                : 'Files unavailable.'}
            </p>
            <Button
              icon="folder"
              onClick={() => navigate({ kind: 'store', ref: store.id })}
            >
              Open team files
            </Button>
          </section>
          {store.team_kind === 'named' &&
          manageReason(snapshot, store, 'roster') === undefined ? (
            <section>
              <h3>Requests</h3>
              <p>
                {requestCount === undefined
                  ? 'Request count unavailable.'
                  : `${requestCount} pending ${requestCount === 1 ? 'request' : 'requests'}`}
              </p>
              <Button onClick={() => teamTab('requests')}>
                Review requests
              </Button>
            </section>
          ) : null}
        </>
      ) : null}
      <section>
        <h3>Team ID</h3>
        <code title={store.team_id_hex}>{shortId(store.team_id_hex)}</code>
        <Button
          icon="copy"
          onClick={() => {
            void bridge.copyText(store.team_id_hex).catch(onError);
          }}
        >
          Copy team ID
        </Button>
        <Button icon="settings" onClick={() => teamTab('settings')}>
          Team settings
        </Button>
      </section>
    </aside>
  );
}
