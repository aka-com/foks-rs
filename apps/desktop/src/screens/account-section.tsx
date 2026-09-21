import { useDeviceMetadata } from '../device-cache';
import type { metadataFreshness } from '../device-cache';
import { WorkflowProvider, useWorkflowAccess } from '../workflow-context';
import { workflowAvailability } from '../model/workflow-availability';
import { FreshnessCaption } from '../components/metadata-status';
import { LocalAliasPanel } from '../components/local-alias-panel';
import { localAliasOf } from '../model';
import { useTabSheetState } from '../navigation-guard';
/**
 * Settings › Account: one account's profile, the account the address's
 * `store` names and the rail header shows.
 *
 * The account is the section's header: its mark, its username as the title,
 * and the server and the server's trust state on the line under it — the
 * shape the team page already uses for a team. The header has no switcher:
 * the rail header's avatar menu is the application's only account switcher,
 * and it sits beside this section. The body displays primary account
 * properties with management links: Username, Shown as (the local alias),
 * Server, Devices, and Teams. Secondary integrations (Bot accounts, web
 * administration, SSO, FOKS CLI import, account recovery, and hardware-backed
 * account creation) render after the device-wide server inventory. The
 * selected account's passphrase is managed here as well.
 * Alerts without a navigable destination, such as catalog read failures,
 * render above the details list; entity-specific alerts render in their
 * corresponding Settings or Teams views.
 *
 * The section draws its own header and tab panel, because its header is an
 * identity rather than the section's name, which is what the other sections'
 * shared header draws.
 */

import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { AdminPanel } from '../components/admin-panel';
import { BotPanel } from '../components/bot-panel';
import { RenamePanel } from '../components/rename-panel';
import { SsoPanel } from '../components/sso-panel';
import {
  Band,
  Button,
  Chip,
  Icon,
  Inset,
  InsetRow,
  Notice,
} from '../components';
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
import { deviceEntries } from './device-model';
import type { DeviceLists } from './device-model';
import { GoProfileConnectSheet } from './go-profile-connect';
import { EnrollSheet, PassphraseSheet, RecoverSheet } from './device-sheets';

const ACTION_UNAVAILABLE =
  'Resolve this under Account servers, or in the team’s settings.';

/** The account panels reached from a row on this page. */
type AccountSheet =
  | 'local-alias'
  | 'rename'
  | 'passphrase'
  | 'bot'
  | 'admin'
  | 'sso'
  | 'recover'
  | 'enroll'
  | 'go-profile';

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
      label: 'Open server',
      where: `Settings › Account › ${serverDisplayName(namedServer)}`,
      location: {
        kind: 'settings',
        section: 'account',
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
 * Settings › Account, a team whose setup is incomplete shows on Teams, and a
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

export interface AccountSectionProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  /** The Settings address this section is open at; `store` is the account. */
  location: Extract<Location, { kind: 'settings' }>;
  /**
   * The id the sub-navigation's Account tab points at, and the tab's own id,
   * so the panel this section draws is the one the tab controls.
   */
  panel: { id: string; labelledBy: string };
  /**
   * `force` marks the replacement this page makes on its own behalf — the
   * default route resolving to an account's exact `StoreRef` — which is the
   * page already on screen writing its own address and not a move any panel
   * may answer for.
   */
  onNavigate: (location: Location, options?: NavigateOptions) => void;
  /**
   * Refreshes data after a mutation. When a profile is specified, only that
   * server profile is reloaded instead of refreshing the full catalog.
   */
  onRefresh: (message: string, profile?: string) => Promise<void>;
  onRefreshSnapshot: () => Promise<AgentSnapshot>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
  /** Device-wide server inventory, composed after every account state. */
  serverSection?: ReactNode;
}

