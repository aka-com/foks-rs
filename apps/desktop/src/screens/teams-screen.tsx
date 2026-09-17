import { useTabSheetState } from '../navigation-guard';
/**
 * The Teams tab: the groups and shares on this Mac, the per-account checks that
 * find more of them, and the entries that create or join one.
 *
 * A row carries only catalog facts — mark, name, server, roster summary and the
 * role this Mac's account holds — and says an abnormal state with a chip at the
 * end of the row. A row opens the group's page; the group page returns here.
 */

import { useEffect, useRef, useState } from 'react';
import { Menu, Popover } from '/kit/overlay-primitives';
import type { ReactNode } from 'react';
import {
  Button,
  Chip,
  Icon,
  MenuButton,
  MenuItem,
  SectionLabel,
} from '../components';
import { InvitationPanel } from '../components/invitation-panel';
import { enqueueProfileWork } from '../bridge';
import {
  accountSubtitle,
  canCreateInStore,
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
  teamCaption,
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
import { PageHeader } from '../shell/page-header';
import { useToast } from '/kit/toasts';
import { GroupMark } from './group-mark';
import {
  discoveryContext,
  manageReason,
  unavailableTitle,
} from './group-model';
import type { DiscoveryContext } from './group-model';
import { GroupSheet } from './groups-screen';
import type { GroupSheetKind } from './groups-screen';

/** Only what this page uses; the tab shares no state with Settings. */
export interface TeamsScreenProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'teams' }>;
  scene: string;
  onNavigate: (location: Location) => void;
  onRefresh: (message: string) => Promise<void>;
  onRefreshSnapshot: () => Promise<AgentSnapshot>;
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
        Find groups <Icon name="chev" className="chevron" />
      </Button>
      {open ? (
        <Popover
          anchorRef={anchorRef}
          className="menu-portal"
          onClose={() => setOpen(false)}
        >
          <Menu
            className="menu find-groups-menu"
            aria-label="Find groups"
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
                      ? 'This account is not signed in on this Mac.'
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
                Add an account to find its groups.
              </p>
            ) : null}
          </Menu>
        </Popover>
      ) : null}
    </>
  );
}

