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
import { ContextMenu, Menu, Popover } from '/kit/overlay-primitives';
import type { ReactNode } from 'react';
import { Band, Button, Chip, Icon, MenuButton, MenuItem } from '../components';
import { InvitationPanel } from '../components/invitation-panel';
import { InviteNewUserSheet } from '../components/invite-new-user-sheet';
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
  serverDisplayLabel,
  serverDisplayLabelForStore as displayServerName,
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
import type { Location, NavigateOptions, TeamsSheetIntent } from '../location';
import type { MutationFailureHandler } from '../mutation-recovery';
import { markProfileRostersStale } from '../roster-staleness';
import { PageHeader } from '../shell/page-header';
import { useToast } from '/kit/toasts';
import { GroupMark } from './group-mark';
import {
  discoveryContext,
  manageReason,
  rosterManageable,
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
  scene?: string;
  onNavigate: (location: Location, options?: NavigateOptions) => void;
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
        Find teams <Icon name="chevronDown" className="chevron" />
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
                    context ? serverDisplayLabel(context.server) : server
                  }
                >
                  <div className="find-groups-server">
                    <Icon name="server" />
                    <span>
                      {context ? serverDisplayLabel(context.server) : server}
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
                        <Icon name="refresh" />
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
  onContextMenu,
}: {
  snapshot: AgentSnapshot;
  store: TeamStore;
  /** Membership requests waiting on this team, drawn as a chip on the row. */
  requests?: number;
  menu: ReactNode;
  onOpen: () => void;
  onContextMenu: (point: { x: number; y: number }) => void;
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
    <div
      className="rowline"
      onContextMenu={(event) => {
        if (
          event.target instanceof Element &&
          event.target.closest('[role="menu"], .rowmenu')
        )
          return;
        event.preventDefault();
        onContextMenu({ x: event.clientX, y: event.clientY });
      }}
    >
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
            <Icon name="chevronDown" />
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
  // The address can arrive asking for one of these sheets: the Chat tab's
  // empty pane and the Files tree's New team button send a reader here to
  // create a team or to paste an invitation, and `?state=create` and
  // `?state=join` name the same two. The intent is spent on arrival — the
  // sheet opens once, and the address is then replaced with the plain list so
  // that Back, a re-render or a return to the tab lands on the list.
  const intent = location.open;
  // Neither sheet is seeded from the scene name: `?state=create` and
  // `?state=join` decode to the `open` intent above, and the scene name lasts
  // the whole session, so seeding from it would reopen the sheet on every
  // return to the tab after the reader closed it.
  const [sheet, setSheet] = useTabSheetState<ListSheet | null>(
    'teams.sheet',
    null,
    (value) => value?.kind === 'create' || value?.kind === 'add',
  );
  const [joining, setJoining] = useTabSheetState<AccountStore | null>(
    'teams.join',
    null,
    (value) => value !== null,
  );
  // Read through refs: the acting account is a fresh object on every render
  // and the shell's callback a fresh closure, neither of which is a new
  // intent to act on.
  const actingRef = useRef(acting);
  actingRef.current = acting;
  const navigateRef = useRef(onNavigate);
  navigateRef.current = onNavigate;
  const actingId = acting?.id;
  // The intent is spent in two commits: the address is canonicalized first,
  // and the sheet opens on the commit after that, from the request held here.
  // Both sheets above are tab-persisted state owned by the address they were
  // opened under. Opening one while the address still carries `open` would
  // hand it an owner that the replacement immediately supersedes, and the
  // saved record would then outlive the reader closing the sheet: the tab
  // would restore it on the way back.
  const requested = useRef<TeamsSheetIntent | null>(null);
  useEffect(() => {
    // With no account yet the page cannot say who would create or join, so
    // the intent waits for the catalog rather than being spent on nothing.
    if (!intent || !actingRef.current) return;
    requested.current = intent;
    navigateRef.current(
      { kind: 'teams', ...(location.store ? { store: location.store } : {}) },
      // The page the reader is on, with its address canonicalized: no new
      // entry to go back to, and no screen may refuse it.
      { replace: true, force: true },
    );
  }, [intent, actingId, location.store]);
  const teamsRef = useRef(teams);
  teamsRef.current = teams;
  const [inviting, setInviting] = useState<TeamStore | null>(null);
  const [context, setContext] = useState<{
    x: number;
    y: number;
    store: TeamStore;
  } | null>(null);
  useEffect(() => {
    const request = requested.current;
    const store = actingRef.current;
    if (intent || !request || !store) return;
    requested.current = null;
    if (request === 'create') setSheet({ kind: 'create', store });
    else if (request === 'join') setJoining(store);
    else {
      const team = teamsRef.current.find(
        (candidate) => candidate.id === location.store,
      );
      if (!team) return;
      if (request === 'invite') setInviting(team);
      else setSheet({ kind: request, store: team });
    }
  }, [intent, actingId, location.store, setSheet, setJoining]);
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
      const result = await enqueueProfileWork(
        bridge,
        context.server.profileName,
        () =>
          bridge.discoverGroups(
            context.server.profileName,
            context.account.alias,
          ),
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
   * Share actions between the row menu and the right-click menu. Use the
   * same availability checks and disabled-action explanations as team settings.
   */
  const rowMenuItems = (store: TeamStore, close: () => void): ReactNode => {
    const rosterReason = manageReason(snapshot, store, 'roster');
    const federationReason = manageReason(snapshot, store, 'federation');
    const serverName = displayServerName(snapshot, store);
    const requests =
      store.team_kind === 'named' ? (requestCounts.get(store.id) ?? 0) : 0;
    return (
      <>
        {store.active === false ? (
          <>
            <MenuItem
              icon="refresh"
              onClick={() => {
                close();
                finishSetup(store);
              }}
            >
              Finish setup…
            </MenuItem>
            {/* Offer removal for incomplete teams whose setup cannot be resumed. */}
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
        <MenuItem
          icon="user"
          reason={rosterReason}
          onClick={() => {
            close();
            setSheet({ kind: 'add', store });
          }}
        >
          <span className="menu-choice">
            <span>Add FOKS user…</span>
            <small>Someone with an account on {serverName}</small>
          </span>
        </MenuItem>
        <MenuItem
          icon="users"
          reason={federationReason}
          onClick={() => {
            close();
            setSheet({ kind: 'add-team', store });
          }}
        >
          <span className="menu-choice">
            <span>Add FOKS team…</span>
            <small>A team from another server</small>
          </span>
        </MenuItem>
        <MenuItem
          icon="door"
          reason={rosterReason}
          onClick={() => {
            close();
            setInviting(store);
          }}
        >
          <span className="menu-choice">
            <span>Invite new user…</span>
            <small>Create an invitation to share</small>
          </span>
        </MenuItem>
        <div className="menu-separator" role="separator" />
        {store.team_kind === 'named' ? (
          <MenuItem
            icon="shield"
            reason={rosterReason}
            onClick={() => {
              close();
              onNavigate({
                kind: 'group-settings',
                ref: store.id,
                tab: 'requests',
              });
            }}
          >
            Review requests
            {requests ? <kbd>{requests}</kbd> : null}
          </MenuItem>
        ) : null}
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
          icon="externalLink"
          onClick={() => {
            close();
            onNavigate({ kind: 'store', ref: store.id });
          }}
        >
          Open in Files
        </MenuItem>
      </>
    );
  };

  const rowMenu = (store: TeamStore): ReactNode => (
    <span className="rowmenu">
      <MenuButton
        variant="quiet"
        icon="ellipsis"
        trailingIcon={null}
        label=""
        menuLabel={`Actions for ${store.name}`}
        aria-label={`Actions for ${store.name}`}
      >
        {(close) => rowMenuItems(store, close)}
      </MenuButton>
    </span>
  );

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
        onContextMenu={(point) => setContext({ ...point, store })}
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
                    <Icon name="users" />
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
          onInvite={
            sheet.store.kind === 'team'
              ? () => {
                  const team = sheet.store as TeamStore;
                  setSheet(null);
                  setInviting(team);
                }
              : undefined
          }
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
      {context ? (
        <ContextMenu
          point={context}
          className="menu-portal"
          onClose={() => setContext(null)}
        >
          <Menu
            className="menu"
            aria-label={`Actions for ${context.store.name}`}
            initialFocus="first"
            onClose={() => setContext(null)}
          >
            {rowMenuItems(context.store, () => setContext(null))}
          </Menu>
        </ContextMenu>
      ) : null}
      {inviting ? (
        <InviteNewUserSheet
          bridge={bridge}
          team={inviting}
          serverLabel={displayServerName(snapshot, inviting)}
          approver={
            snapshot.accounts.find(
              (candidate) =>
                candidate.alias === inviting.account &&
                candidate.server === inviting.server,
            )?.username
          }
          onClose={() => setInviting(null)}
          onComplete={() => onRefresh('Invitation created', inviting.server)}
        />
      ) : null}
      {joining ? (
        <InvitationPanel
          bridge={bridge}
          profile={joining.server}
          account={joining.account}
          servers={snapshot.servers}
          // A team may join when it is on the account's server and the account
          // can manage its roster.
          teams={teams.filter(
            (team) =>
              team.server === joining.server &&
              team.account === joining.account &&
              rosterManageable(snapshot, team),
          )}
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