export function AccountSection({
  snapshot,
  bridge,
  location,
  panel,
  onNavigate,
  onRefresh,
  onRefreshSnapshot,
  onError,
  onMutationError,
  serverSection,
}: AccountSectionProps): ReactNode {
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
  >('settings.account.sheet', null, false);
  const [pairingProfile, setPairingProfile] = useState<Server | undefined>();
  // What this account holds: the keys are read here because the profile's
  // Devices row counts them, not because anything on this page acts on one.
  const stopped = selected
    ? accountStopped(snapshot, selected)
    : { stopped: true, reason: '' };
  const keysStopped =
    !selected ||
    !workflowAvailability(snapshot, 'devices-list', {
      profile: selected.server,
      account: selected.account,
    }).available;
  const {
    lists,
    loading: loadingKeys,
    failed: keysFailed,
    freshness,
    retry: retryMetadata,
  } = useDeviceMetadata({
    bridge,
    snapshot,
    profile: selected?.server,
    store: selected?.id,
    enabled: !keysStopped,
    recovery: { refresh: onRefreshSnapshot },
    onError,
  });

  // Normalize the route to the active account's StoreRef. Because the
  // destination matches the current location, navigation guards are bypassed
  // and active dialogs remain open.
  useEffect(() => {
    if (location.store || !selected) return;
    onNavigate(
      { ...location, section: 'account', store: selected.id },
      { force: true },
    );
  }, [location, onNavigate, selected]);

  // Changing account identity resets input state; query data is selected by
  // exact account/profile identity on the same render, without copying it.
  useEffect(() => {
    setSheet(null);
  }, [selected?.id, setSheet]);

  // The page's header is the account the page is about: its mark, its username
  // as the title, and the server on the line under it. The branches that name no account — an address
  // naming one this Mac no longer holds, or no account at all — have no
  // identity to state, so they keep the tab's own name as the title.
  //
  // Whether access is stopped is a fact about the header's subject rather than
  // part of the account's name or of its server line, so it is the header's
  // own right-hand mark: at the end of the row, centred against both lines,
  // instead of trailing the server on the second one.
  const headerUsername = selected ? usernameOf(snapshot, selected) : undefined;
  const identity = selected
    ? {
        title: headerUsername ?? 'Identity unavailable',
        mark: (
          <AccountMark name={headerUsername ?? selected.account} size="round" />
        ),
        sub: <span>{serverName(snapshot, selected)}</span>,
        state: stopped.stopped ? (
          <Chip tone="warn">{storeDescription(snapshot, selected)}</Chip>
        ) : undefined,
      }
    : undefined;

  // The fallback band belongs above the facts of the account the page names.
  // The branches that state no identity draw no facts either, so they keep it
  // at the top of the page.
  const notices = (
    <UnroutedNotices
      snapshot={snapshot}
      onRefreshSnapshot={onRefreshSnapshot}
      onError={onError}
    />
  );
  const unpairedServers =
    snapshot.profileInventoryStatus === 'complete'
      ? snapshot.servers.filter(
          (server) =>
            server.accounts.length === 0 &&
            snapshot.profileInventory.find(
              (inventory) => inventory.profile === server.id,
            )?.accounts === 'complete',
        )
      : [];
  const noticesAndUnpairedServers = (
    <>
      {notices}
      {unpairedServers.map((server) => (
        <Inset key={server.id}>
          {/* The server's name is the value, not the label: the label column
              is a fixed 88px and a hostname overran it. */}
          <InsetRow
            label="Server"
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
            {serverDisplayName(server)}{' '}
            <span className="dim">Connected, not paired</span>
          </InsetRow>
        </Inset>
      ))}
    </>
  );

  return (
    <WorkflowProvider snapshot={snapshot}>
      <PageHeader
        ruled
        title={identity?.title ?? 'Account'}
        mark={identity?.mark}
        sub={identity?.sub}
        action={identity?.state}
      />
      <div
        className="body"
        role="tabpanel"
        id={panel.id}
        aria-labelledby={panel.labelledBy}
      >
        <div className="settings-main account-main">
          {unavailable ? (
            <>
              {noticesAndUnpairedServers}
              <UnavailableAccount
                stores={stores}
                snapshot={snapshot}
                onSelect={(store) =>
                  onNavigate({
                    kind: 'settings',
                    section: 'account',
                    store: store.id,
                  })
                }
                onRefresh={() => void onRefreshSnapshot().catch(onError)}
              />
            </>
          ) : selected ? (
            <>
              <AccountPanel
                notices={noticesAndUnpairedServers}
                freshness={freshness}
                onRetry={retryMetadata}
                snapshot={snapshot}
                store={selected}
                lists={lists}
                loading={loadingKeys}
                failed={keysFailed}
                stopped={stopped}
                onNavigate={onNavigate}
                onSheet={(next) => {
                  // The import sheet opened from the account's own line adds an
                  // account; a server row's Pair fills `pairingProfile` instead.
                  if (next === 'go-profile') setPairingProfile(undefined);
                  setSheet(next);
                }}
              />
            </>
          ) : (
            <>
              {noticesAndUnpairedServers}
              <NoAvailableAccount
                onConnectGoProfile={() => setSheet('go-profile')}
              />
            </>
          )}
          {serverSection}
          {selected ? (
            <AccountActions
              snapshot={snapshot}
              store={selected}
              onSheet={(next) => {
                if (next === 'go-profile') setPairingProfile(undefined);
                setSheet(next);
              }}
            />
          ) : null}
        </div>
      </div>
      {sheet === 'go-profile' ? (
        <GoProfileConnectSheet
          bridge={bridge}
          existingProfile={pairingProfile}
          onAdded={async (profile) => {
            setSheet(null);
            await onRefresh(
              'Server connected. Pair an account to continue.',
              profile,
            );
          }}
          onClose={() => setSheet(null)}
          onConnected={async (profile, alias) => {
            setSheet(null);
            await onRefresh(
              `Connected account "${alias}" from FOKS CLI`,
              profile,
            );
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
            // Close the sheet immediately after saving without waiting for the
            // background catalog refresh to prevent blocking user navigation.
            setSheet(null);
            await onRefresh('Local alias updated', selected.server);
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
          onComplete={() => onRefresh('Username updated', selected.server)}
        />
      ) : null}
      {selected && sheet === 'passphrase' ? (
        <PassphraseSheet
          bridge={bridge}
          store={selected}
          onClose={() => setSheet(null)}
          onDone={(message) => {
            setSheet(null);
            void onRefresh(message, selected.server).catch((error) =>
              onMutationError(error),
            );
          }}
          onError={(error) => void onMutationError(error)}
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
          onComplete={() => onRefresh('Bot account updated', selected.server)}
        />
      ) : null}
      {selected && sheet === 'admin' ? (
        <AdminPanel
          bridge={bridge}
          profile={selected.server}
          account={selected.account}
          presentation={{
            title: 'Web admin panel',
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
            title: 'Sign in via SSO',
            onClose: () => setSheet(null),
          }}
          onComplete={() => onRefresh('SSO sign-in verified', selected.server)}
        />
      ) : null}
      {selected && sheet === 'recover' ? (
        <RecoverSheet
          bridge={bridge}
          store={selected}
          onClose={() => setSheet(null)}
          onDone={async () => {
            setSheet(null);
            await onRefresh(
              'Recovery submitted successfully.',
              selected.server,
            );
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {selected && sheet === 'enroll' ? (
        <EnrollSheet
          bridge={bridge}
          store={selected}
          card={snapshot.cardsConnected[0]}
          onClose={() => setSheet(null)}
          onDone={async () => {
            setSheet(null);
            await onRefresh(
              'Hardware-key account created.',
              selected.server,
            );
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
    </WorkflowProvider>
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
  notices,
  freshness,
  onRetry,
}: {
  snapshot: AgentSnapshot;
  store: AccountStore;
  /** The keys this account holds, once the agent has answered. */
  lists: DeviceLists;
  /** Whether the keys behind the counts are current, stale or on their way. */
  freshness: ReturnType<typeof metadataFreshness>;
  onRetry: () => void;
  loading: boolean;
  /** Whether the read failed, which is not the same as holding no keys. */
  failed: boolean;
  stopped: { stopped: boolean; reason: string };
  onNavigate: (location: Location) => void;
  onSheet: (sheet: AccountSheet) => void;
  /** The notices no tab can resolve, drawn above the facts they qualify. */
  notices: ReactNode;
}): ReactNode {
  const username = usernameOf(snapshot, store);
  const access = useWorkflowAccess(snapshot);
  const target = { profile: store.server, account: store.account };
  const devicesAccess = access.props('devices-list', target);
  const devicesStopped = {
    stopped: devicesAccess.disabled,
    reason: devicesAccess.title,
  };
  const deviceCount = deviceEntries(lists).length;
  const teams = teamsOnAccount(snapshot, store);
  const server = snapshot.servers.find((entry) => entry.id === store.server);
  // The account this page is about is named by the page's own header, so the
  // body opens on the facts rather than stating the identity a second time.
  return (
    <>
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
                  section: 'account',
                  profile: store.server,
                })
              }
            >
              Open server
            </Button>
          }
        >
          Remote operations are unavailable until a connection is reestablished.
        </Band>
      ) : null}
      {notices}
      <Inset
        className={`settings-inset middle wide${freshness.stale ? ' stale' : ''}`}
      >
        <InsetRow
          label="Username"
          action={
            <>
              <Button
                size="sm"
                {...access.props('account-rename', target)}
                onClick={() => onSheet('rename')}
              >
                Change…
              </Button>
              <Button
                size="sm"
                {...access.props('passphrase', target)}
                onClick={() => onSheet('passphrase')}
              >
                Passphrase…
              </Button>
            </>
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
              {...access.props('local-alias', target)}
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
              className="account-fact-link"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'account',
                  profile: store.server,
                  store: store.id,
                })
              }
            >
              Server details ›
            </Button>
          }
        >
          {/* The server names itself; its state is a quieter second phrase
              after it, not a clause of equal weight behind a middot. The two
              share one element because this inset stacks its value's children,
              and the state belongs on the name's own line. */}
          <span>
            {serverName(snapshot, store)}
            {stopped.stopped ? (
              <>
                {' '}
                <span className="dim">{storeDescription(snapshot, store)}</span>
              </>
            ) : server?.trust.status === 'verified' ? (
              <span className="verified-mark" role="img" aria-label="Verified">
                <Icon name="circleCheck" />
              </span>
            ) : null}
          </span>
        </InsetRow>
        <InsetRow
          label="Devices"
          action={
            <Button
              size="sm"
              className="account-fact-link"
              disabled={devicesStopped.stopped}
              title={devicesStopped.stopped ? devicesStopped.reason : undefined}
              onClick={() => onNavigate({ kind: 'devices', store: store.id })}
            >
              Devices ›
            </Button>
          }
        >
          {devicesStopped.stopped
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
              className="account-fact-link"
              onClick={() => onNavigate({ kind: 'teams', store: store.id })}
            >
              Teams ›
            </Button>
          }
        >
          {plural(teams.length, 'team')}
        </InsetRow>
      </Inset>
      <FreshnessCaption
        label="device metadata"
        freshness={freshness}
        onRetry={onRetry}
      />
    </>
  );
}

/** Secondary account workflows, placed after the device-wide server list. */
function AccountActions({
  snapshot,
  store,
  onSheet,
}: {
  snapshot: AgentSnapshot;
  store: AccountStore;
  onSheet: (sheet: AccountSheet) => void;
}): ReactNode {
  const access = useWorkflowAccess(snapshot);
  const target = { profile: store.server, account: store.account };
  return (
    <div className="fn account-more">
      <Button
        variant="plain"
        size="sm"
        className="lnk"
        {...access.props('bot-list', target)}
        onClick={() => onSheet('bot')}
      >
        Bot accounts
      </Button>
      <Button
        variant="plain"
        size="sm"
        className="lnk"
        {...access.props('web-admin-configure', target)}
        onClick={() => onSheet('admin')}
      >
        Web admin panel
      </Button>
      <Button
        variant="plain"
        size="sm"
        className="lnk"
        {...access.props('sso-login', target)}
        onClick={() => onSheet('sso')}
      >
        Sign in via SSO
      </Button>
      <Button
        variant="plain"
        size="sm"
        className="lnk"
        onClick={() => onSheet('go-profile')}
      >
        Import from FOKS CLI
      </Button>
      <Button
        variant="plain"
        size="sm"
        className="lnk"
        {...access.props('account-recover', { profile: store.server })}
        onClick={() => onSheet('recover')}
      >
        Connect an existing account with a paper key
      </Button>
      <Button
        variant="plain"
        size="sm"
        className="lnk"
        {...access.props('yubi-create', { profile: store.server })}
        onClick={() => onSheet('enroll')}
      >
        Create an account on a hardware key…
      </Button>
    </div>
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
      {/* The notice carries its actions at the right, so the sentence no
          longer points below itself for them. */}
      <p>
        This account is no longer available on this device. Refresh, or select
        another account.
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
