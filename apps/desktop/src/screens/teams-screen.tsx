import { synchronizeApplied } from '../operation-outcome';
import { useTabSheetState } from '../navigation-guard';
/**
 * The Teams tab: every named team and ad-hoc share on this Mac in one list,
 * the per-account checks that find more of them, and the entries that create
 * or join one.
 *
 * Each row displays team metadata: avatar, name, server, member count, the
 * user's role, a Chat or Share badge for team type, and a status chip for
 * abnormal states. A row opens the team's
 * page; the team page returns here.
 */

import { useEffect, useRef, useState } from 'react';
import { Menu, Popover } from '/kit/overlay-primitives';
import type { ReactNode } from 'react';
import { Band, Button, Chip, Icon, MenuButton, MenuItem } from '../components';
import { InvitationPanel } from '../components/invitation-panel';
import { useTeamRequestCounts } from '../operation-queries';
import {
  commandRecovery,
  enqueueProfileWork,
  isTerminalCommandError,
  normalizeCommandError,
} from '../bridge';
import {
  storeOperationAvailability,
  groupDetailFailure,
  parseRole,
  partiesOf,
  plural,
  roleName,
  serverDisplayName,
  serverName as displayServerName,
  storeAttentionState,
  storeDescription,
  storeDescriptionState,
  storeNavigationOrder,
} from '../model';
import type {
  AccountStore,
  AgentSnapshot,
  Store,
  StoreRef,
  TeamStore,
} from '../model';
import type { Bridge } from '../bridge';
import type { Location } from '../location';
import type { MutationFailureHandler } from '../mutation-recovery';
import { markProfileRostersStale } from '../roster-staleness';
import { PageHeader } from '../shell/page-header';
import { useToast } from '/kit/toasts';
import { GroupMark } from './group-mark';
import {
  discoveryContext,
  manageReason,
  unavailableTitle,
} from './group-model';
import type { DiscoveryContext } from './group-model';
import { AbandonGroupSheet, GroupSheet } from './groups-screen';
import type { GroupSheetKind } from './groups-screen';

/** Only what this page uses; the tab shares no state with Settings. */
export interface TeamsScreenProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'teams' }>;
  scene: string;
  onNavigate: (location: Location) => void;
  onRefresh: (message: string, profile?: string) => Promise<void>;
  onRefreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
}

/** A sheet opened from this page, and the store it acts on. */
interface ListSheet {
  kind: GroupSheetKind;
  store: Store;
}

/** Account-scoped discovery, grouped by server in an anchored menu. */
function FindGroups({
  snapshot,
  accounts,
  discovering,
  results,
  onCheck,
}: {
  snapshot: AgentSnapshot;
  accounts: AccountStore[];
  discovering: StoreRef | null;
  results: Readonly<Record<string, string>>;
  onCheck: (context: DiscoveryContext) => void;
}): ReactNode {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const servers = [...new Set(accounts.map((account) => account.server))];
  return (
    <>
      <Button
        ref={anchorRef}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        Find teams <Icon name="chev" className="chevron" />
      </Button>
      {open ? (
        <Popover
          anchorRef={anchorRef}
          className="menu-portal"
          onClose={() => setOpen(false)}
        >
          <Menu
            className="menu find-groups-menu"
            aria-label="Find teams"
            anchorRef={anchorRef}
            onClose={() => setOpen(false)}
          >
            {servers.map((server) => {
              const members = accounts.filter(
                (account) => account.server === server,
              );
              const context = discoveryContext(snapshot, members[0].id);
              return (
                <div
                  key={server}
                  role="group"
                  aria-label={
                    context ? serverDisplayName(context.server) : server
                  }
                >
                  <div className="find-groups-server">
                    <Icon name="server" />
                    <span>
                      {context ? serverDisplayName(context.server) : server}
                    </span>
                  </div>
                  {members.map((store) => {
                    const context = discoveryContext(snapshot, store.id);
                    const busy = discovering === store.id;
                    const reason = !context
                      ? 'This account is not signed in on this device.'
                      : !context.available
                        ? unavailableTitle(context)
                        : discovering
                          ? 'Wait for the current check to finish.'
                          : undefined;
                    return (
                      <MenuItem
                        key={store.id}
                        reason={reason}
                        onClick={() => {
                          if (context) onCheck(context);
                        }}
                      >
                        <span className="find-groups-account">
                          <span>
                            {context
                              ? `Check as ${context.account.username}`
                              : store.account}
                          </span>
                          <small role="status">
                            {busy
                              ? 'Checking…'
                              : (results[store.id] ?? (reason ? reason : ''))}
                          </small>
                        </span>
                        <Icon name="again" />
                      </MenuItem>
                    );
                  })}
                </div>
              );
            })}
            {!accounts.length ? (
              <p className="find-groups-empty">
                Add an account to find its teams.
              </p>
            ) : null}
          </Menu>
        </Popover>
      ) : null}
    </>
  );
}

