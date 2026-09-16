import { useDeviceMetadata } from '../device-cache';
import { deviceAlertRegistry } from './device-alert';
import { LocalAliasPanel } from '../components/local-alias-panel';
import { localAliasOf } from '../model';
import { useTabSheetState } from '../navigation-guard';
/**
 * The Account tab: one account's profile, the account the rail header names.
 *
 * The mark and the identity that name the account sit on one row, with a
 * "Switch account" button that opens the same menu the rail header's avatar
 * opens — there is exactly one account switcher, not a second one repeating
 * it. Under that, five facts are rows with their action at the right:
 * Username, Shown as (the local alias), Server, Devices and Teams, each
 * linking to where it is changed or managed. Bot accounts, Manage via web
 * and Organization sign-in stay reachable from a quieter line under the
 * facts. A stray notice this page cannot route anywhere else — a catalog
 * read that only a retry can fix, or a note whose place could not be
 * resolved — is kept in a small band above the facts; every other notice
 * already has a home of its own on Settings, Teams or a team's own page.
 */

import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { AdminPanel } from '../components/admin-panel';
import { BotPanel } from '../components/bot-panel';
import { RenamePanel } from '../components/rename-panel';
import { SsoPanel } from '../components/sso-panel';
import { Band, Button, Chip, Inset, InsetRow, Notice } from '../components';
import type { Bridge } from '../bridge';
import {
  accountStopped,
  accountStores,
  notesNow,
  plural,
  serverDisplayName,
  serverName,
  storeDescription,
  storeNavigationOrder,
  usernameOf,
} from '../model';
import type {
  AccountStore,
  AgentSnapshot,
  Notification,
  TeamStore,
} from '../model';
import type { Server } from '../model';
import type { Location, NavigateOptions } from '../location';
import type { MutationFailureHandler } from '../mutation-recovery';
import { PageHeader } from '../shell/page-header';
import { AccountMark } from './account-switcher';
import { AccountHeader } from '../shell/sidebar';
import { deviceEntries } from './device-model';
import type { DeviceLists } from './device-model';
import { GoProfileConnectSheet } from './go-profile-connect';

const ACTION_UNAVAILABLE =
  'Resolve this in Settings › Servers, or in the team’s settings.';

/** The account panels reached from a row on this page. */
type AccountSheet = 'local-alias' | 'rename' | 'bot' | 'admin' | 'sso';

function canRetry(note: Notification): boolean {
  return note.action === 'Retry' && note.id.startsWith('catalog-');
}

/**
 * Returns navigation parameters for the group's Members tab, where setup
 * status and membership admission details can be reviewed and completed.
 */
function openGroup(group: TeamStore): Destination {
  return {
    label: `Open ${group.name}`,
    where: `Teams › ${group.name} › Members`,
    location: { kind: 'group-settings', ref: group.id, tab: 'people' },
  };
}

/** Where a note is resolved: the button's label, and the path it leads to. */
interface Destination {
  label: string;
  /** Human-readable path for the notification card subtitle. */
  where: string;
  location: Location;
}

/** A server profile named by a note id's reason prefix, e.g. `check-in-expired-acme`. */
function serverNamedByReason(
  snapshot: AgentSnapshot,
  id: string,
): Server | undefined {
  const reasons = [
    'check-in-expired',
    'check-in-unavailable',
    'server-status-unavailable',
    'verification-failed',
    'verification-required',
    'status-unavailable',
  ];
  for (const reason of reasons) {
    if (!id.startsWith(`${reason}-`)) continue;
    const profile = id.slice(reason.length + 1);
    const server = snapshot.servers.find((entry) => entry.id === profile);
    if (server) return server;
  }
  return undefined;
}