/** One group or share: mark, name, server, roster summary, role and state. */
function TeamRow({
  snapshot,
  store,
  shared,
  menu,
  onOpen,
}: {
  snapshot: AgentSnapshot;
  store: TeamStore;
  shared: boolean;
  menu: ReactNode;
  onOpen: () => void;
}): ReactNode {
  const state = storeDescriptionState(snapshot, store);
  const description = storeDescription(snapshot, store);
  // The abnormal state is a chip at the end of the row; the roster summary
  // takes its place when there is nothing wrong.
  const abnormal = storeAttentionState(snapshot, store) !== 'normal';
  const mine = partiesOf(snapshot, store.id).find(
    (party) => party.label === 'you',
  );
  const role = mine ? parseRole(mine.destination_role) : null;
  return (
    // The row itself is the button that opens the group; its menu is a sibling
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
          {/* The section above already says whether this is a group or a
              share, so the caption says only where it lives. */}
          <small>{teamCaption(snapshot, store, { shared, kind: false })}</small>
        </span>
        <span className="tail">
          {abnormal ? (
            <Chip tone="warn">{description}</Chip>
          ) : (
            <span className="summary">{description}</span>
          )}
          {/* The chip sits in a column of its own width so the roles down the
              list line up; the chip itself keeps its own. */}
          {role ? (
            <span className="rolecell">
              <Chip className="role">{roleName(role)}</Chip>
            </span>
          ) : null}
          {/* The row opens the group, and says so at its end. */}
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
  const teams = stores.filter(
    (store): store is TeamStore => store.kind === 'team',
  );
  const groups = teams.filter((store) => store.team_kind === 'named');
  const shares = teams.filter((store) => store.team_kind === 'adhoc');
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
  const [discovering, setDiscovering] = useState<StoreRef | null>(null);
  const [results, setResults] = useState<Readonly<Record<string, string>>>({});
  const canCreate = accounts.some((store) =>
    canCreateInStore(snapshot, store.id),
  );
  const servers = new Set(accounts.map((store) => store.server));
  // A check's result belongs to the row it was run from. The rows themselves
  // change when an account is added or removed, and a result read against a
  // different set of accounts is stale, so it is dropped with them.
  const accountKey = accounts.map((store) => store.id).join('|');
  useEffect(() => {
    setResults({});
  }, [accountKey]);

  // Ask one account's server which groups it belongs to. Discovery writes
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
          'Group discovery returned data for a different account.',
        );
      const found = result.groups.filter((group) => group.active).length;
      const message = found
        ? `Found ${plural(found, 'group')} for ${context.account.username}.`
        : `No groups found for ${context.account.username}.`;
      // The row says what the check found; a toast repeating that sentence
      // would say it twice, so the refresh is silent.
      setResults((old) => ({ ...old, [context.store.id]: message }));
      await onRefreshSnapshot();
    } catch (error) {
      setResults((old) => ({
        ...old,
        [context.store.id]: 'Check failed. Try again.',
      }));
      await onMutationError(error);
    } finally {
      setDiscovering(null);
    }
  };

  const copyId = (store: TeamStore): void => {
    void bridge
      .copyText(store.team_id_hex)
      .then(() => toasts.show('Group ID copied.'))
      .catch(onError);
  };

  const finishSetup = (store: TeamStore): void => {
    void bridge
      .resumeGroupCreation(store.id)
      .then(() => onRefresh('Group creation resumed'))
      .catch((error: unknown) => onMutationError(error));
  };

  /**
   * A row's menu: actions only, inert where they do not apply, with the same
   * reasons the group's own page gives.
   */
  const rowMenu = (store: TeamStore): ReactNode => {
    const serverName = displayServerName(snapshot, store);
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
                <MenuItem
                  icon="again"
                  onClick={() => {
                    close();
                    finishSetup(store);
                  }}
                >
                  Finish setup…
                </MenuItem>
              ) : null}
              <MenuItem
                icon="plus"
                reason={rosterReason}
                onClick={() => {
                  close();
                  setSheet({ kind: 'add', store });
                }}
              >
                Add someone on {serverName}…
              </MenuItem>
              <MenuItem
                icon="people"
                reason={federationReason}
                onClick={() => {
                  close();
                  setSheet({ kind: 'admit', store });
                }}
              >
                Add a group…
              </MenuItem>
              <hr />
              <MenuItem
                icon="copy"
                onClick={() => {
                  close();
                  copyId(store);
                }}
              >
                Copy group ID
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
        shared={
          accounts.filter((account) => account.server === store.server).length >
          1
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
          plural(groups.length, 'group'),
          plural(shares.length, 'share'),
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
                  ? 'Paste an invitation from an administrator of that group.'
                  : 'No account on this Mac can request membership.'
              }
              onClick={() => setJoining(acting ?? null)}
            >
              Join a group…
            </Button>
            <Button
              variant="primary"
              icon="plus"
              disabled={!canCreate || !acting}
              title={
                canCreate
                  ? undefined
                  : 'No available account can create a group'
              }
              onClick={() => {
                if (acting) setSheet({ kind: 'create', store: acting });
              }}
            >
              Create a group
            </Button>
          </>
        }
      />
      <div className="body nav-rows">
        <div className="list-window">
          <div className="virtual-rows">
            <SectionLabel>Groups</SectionLabel>
            {groups.length ? (
              teamRows(groups)
            ) : (
              <div className="empty">
                <div className="big">
                  <Icon name="people" />
                </div>
                <h2>No groups yet</h2>
                <p>
                  A group is a shared store with roles. Create one on a server
                  this Mac holds an account on, or ask a server whether it
                  already lists you in one.
                </p>
                <Button
                  variant="primary"
                  icon="plus"
                  disabled={!canCreate || !acting}
                  title={
                    canCreate
                      ? undefined
                      : 'No available account can create a group'
                  }
                  onClick={() => {
                    if (acting) setSheet({ kind: 'create', store: acting });
                  }}
                >
                  Create a group
                </Button>
              </div>
            )}
            {shares.length ? (
              <>
                <SectionLabel>Shares</SectionLabel>
                {teamRows(shares)}
              </>
            ) : null}
          </div>
        </div>
      </div>
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
          onApplied={async (message, created) => {
            const next = await onRefreshSnapshot();
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
          presentation={{
            title: 'Join a group',
            subtitle: accountSubtitle(snapshot, joining),
            onClose: () => setJoining(null),
          }}
          onComplete={() => onRefresh('Group membership refreshed')}
        />
      ) : null}
    </>
  );
}