/**
 * One team, named or ad-hoc: mark, then a caption built only from what the
 * catalog already gives this row — its server, its member count, and the
 * role this account holds in it — each clause dropped rather than guessed
 * when the catalog does not have it. A pill at the end says its kind: a named
 * team opens onto Chat, an ad-hoc team is a fixed share with none.
 */
function TeamRow({
  snapshot,
  store,
  requests = 0,
  menu,
  onOpen,
}: {
  snapshot: AgentSnapshot;
  store: TeamStore;
  /** Membership requests waiting on this team, drawn as a chip on the row. */
  requests?: number;
  menu: ReactNode;
  onOpen: () => void;
}): ReactNode {
  const state = storeDescriptionState(snapshot, store, { operation: 'teams' });
  const description = storeDescription(snapshot, store, { operation: 'teams' });
  // The abnormal state is a chip at the end of the row.
  const abnormal =
    storeAttentionState(snapshot, store, { operation: 'teams' }) !== 'normal';
  const mine = partiesOf(snapshot, store.id).find(
    (party) => party.label === 'you',
  );
  const role = mine ? parseRole(mine.destination_role) : null;
  // A roster the agent could not read is not a member count of zero; the
  // clause is dropped rather than stating a count that may be wrong.
  const rosterKnown =
    ![
      'loading',
      'server-status-unavailable',
      'capability-unavailable',
    ].includes(state) && !groupDetailFailure(snapshot, store.id, 'roster');
  const caption = [
    displayServerName(snapshot, store),
    rosterKnown ? plural(partiesOf(snapshot, store.id).length, 'member') : null,
  ]
    .filter((part): part is string => Boolean(part))
    .join(' · ');
  return (
    // The row itself is the button that opens the team; its menu is a sibling
    // of that button, not a control nested inside one.
    <div className="rowline">
      <button
        type="button"
        className={state === 'normal' ? 'row' : 'row off'}
        onClick={onOpen}
      >
        <GroupMark store={store} size="sm" />
        <span className="name">
          <span className="tt">
            <span>{store.name}</span>
          </span>
          <small>{caption}</small>
        </span>
        <span className="tail">
          {/* This Mac's role in the team, a chip at the end of the row. */}
          {role ? <Chip className="role">{roleName(role)}</Chip> : null}
          {abnormal ? <Chip tone="warn">{description}</Chip> : null}
          {/* The band above the list names the team; the row says it too,
              so on a long list the reader need not match the two by name. */}
          {requests > 0 ? (
            <Chip tone="warn">{plural(requests, 'request')}</Chip>
          ) : null}
          <Chip className="kind">
            {store.team_kind === 'adhoc' ? 'Share' : 'Chat'}
          </Chip>
          {/* The row opens the team, and says so at its end. */}
          <span className="go" aria-hidden="true">
            <Icon name="chev" />
          </span>
        </span>
      </button>
      {menu}
    </div>
  );
}