/** A team named by a `group-roster-unavailable-` or `group-federation-unavailable-` note id. */
function groupNamedByFailure(
  snapshot: AgentSnapshot,
  id: string,
): TeamStore | undefined {
  const prefixes = [
    'group-roster-unavailable-',
    'group-federation-unavailable-',
  ];
  for (const prefix of prefixes) {
    if (!id.startsWith(prefix)) continue;
    const storeId = id.slice(prefix.length);
    return snapshot.stores.find(
      (store): store is TeamStore =>
        store.kind === 'team' && store.id === storeId,
    );
  }
  return undefined;
}

/**
 * Where a note is resolved, when its id names a place this Mac holds. A note
 * about a server — a lapsed check-in, an unverified or blocked server, a
 * status read that failed — names the profile in its id's reason prefix and
 * resolves to that server's own Settings page, where the same state already
 * shows in "Needs attention". A note about a team's roster or federation read
 * failing names the store and resolves to that team's own page. The
 * synthetic `lease-`, `team-` and `fed-` prefixes are the fixture's own
 * shorthand for the same three destinations, kept for its notices. A note
 * whose place cannot be derived keeps the label it always had rather than
 * opening the wrong page.
 *
 * A note this resolves is not drawn on this page at all: the place it names
 * already shows the same state, so this function is used only to tell such a
 * note apart from one that has nowhere else to go.
 */
function destinationOf(
  snapshot: AgentSnapshot,
  note: Notification,
): Destination | null {
  const namedServer =
    (note.id.startsWith('lease-')
      ? snapshot.servers.find(
          (entry) => entry.id === note.id.slice('lease-'.length),
        )
      : undefined) ?? serverNamedByReason(snapshot, note.id);
  if (namedServer)
    return {
      label: 'Open the server',
      where: `Settings › Servers › ${serverDisplayName(namedServer)}`,
      location: {
        kind: 'settings',
        section: 'servers',
        profile: namedServer.id,
      },
    };
  const namedGroup = groupNamedByFailure(snapshot, note.id);
  if (namedGroup) return openGroup(namedGroup);
  if (note.id.startsWith('team-')) {
    const alias = note.id.slice('team-'.length);
    const matches = snapshot.stores.filter(
      (store): store is TeamStore =>
        store.kind === 'team' && store.alias === alias,
    );
    return matches.length === 1 ? openGroup(matches[0]) : null;
  }
  if (note.id.startsWith('fed-')) {
    // `fed-homelab` is Homelab's admission *into* another group, and only the
    // host group can restore it. The note names the admitted group alone, so
    // it points somewhere only while exactly one inactive admission carries
    // that alias: two hosts would make the choice of page a guess.
    const alias = note.id.slice('fed-'.length);
    const matches = snapshot.federation.filter(
      (entry) => entry.remote_team_alias === alias && !entry.active,
    );
    if (matches.length !== 1) return null;
    const host = snapshot.stores.find(
      (store): store is TeamStore =>
        store.kind === 'team' && store.id === matches[0].store,
    );
    return host ? openGroup(host) : null;
  }
  return null;
}

/**
 * The notices no tab can resolve: a catalog read that only a retry can fix,
 * or a note whose place could not be derived — an alias that matches more
 * than one store, or an admission this Mac cannot place. This is what the
 * rail's account-avatar dot still counts, now that every other note type has
 * a badge or an established home of its own.
 */
export function unroutedNotices(snapshot: AgentSnapshot): Notification[] {
  return notesNow(snapshot).filter(
    (note) => canRetry(note) || destinationOf(snapshot, note) === null,
  );
}

interface UnroutedNoticesProps {
  snapshot: AgentSnapshot;
  onRefreshSnapshot: () => Promise<AgentSnapshot>;
  onError: (error: unknown) => void;
}

/**
 * Notices this page cannot route anywhere else: a catalog read that only a
 * retry can fix, or a note whose place could not be resolved — an alias that
 * matches more than one store, or an admission this Mac cannot place. Every
 * other note already has a home: a lapsed or unverified server shows on
 * Settings › Servers, a team whose setup is incomplete shows on Teams, and a
 * team's admission into another team shows as Inactive on the host team's own
 * Members page. Drawing those again here would be a second copy of a state
 * the reader can already see where it is acted on.
 */
