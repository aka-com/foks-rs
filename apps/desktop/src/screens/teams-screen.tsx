/**
 * The Teams tab: the groups and shares on this Mac, the per-account checks that
 * find more of them, and the entries that create or join one.
 *
 * A row carries only catalog facts — mark, name, server, roster summary and the
 * role this Mac's account holds — and says an abnormal state with a chip at the
 * end of the row. A row opens the group's page; the group page returns here.
 */

import { useEffect, useState } from 'react';
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
  canCreateInStore,
  groupDetailFailure,
  parseRole,
  partiesOf,
  plural,
  roleName,
  serverOf,
  storeDescription,
  storeDescriptionState,
  storeNavigationOrder,
  storeReadable,
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
import {
  checkLabel,
  discoveryContext,
  GroupMark,
  GroupSheet,
  inviteUnavailableTitle,
  leaveReason,
  manageReason,
  unavailableTitle,
} from './groups-screen';
import type { DiscoveryContext, GroupSheetKind } from './groups-screen';
import { accountSubtitle } from './settings-screen';

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

/**
 * The caption under a group's name: the server it lives on. The account it is
 * held through is added only when this Mac holds two accounts on that server,
 * where the server alone would not say which one this row belongs to.
 */
function teamCaption(
  snapshot: AgentSnapshot,
  store: TeamStore,
  shared: boolean,
): string {
  const server = serverOf(snapshot, store.id)?.name ?? store.server;
  if (!shared) return server;
  const username =
    snapshot.accounts.find(
      (account) =>
        account.server === store.server && account.alias === store.account,
    )?.username ?? store.account;
  return `${server} · as ${username}`;
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
  const failed = Boolean(
    groupDetailFailure(snapshot, store.id, 'roster') ??
    groupDetailFailure(snapshot, store.id, 'federation'),
  );
  // The abnormal state is a chip at the end of the row; the roster summary
  // takes its place when there is nothing wrong.
  const abnormal = state !== 'normal' || failed;
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
          <small>{teamCaption(snapshot, store, shared)}</small>
        </span>
        <span className="tail">
          {abnormal ? (
            <Chip tone="warn">{description}</Chip>
          ) : (
            <span className="summary">{description}</span>
          )}
          {role ? <Chip className="role">{roleName(role)}</Chip> : null}
        </span>
      </button>
      {menu}
    </div>
  );
}

/**
 * One account store's row: what the server lists for it, and the invitation
 * message that brings someone new to that server.
 */
function AccountRow({
  snapshot,
  store,
  busy,
  result,
  onCheck,
  onInvite,
}: {
  snapshot: AgentSnapshot;
  store: AccountStore;
  busy: boolean;
  result?: string;
  onCheck: (context: DiscoveryContext) => void;
  onInvite: () => void;
}): ReactNode {
  const context = discoveryContext(snapshot, store.id);
  const account = snapshot.accounts.find(
    (candidate) => candidate.store === store.id,
  );
  const available = Boolean(context?.available);
  const caption = account
    ? `as ${account.username} · ${store.account}`
    : 'This account is not signed in on this Mac.';
  // An account row says its state the way every other row does: a chip at the
  // end of the row, not a second caption under the name.
  const state = storeDescriptionState(snapshot, store);
  const serverName = context?.server.name ?? store.name;
  return (
    <div className={available ? 'row flat' : 'row flat off'}>
      <span className="kic Store">
        <Icon name="server" />
      </span>
      <span className="name">
        <span className="tt">
          <span>{serverName}</span>
        </span>
        <small>{caption}</small>
      </span>
      <span className="tail">
        {state === 'normal' ? null : (
          <Chip tone="warn">{storeDescription(snapshot, store)}</Chip>
        )}
        {result ? <span className="summary">{result}</span> : null}
        <Button
          size="sm"
          aria-label={context ? checkLabel(context) : 'Check for groups'}
          title={
            context && !context.available
              ? unavailableTitle(context)
              : 'Ask the server which groups this account belongs to.'
          }
          disabled={!available || busy}
          onClick={() => {
            if (context) onCheck(context);
          }}
        >
          {busy ? 'Checking…' : 'Check for groups'}
        </Button>
        <Button
          size="sm"
          disabled={!available}
          title={
            available
              ? 'Copy a message that helps someone create an account on this server.'
              : context
                ? inviteUnavailableTitle(context.server.name)
                : 'This account is not signed in on this Mac.'
          }
          onClick={onInvite}
        >
          Invite someone…
        </Button>
      </span>
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
  const [sheet, setSheet] = useState<ListSheet | null>(() =>
    scene === 'create' && acting ? { kind: 'create', store: acting } : null,
  );
  const [joining, setJoining] = useState<AccountStore | null>(() =>
    scene === 'join' ? (acting ?? null) : null,
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
    const server = serverOf(snapshot, store.id);
    const serverName = server?.name ?? store.server;
    const rosterReason = manageReason(snapshot, store, 'roster');
    const federationReason = manageReason(snapshot, store, 'federation');
    const inviteReason = storeReadable(snapshot, store.id)
      ? undefined
      : inviteUnavailableTitle(serverName);
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
              <MenuItem
                icon="mail"
                reason={inviteReason}
                onClick={() => {
                  close();
                  setSheet({ kind: 'invite', store });
                }}
              >
                Invite someone…
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
              <hr />
              <MenuItem reason={leaveReason(snapshot, store)}>Leave…</MenuItem>
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
              <p className="fn">No groups on this Mac yet.</p>
            )}
            {shares.length ? (
              <>
                <SectionLabel>Shares</SectionLabel>
                {teamRows(shares)}
              </>
            ) : null}
            <SectionLabel>Servers</SectionLabel>
            {accounts.length ? (
              accounts.map((store) => (
                <AccountRow
                  key={store.id}
                  snapshot={snapshot}
                  store={store}
                  busy={discovering === store.id}
                  result={results[store.id]}
                  onCheck={(context) => void discover(context)}
                  onInvite={() => setSheet({ kind: 'invite', store })}
                />
              ))
            ) : (
              <p className="fn">No accounts configured on this Mac.</p>
            )}
            {canCreate ? null : (
              <p className="fn">
                No available account can create a group right now. Add an
                account, or restore access to a server.
              </p>
            )}
          </div>
        </div>
      </div>
      {sheet ? (
        <GroupSheet
          snapshot={snapshot}
          bridge={bridge}
          store={sheet.store}
          sheet={sheet.kind}
          target={null}
          onClose={() => setSheet(null)}
          onSwitch={() => undefined}
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
          onError={onError}
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