export function TeamsScreen({
  snapshot,
  bridge,
  location,
  scene,
  onNavigate,
  onRefresh,
  onRefreshSnapshot,
  onError,
  onMutationError,
}: TeamsScreenProps): ReactNode {
  const toasts = useToast();
  const stores = storeNavigationOrder(snapshot);
  const catalogLoading =
    snapshot.profileInventoryStatus !== 'complete' &&
    !snapshot.servers.length &&
    !snapshot.stores.length;
  // Named teams and ad-hoc shares are one list here; a row's own pill says
  // which it is, so nothing above the list needs to split them.
  // A team whose setup never finished is listed after every team that works;
  // among the rest the navigation order stands.
  const teams = stores
    .filter((store): store is TeamStore => store.kind === 'team')
    .sort(
      (left, right) =>
        Number(left.active === false) - Number(right.active === false),
    );
  const accounts = stores.filter(
    (store): store is AccountStore => store.kind === 'account',
  );
  // The list is the whole Mac's. The address only names the account that
  // creating, inviting and joining act as; with none, the first account.
  const acting =
    (location.store
      ? accounts.find((store) => store.id === location.store)
      : undefined) ?? accounts[0];
  const [sheet, setSheet] = useTabSheetState<ListSheet | null>(
    'teams.sheet',
    () =>
      scene === 'create' && acting ? { kind: 'create', store: acting } : null,
    (value) => value?.kind === 'create' || value?.kind === 'add',
  );
  const [joining, setJoining] = useTabSheetState<AccountStore | null>(
    'teams.join',
    () => (scene === 'join' ? (acting ?? null) : null),
    (value) => value !== null,
  );
  // The stuck creation a row asked to forget. Held apart from `sheet`, which
  // is the group sheet's own set of kinds.
  const [abandoning, setAbandoning] = useState<TeamStore | null>(null);
  const [discovering, setDiscovering] = useState<StoreRef | null>(null);
  const [results, setResults] = useState<Readonly<Record<string, string>>>({});
  // The rail's own Teams badge sums these same shared rows across the whole
  // Mac; here they are drawn per team.
  const requestCounts = useTeamRequestCounts(bridge, snapshot, onError);
  const flaggedTeams = teams.filter(
    (store) =>
      store.team_kind === 'named' && (requestCounts.get(store.id) ?? 0) > 0,
  );
  const canCreate = accounts.some(
    (store) => storeOperationAvailability(snapshot, store, 'teams').available,
  );
  const servers = new Set(accounts.map((store) => store.server));
  // A check's result belongs to the row it was run from. The rows themselves
  // change when an account is added or removed, and a result read against a
  // different set of accounts is stale, so it is dropped with them.
  const accountKey = accounts.map((store) => store.id).join('|');
  useEffect(() => {
    setResults({});
  }, [accountKey]);

  // Ask one account's server which teams it belongs to. Discovery writes
  // durable local bindings, so it runs through the profile work queue and any
  // failure is reconciled like a mutation rather than replayed blindly.
  const discover = async (context: DiscoveryContext): Promise<void> => {
    setDiscovering(context.store.id);
    try {
      const result = await enqueueProfileWork(bridge, context.server.id, () =>
        bridge.discoverGroups(context.server.id, context.account.alias),
      );
      if (result.accountAlias !== context.account.alias)
        throw new Error(
          'Team discovery returned data for a different account.',
        );
      const found = result.groups.filter((group) => group.active).length;
      const message = found
        ? `Found ${plural(found, 'team')} for ${context.account.username}.`
        : `No teams found for ${context.account.username}.`;
      // The row says what the check found; a toast repeating that sentence
      // would say it twice, so the refresh is silent.
      setResults((old) => ({ ...old, [context.store.id]: message }));
      await onRefreshSnapshot(true);
    } catch (error) {
      const typed = normalizeCommandError(error);
      const recovery = commandRecovery(typed);
      // The row states the failure in the agent's words, and "Try again" only
      // where trying again is the answer. A cancelled check leaves no result.
      setResults((old) => {
        if (recovery.kind === 'ignore') {
          const { [context.store.id]: dropped, ...rest } = old;
          void dropped;
          return rest;
        }
        return {
          ...old,
          [context.store.id]:
            recovery.kind === 'retry'
              ? `Check failed. ${typed.message} Try again.`
              : `Check failed. ${typed.message}`,
        };
      });
      // The row is the failure's surface, so an ordinary refusal is not
      // toasted as well; a lost or unsafe agent still reaches the shell.
      await onMutationError(error, { report: isTerminalCommandError(typed) });
    } finally {
      setDiscovering(null);
    }
  };

  const copyId = (store: TeamStore): void => {
    void bridge
      .copyText(store.team_id_hex)
      .then(() => toasts.show('Team ID copied.'))
      .catch(onError);
  };

  const finishSetup = (store: TeamStore): void => {
    // Resuming creation moves the team's chain, so the read back must read
    // its roster again rather than reuse the one it holds.
    markProfileRostersStale(store.server);
    void bridge
      .resumeGroupCreation(store.id)
      .then(() => onRefresh('Team creation resumed', store.server))
      .catch((error: unknown) => onMutationError(error));
  };

  /**
   * A row's menu: actions only, inert where they do not apply, with the same
   * reasons the team's own page gives.
   */
  const rowMenu = (store: TeamStore): ReactNode => {
    const rosterReason = manageReason(snapshot, store, 'roster');
    const federationReason = manageReason(snapshot, store, 'federation');
    return (
      <span className="rowmenu">
        <MenuButton
          variant="quiet"
          icon="more"
          trailingIcon={null}
          label=""
          menuLabel={`Actions for ${store.name}`}
          aria-label={`Actions for ${store.name}`}
        >
          {(close) => (
            <>
              {store.active === false ? (
                <>
                  <MenuItem
                    icon="again"
                    onClick={() => {
                      close();
                      finishSetup(store);
                    }}
                  >
                    Finish setup…
                  </MenuItem>
                  {/* Finishing cannot succeed for every stuck creation, so the
                      row that offers it offers the way out beside it. */}
                  <MenuItem
                    icon="trash"
                    danger
                    onClick={() => {
                      close();
                      setAbandoning(store);
                    }}
                  >
                    Remove team…
                  </MenuItem>
                  <div className="menu-separator" role="separator" />
                </>
              ) : null}
              {/* Invitation options matching the team page: add an individual
                  user or admit a federated team from another server. Disabled
                  options provide an explanatory reason directly in the menu. */}
              <MenuItem
                icon="person"
                reason={rosterReason}
                onClick={() => {
                  close();
                  setSheet({ kind: 'add', store });
                }}
              >
                <span className="menu-choice">
                  <span>Add a user…</span>
                  <small>Invite by their username</small>
                </span>
              </MenuItem>
              <MenuItem
                icon="people"
                reason={federationReason}
                onClick={() => {
                  close();
                  setSheet({ kind: 'admit', store });
                }}
              >
                <span className="menu-choice">
                  <span>Add a team from another server…</span>
                  <small>Add a remote team via federation</small>
                </span>
              </MenuItem>
              <div className="menu-separator" role="separator" />
              <MenuItem
                icon="copy"
                onClick={() => {
                  close();
                  copyId(store);
                }}
              >
                Copy team ID
              </MenuItem>
              <MenuItem
                icon="out"
                onClick={() => {
                  close();
                  onNavigate({ kind: 'store', ref: store.id });
                }}
              >
                Open in Files
              </MenuItem>
            </>
          )}
        </MenuButton>
      </span>
    );
  };

  const teamRows = (list: readonly TeamStore[]): ReactNode =>
    list.map((store) => (
      <TeamRow
        key={store.id}
        snapshot={snapshot}
        store={store}
        requests={
          store.team_kind === 'named' ? (requestCounts.get(store.id) ?? 0) : 0
        }
        menu={rowMenu(store)}
        onOpen={() =>
          onNavigate({
            kind: 'group-settings',
            ref: store.id,
            tab: 'people',
          })
        }
      />
    ));

  return (
    <>
      <PageHeader
        title="Teams"
        subtitle={[
          plural(teams.length, 'team'),
          plural(servers.size, 'server'),
        ].join(' · ')}
        action={
          <>
            <FindGroups
              snapshot={snapshot}
              accounts={accounts}
              discovering={discovering}
              results={results}
              onCheck={(context) => void discover(context)}
            />
            <Button
              disabled={!acting}
              title={
                acting
                  ? 'Paste an invitation from an administrator of that team.'
                  : 'No account on this device can request membership.'
              }
              onClick={() => setJoining(acting ?? null)}
            >
              Join a team…
            </Button>
            <Button
              variant="primary"
              icon="plus"
              disabled={!canCreate || !acting}
              title={
                canCreate ? undefined : 'No available account can create a team'
              }
              onClick={() => {
                if (acting) setSheet({ kind: 'create', store: acting });
              }}
            >
              Create a team
            </Button>
          </>
        }
      />
      {flaggedTeams.length ? (
        <div className="bandstrip">
          {flaggedTeams.map((store) => {
            const count = requestCounts.get(store.id) ?? 0;
            return (
              <Band
                key={store.id}
                action={
                  <Button
                    variant="primary"
                    size="sm"
                    onClick={() =>
                      onNavigate({
                        kind: 'group-settings',
                        ref: store.id,
                        tab: 'requests',
                      })
                    }
                  >
                    Review
                  </Button>
                }
              >
                {plural(count, 'request')} to join {store.name}{' '}
                {count === 1 ? 'is' : 'are'} waiting for you.
              </Band>
            );
          })}
        </div>
      ) : null}
      {catalogLoading ? (
        <div
          className="body app-loading"
          role="status"
          aria-label="Loading teams"
        >
          <span className="spin" aria-hidden="true" />
        </div>
      ) : (
        <div className="body nav-rows">
          <div className="list-window">
            <div className="virtual-rows">
              {teams.length ? (
                teamRows(teams)
              ) : (
                <div className="empty">
                  <div className="big">
                    <Icon name="people" />
                  </div>
                  <h2>No teams yet</h2>
                  <p>Share files and channels with a team.</p>
                  <Button
                    variant="primary"
                    icon="plus"
                    disabled={!canCreate || !acting}
                    title={
                      canCreate
                        ? undefined
                        : 'No available account can create a team'
                    }
                    onClick={() => {
                      if (acting) setSheet({ kind: 'create', store: acting });
                    }}
                  >
                    Create a team
                  </Button>
                </div>
              )}
            </div>
          </div>
        </div>
      )}
      {sheet ? (
        <GroupSheet
          // Each half of the add sheet answers for itself: the band and the
          // role given on one are not carried into the other.
          key={sheet.kind}
          snapshot={snapshot}
          bridge={bridge}
          store={sheet.store}
          sheet={sheet.kind}
          target={null}
          onClose={() => setSheet(null)}
          // The add sheet's own switch between a person and another server's
          // group: the sheet's store does not change, only what is added.
          onSwitch={(next) => {
            if (next) setSheet({ kind: next, store: sheet.store });
          }}
          onApplied={async (message, options) => {
            // The list navigates to the team a creation just made, which it
            // can only find in the whole catalog, so this completion keeps
            // the whole-catalog read rather than the profile read.
            const created = options?.created;
            const result = await synchronizeApplied(() =>
              onRefreshSnapshot(true),
            );
            if (result.synchronization === 'pending') {
              if (result.error.code === 'catalog-read-retired') return;
              toasts.show(
                'Change completed. Updated data could not be loaded. Use Refresh to reload it.',
                { tone: 'warning' },
              );
              return;
            }
            const next = result.value;
            toasts.show(message);
            if (!created) return;
            const accountStore = next.stores.find(
              (candidate) =>
                candidate.id === created.accountStoreId &&
                candidate.kind === 'account',
            );
            const createdStore = accountStore
              ? next.stores.find(
                  (candidate): candidate is TeamStore =>
                    candidate.kind === 'team' &&
                    candidate.server === accountStore.server &&
                    candidate.account === accountStore.account &&
                    candidate.alias === created.teamAlias,
                )
              : undefined;
            if (createdStore)
              onNavigate({ kind: 'store', ref: createdStore.id });
          }}
          onMutationError={onMutationError}
        />
      ) : null}
      {joining ? (
        <InvitationPanel
          bridge={bridge}
          profile={joining.server}
          account={joining.account}
          servers={snapshot.servers}
          presentation={{
            title: 'Join a team',
            onClose: () => setJoining(null),
          }}
          onComplete={() =>
            onRefresh('Team membership refreshed', joining.server)
          }
        />
      ) : null}
      {abandoning ? (
        <AbandonGroupSheet
          snapshot={snapshot}
          bridge={bridge}
          store={abandoning}
          onClose={() => setAbandoning(null)}
          onRemoved={async () => {
            const { name, server } = abandoning;
            setAbandoning(null);
            await onRefresh(`${name} removed`, server);
          }}
          onMutationError={onMutationError}
        />
      ) : null}
    </>
  );
}