function UnroutedNotices({
  snapshot,
  onRefreshSnapshot,
  onError,
}: UnroutedNoticesProps): ReactNode {
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const notes = unroutedNotices(snapshot);
  if (!notes.length) return null;

  const retry = (note: Notification): void => {
    if (!canRetry(note) || busy.has(note.id)) return;
    setBusy((current) => new Set([...current, note.id]));
    onRefreshSnapshot()
      .catch(onError)
      .finally(() => {
        setBusy((current) => {
          const next = new Set(current);
          next.delete(note.id);
          return next;
        });
      });
  };

  return (
    <section className="people-attention" aria-label="Needs attention">
      {notes.map((note) => (
        <Band
          key={note.id}
          severity={note.severity}
          label={note.title}
          action={
            canRetry(note) ? (
              <Button
                variant="primary"
                size="sm"
                disabled={busy.has(note.id)}
                title="Retry loading"
                onClick={() => retry(note)}
              >
                {note.action}
              </Button>
            ) : note.action ? (
              <Chip tone="warn" title={ACTION_UNAVAILABLE}>
                {note.action}
              </Chip>
            ) : null
          }
        >
          {note.detail}
        </Band>
      ))}
    </section>
  );
}

/** The teams this account belongs to, as the catalog holds them. */
function teamsOnAccount(
  snapshot: AgentSnapshot,
  store: AccountStore,
): TeamStore[] {
  return storeNavigationOrder(snapshot).filter(
    (candidate): candidate is TeamStore =>
      candidate.kind === 'team' &&
      candidate.server === store.server &&
      candidate.account === store.account,
  );
}

export interface PeopleScreenProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'people' }>;
  /**
   * `force` marks the replacement this page makes on its own behalf — the
   * default route resolving to an account's exact `StoreRef` — which is the
   * page already on screen writing its own address and not a move any panel
   * may answer for.
   */
  onNavigate: (location: Location, options?: NavigateOptions) => void;
  onRefresh: (message: string) => Promise<void>;
  onRefreshSnapshot: () => Promise<AgentSnapshot>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
  onLock?: () => void;
}

export function PeopleScreen({
  snapshot,
  bridge,
  location,
  onNavigate,
  onRefresh,
  onRefreshSnapshot,
  onError,
  onMutationError,
  onLock,
}: PeopleScreenProps): ReactNode {
  const stores = accountStores(snapshot);
  // Select by exact StoreRef: two servers may both hold an account aliased
  // `personal`, so an alias would not say which one an address means.
  const requested = location.store;
  const selected = requested
    ? stores.find((store) => store.id === requested)
    : stores[0];
  const unavailable = requested !== undefined && selected === undefined;
  const [sheet, setSheet] = useTabSheetState<
    AccountSheet | 'go-profile' | null
  >('people.sheet', null, false);
  const [pairingProfile, setPairingProfile] = useState<Server | undefined>();
  // What this account holds: the keys are read here because the profile's
  // Devices row counts them, not because anything on this page acts on one.
  const stopped = selected
    ? accountStopped(snapshot, selected)
    : { stopped: true, reason: '' };
  const {
    lists,
    loading: loadingKeys,
    failed: keysFailed,
  } = useDeviceMetadata({
    bridge,
    profile: selected?.server,
    store: selected?.id,
    enabled: !stopped.stopped,
    recovery: { refresh: onRefreshSnapshot },
    onError,
  });
  // The Devices rail indicator initiates no background request; it reflects
  // cached results from the account key list query.
  useEffect(() => {
    if (!selected || stopped.stopped || loadingKeys || keysFailed) return;
    deviceAlertRegistry(bridge).reportPaperKey(
      selected.id,
      lists.backups.length > 0,
    );
  }, [
    bridge,
    selected,
    stopped.stopped,
    loadingKeys,
    keysFailed,
    lists.backups.length,
  ]);

  // Secrets typed into a panel must not stay on screen behind another window.
  // Browser sign-in is the exception: it hands focus away on purpose.
  const open = useRef(sheet);
  open.current = sheet;
  useEffect(() => {
    const conceal = (): void => {
      if (
        open.current === 'local-alias' ||
        open.current === 'go-profile' ||
        open.current === 'sso'
      )
        return;
      setSheet(null);
    };
    const concealWhenHidden = (): void => {
      if (document.hidden) conceal();
    };
    window.addEventListener('blur', conceal);
    document.addEventListener('visibilitychange', concealWhenHidden);
    return () => {
      window.removeEventListener('blur', conceal);
      document.removeEventListener('visibilitychange', concealWhenHidden);
    };
  }, [setSheet]);

  // Normalize the route to the active account's StoreRef. Because the
  // destination matches the current location, navigation guards are bypassed
  // and active dialogs remain open.
  useEffect(() => {
    if (location.store || !selected) return;
    onNavigate({ ...location, store: selected.id }, { force: true });
  }, [location, onNavigate, selected]);

  // Changing account identity resets input state; query data is selected by
  // exact account/profile identity on the same render, without copying it.
  useEffect(() => {
    setSheet(null);
  }, [selected?.id, setSheet]);

  return (
    <>
      <PageHeader
        ruled
        title="Account"
        action={
          <Button
            onClick={() => {
              setPairingProfile(undefined);
              setSheet('go-profile');
            }}
          >
            Connect from FOKS CLI…
          </Button>
        }
      />
      <div className="body">
        <div className="settings-main">
          <UnroutedNotices
            snapshot={snapshot}
            onRefreshSnapshot={onRefreshSnapshot}
            onError={onError}
          />
          {snapshot.servers
            .filter((server) => server.accounts.length === 0)
            .map((server) => (
              <Inset key={server.id}>
                <InsetRow
                  label={serverDisplayName(server)}
                  action={
                    <Button
                      onClick={() => {
                        setPairingProfile(server);
                        setSheet('go-profile');
                      }}
                    >
                      Pair
                    </Button>
                  }
                >
                  Connected, not yet paired
                </InsetRow>
              </Inset>
            ))}
          {unavailable ? (
            <UnavailableAccount
              stores={stores}
              snapshot={snapshot}
              onSelect={(store) =>
                onNavigate({ kind: 'people', store: store.id })
              }
              onRefresh={() => void onRefreshSnapshot().catch(onError)}
            />
          ) : selected ? (
            <AccountPanel
              accountSelector={
                <AccountHeader
                  compact
                  snapshot={snapshot}
                  location={location}
                  account={selected.id}
                  onNavigate={onNavigate}
                  onLock={onLock}
                />
              }
              snapshot={snapshot}
              store={selected}
              lists={lists}
              loading={loadingKeys}
              failed={keysFailed}
              stopped={stopped}
              onNavigate={onNavigate}
              onSheet={setSheet}
            />
          ) : (
            <NoAvailableAccount
              onConnectGoProfile={() => setSheet('go-profile')}
            />
          )}
        </div>
      </div>
      {sheet === 'go-profile' ? (
        <GoProfileConnectSheet
          bridge={bridge}
          existingProfile={pairingProfile}
          onAdded={async () => {
            setSheet(null);
            await onRefresh('Server connected. Pair an account to continue.');
          }}
          onClose={() => setSheet(null)}
          onConnected={async (_profile, alias) => {
            setSheet(null);
            await onRefresh(`Connected account "${alias}" from FOKS CLI`);
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {selected && sheet === 'local-alias' ? (
        <LocalAliasPanel
          key={selected.id}
          bridge={bridge}
          store={selected.id}
          alias={localAliasOf(snapshot, selected)}
          presentation={{
            title: 'Change local alias',
            onClose: () => setSheet(null),
          }}
          onComplete={async () => {
            await onRefresh('Local alias updated');
            setSheet(null);
          }}
        />
      ) : null}
      {selected && sheet === 'rename' ? (
        <RenamePanel
          bridge={bridge}
          profile={selected.server}
          account={selected.account}
          presentation={{
            title: 'Change username',
            onClose: () => setSheet(null),
          }}
          onComplete={() => onRefresh('Username updated')}
        />
      ) : null}
      {selected && sheet === 'bot' ? (
        <BotPanel
          bridge={bridge}
          profile={selected.server}
          account={selected.account}
          presentation={{
            title: 'Bot accounts',
            onClose: () => setSheet(null),
          }}
          onComplete={() => onRefresh('Bot account updated')}
        />
      ) : null}
      {selected && sheet === 'admin' ? (
        <AdminPanel
          bridge={bridge}
          profile={selected.server}
          account={selected.account}
          presentation={{
            title: 'Manage via web',
            onClose: () => setSheet(null),
          }}
        />
      ) : null}
      {selected && sheet === 'sso' ? (
        <SsoPanel
          bridge={bridge}
          profile={selected.server}
          account={selected.account}
          login={true}
          presentation={{
            title: 'Organization sign-in',
            onClose: () => setSheet(null),
          }}
          onComplete={() => onRefresh('Organization sign-in verified')}
        />
      ) : null}
    </>
  );
}

/** The chosen account: who it is, and the facts and workflows that act on it. */
function AccountPanel({
  snapshot,
  store,
  lists,
  loading,
  failed,
  stopped,
  onNavigate,
  onSheet,
  accountSelector,
}: {
  snapshot: AgentSnapshot;
  store: AccountStore;
  /** The keys this account holds, once the agent has answered. */
  lists: DeviceLists;
  loading: boolean;
  /** Whether the read failed, which is not the same as holding no keys. */
  failed: boolean;
  stopped: { stopped: boolean; reason: string };
  onNavigate: (location: Location) => void;
  onSheet: (sheet: AccountSheet) => void;
  accountSelector: ReactNode;
}): ReactNode {
  const username = usernameOf(snapshot, store);
  const reason = stopped.stopped ? stopped.reason : undefined;
  const deviceCount = deviceEntries(lists).length;
  const teams = teamsOnAccount(snapshot, store);
  const server = snapshot.servers.find((entry) => entry.id === store.server);
  return (
    <>
      {/* The profile head: the account's mark, the four facts that name it,
          and the button that switches to another account. */}
      <div className="phead">
        <AccountMark name={username ?? store.account} size="big" />
        <span className="t">
          <h2>{username ?? 'Identity unavailable'}</h2>
          {/* The alias and the availability qualify the server line, so they
              sit on it rather than at the far end of the band. */}
          <span className="line">
            <small>on {serverName(snapshot, store)}</small>
            <Chip>{localAliasOf(snapshot, store)}</Chip>
            {stopped.stopped ? (
              <Chip tone="warn">{storeDescription(snapshot, store)}</Chip>
            ) : null}
          </span>
        </span>
        {accountSelector}
      </div>
      {stopped.stopped ? (
        <Band
          severity="crit"
          label="Account access is stopped"
          action={
            <Button
              size="sm"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'servers',
                  profile: store.server,
                })
              }
            >
              Open the server…
            </Button>
          }
        >
          Nothing on this account can be read, changed or verified until{' '}
          {serverName(snapshot, store)} is checked.
        </Band>
      ) : null}
      <Inset className="settings-inset wide">
        <InsetRow
          label="Username"
          action={
            <Button
              size="sm"
              disabled={stopped.stopped}
              title={reason}
              onClick={() => onSheet('rename')}
            >
              Change…
            </Button>
          }
        >
          <b>{username ?? 'Identity unavailable'}</b>
        </InsetRow>
        <InsetRow
          label="Shown as"
          action={
            <Button
              size="sm"
              aria-label="Change local alias"
              disabled={stopped.stopped}
              title={reason}
              onClick={() => onSheet('local-alias')}
            >
              Change…
            </Button>
          }
        >
          {localAliasOf(snapshot, store)}
        </InsetRow>
        <InsetRow
          label="Server"
          action={
            <Button
              size="sm"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'servers',
                  profile: store.server,
                  store: store.id,
                })
              }
            >
              Settings › Servers
            </Button>
          }
        >
          {stopped.stopped
            ? `${serverName(snapshot, store)} · ${storeDescription(snapshot, store)}`
            : server?.trust.status === 'verified'
              ? `${serverName(snapshot, store)} · verified`
              : serverName(snapshot, store)}
        </InsetRow>
        <InsetRow
          label="Devices"
          action={
            <Button
              size="sm"
              disabled={stopped.stopped}
              title={reason}
              onClick={() => onNavigate({ kind: 'devices', store: store.id })}
            >
              Devices ›
            </Button>
          }
        >
          {stopped.stopped
            ? 'Not listed while access is stopped'
            : loading
              ? 'Reading this account’s keys…'
              : failed
                ? 'Devices and keys could not be read.'
                : plural(deviceCount, 'device and key', 'devices and keys')}
        </InsetRow>
        <InsetRow
          label="Teams"
          action={
            <Button
              size="sm"
              onClick={() => onNavigate({ kind: 'teams', store: store.id })}
            >
              Teams ›
            </Button>
          }
        >
          {teams.length ? teams.map((team) => team.name).join(', ') : 'None'}
        </InsetRow>
      </Inset>
      <p className="fn">
        More:{' '}
        <Button
          variant="plain"
          size="sm"
          className="lnk"
          onClick={() => onSheet('bot')}
        >
          Bot accounts
        </Button>
        {' · '}
        <Button
          variant="plain"
          size="sm"
          className="lnk"
          onClick={() => onSheet('admin')}
        >
          Manage via web
        </Button>
        {' · '}
        <Button
          variant="plain"
          size="sm"
          className="lnk"
          onClick={() => onSheet('sso')}
        >
          Organization sign-in
        </Button>
      </p>
    </>
  );
}

/** Displayed when the account an address names is not in the catalog. */
export function UnavailableAccount({
  stores,
  snapshot,
  onSelect,
  onRefresh,
}: {
  stores: readonly AccountStore[];
  snapshot: AgentSnapshot;
  onSelect: (store: AccountStore) => void;
  onRefresh: () => void;
}): ReactNode {
  return (
    <Notice
      severity="crit"
      title="Account no longer available"
      actions={
        <>
          <Button variant="primary" onClick={onRefresh}>
            Refresh
          </Button>
          {stores.map((store) => (
            <Button key={store.id} onClick={() => onSelect(store)}>
              {localAliasOf(snapshot, store)} · {serverName(snapshot, store)}
            </Button>
          ))}
        </>
      }
    >
      <p>
        This account is no longer available on this device. Select another
        account below or refresh.
      </p>
      {stores.length ? null : (
        <p>
          This device has no active account. Add and verify a server, then
          create or recover an account.
        </p>
      )}
    </Notice>
  );
}

/** Displayed when this Mac holds no account at all. */
function NoAvailableAccount({
  onConnectGoProfile,
}: {
  onConnectGoProfile: () => void;
}): ReactNode {
  return (
    <Notice
      title="No available account on this device"
      actions={
        <Button variant="primary" onClick={onConnectGoProfile}>
          Connect existing account
        </Button>
      }
    >
      <p>
        Add and verify a server, then create or recover an account, or connect
        an existing FOKS account on your device.
      </p>
    </Notice>
  );
}
