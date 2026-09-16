/**
 * The Accounts tab: what needs attention, then the accounts on this Mac.
 *
 * `AttentionList` is the page that used to be called Alerts. It is one amber
 * card: a head that names the region and counts the open notes, then a row per
 * note. Each row describes the issue's effect, identifies where to resolve it,
 * and provides the action at the right end. The catalog action retries; every
 * other action opens the relevant page. With no notes
 * the card is replaced by a single green line. Under it, the account switcher
 * chooses which account the page is about, and that account's facts and
 * workflows are rows with their action at the right — the rows Settings ›
 * Accounts had, with the same panels behind them.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { AdminPanel } from '../components/admin-panel';
import { BotPanel } from '../components/bot-panel';
import { InvitationPanel } from '../components/invitation-panel';
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
  SectionLabel,
} from '../components';
import { normalizeCommandError } from '../bridge';
import type { Bridge } from '../bridge';
import {
  accountStopped,
  accountStores,
  accountSubtitle,
  notesNow,
  parseRole,
  partiesOf,
  plural,
  roleName,
  serverDisplayName,
  serverName,
  shortId,
  storeAttentionState,
  storeDescription,
  storeNavigationOrder,
  teamCaption,
  usernameOf,
} from '../model';
import type {
  AccountStore,
  AgentSnapshot,
  Notification,
  TeamStore,
} from '../model';
import type { Server } from '../model';
import type { FoksIconName } from '../icons';
import type { Location, NavigateOptions } from '../location';
import type { MutationFailureHandler } from '../mutation-recovery';
import { PageHeader } from '../shell/page-header';
import { AccountMark, AccountSwitcher } from './account-switcher';
import {
  deviceEntries,
  NO_DEVICES,
  readAccountAndProfileKeys,
} from './device-model';
import type { DeviceEntry, DeviceLists } from './device-model';
import { GroupMark } from './group-mark';
import { GoProfileConnectSheet } from './go-profile-connect';

const ACTION_UNAVAILABLE =
  'Resolve this in Settings › Servers, or in the group’s settings.';

/** The accessible name for the severity indicator. */
const SEVERITY_LABELS: Record<Notification['severity'], string> = {
  crit: 'Critical',
  warn: 'Warning',
  info: 'Information',
};

/** The account panels reached from a row on this page. */
type AccountSheet = 'rename' | 'bot' | 'admin' | 'join' | 'sso';

interface AttentionListProps {
  snapshot: AgentSnapshot;
  onNavigate: (location: Location) => void;
  onRefreshSnapshot: () => Promise<AgentSnapshot>;
  onError: (error: unknown) => void;
}

function canRetry(note: Notification): boolean {
  return note.action === 'Retry' && note.id.startsWith('catalog-');
}

/**
 * The glyph on a row's mark, chosen by what the note is about: a group whose
 * setup is unfinished, a group's membership in another group, or a server this
 * Mac has not checked in with. A note of any other kind keeps the generic
 * warning glyph.
 */
function markGlyph(note: Notification): FoksIconName {
  if (note.id.startsWith('team-')) return 'flag';
  if (note.id.startsWith('fed-')) return 'people';
  if (note.id.startsWith('lease-')) return 'server';
  return 'alert';
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

/**
 * Where a note is resolved, when its id names a place this Mac holds: a
 * `lease-` note names a server profile, a `team-` note names the group that is
 * not set up, and a `fed-` note names the *admitted* group, whose admission is
 * resolved in the host group it was admitted to. A note whose place cannot be
 * derived keeps the label it always had rather than opening the wrong page.
 */
function destinationOf(
  snapshot: AgentSnapshot,
  note: Notification,
): Destination | null {
  if (note.id.startsWith('lease-')) {
    const profile = note.id.slice('lease-'.length);
    const server = snapshot.servers.find((entry) => entry.id === profile);
    if (!server) return null;
    return {
      label: 'Open the server',
      where: `Settings › Servers › ${serverDisplayName(server)}`,
      location: { kind: 'settings', section: 'servers', profile: server.id },
    };
  }
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

/** Active warnings and blocked operations, each with the action it allows. */
function AttentionList({
  snapshot,
  onNavigate,
  onRefreshSnapshot,
  onError,
}: AttentionListProps): ReactNode {
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const notes = notesNow(snapshot);

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
    <section
      className="people-attention"
      aria-labelledby="people-attention-label"
    >
      {notes.length ? (
        <div className="attn-card">
          {/* The card's head is the region's label: there is no separate
              section label over it. */}
          <div className="attn-head">
            <Icon name="alert" />
            <span id="people-attention-label">Needs attention</span>
            <span className="grow" />
            <span className="n">{notes.length}</span>
          </div>
          {notes.map((note) => {
            const destination = canRetry(note)
              ? null
              : destinationOf(snapshot, note);
            return (
              <div className="attn-row" key={note.id}>
                {/* The severity is a colour, so it is also a name: a reader
                    who cannot see the colour is told the same thing. */}
                <span
                  className="mk"
                  role="img"
                  aria-label={SEVERITY_LABELS[note.severity]}
                >
                  <Icon name={markGlyph(note)} />
                </span>
                <div className="t">
                  <b>{note.title}</b>
                  <small>{note.detail}</small>
                  {/* The same place the action button opens, named as the
                      reader would read the path aloud. */}
                  {destination ? (
                    <button
                      type="button"
                      className="go"
                      onClick={() => onNavigate(destination.location)}
                    >
                      {destination.where}
                    </button>
                  ) : null}
                </div>
                {/* Not the design's `.acts`: that class is the hover-only
                    icon strip a tile carries, and it hides what it holds. */}
                <div className="attn-acts">
                  {canRetry(note) ? (
                    <Button
                      variant="primary"
                      disabled={busy.has(note.id)}
                      title="Retry loading catalog"
                      onClick={() => retry(note)}
                    >
                      {note.action}
                    </Button>
                  ) : destination ? (
                    <Button
                      variant="primary"
                      onClick={() => onNavigate(destination.location)}
                    >
                      {destination.label}
                    </Button>
                  ) : note.action ? (
                    <Chip tone="warn" title={ACTION_UNAVAILABLE}>
                      {note.action}
                    </Chip>
                  ) : null}
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        <p className="allclear">
          <Icon name="check" />
          <span id="people-attention-label">Nothing needs attention.</span>
        </p>
      )}
    </section>
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
}: PeopleScreenProps): ReactNode {
  const stores = accountStores(snapshot);
  // Select by exact StoreRef: two servers may both hold an account aliased
  // `personal`, so an alias would not say which one an address means.
  const requested = location.store;
  const selected = requested
    ? stores.find((store) => store.id === requested)
    : stores[0];
  const unavailable = requested !== undefined && selected === undefined;
  const [sheet, setSheet] = useState<AccountSheet | 'go-profile' | null>(null);
  const [pairingProfile, setPairingProfile] = useState<Server | undefined>();
  // What this account holds: the keys are read here because the profile lists
  // them, not because anything on this page acts on one.
  const [lists, setLists] = useState<DeviceLists>(NO_DEVICES);
  const [loadingKeys, setLoadingKeys] = useState(true);
  // A read that failed is not an account with no keys, and the sections say
  // which of the two this is.
  const [keysFailed, setKeysFailed] = useState(false);
  const stopped = selected
    ? accountStopped(snapshot, selected)
    : { stopped: true, reason: '' };

  // Secrets typed into a panel must not stay on screen behind another window.
  // Browser sign-in is the exception: it hands focus away on purpose.
  const open = useRef(sheet);
  open.current = sheet;
  useEffect(() => {
    const conceal = (): void => {
      if (open.current === 'go-profile' || open.current === 'sso') return;
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
  }, []);

  // Normalize the route to the active account's StoreRef. Because the
  // destination matches the current location, navigation guards are bypassed
  // and active dialogs remain open.
  useEffect(() => {
    if (location.store || !selected) return;
    onNavigate({ ...location, store: selected.id }, { force: true });
  }, [location, onNavigate, selected]);

  // A catalog the agent has replaced mid-read is recovered by the shell's own
  // refresh, once, and the read is retried once; concurrent requests share the
  // one refresh. The shell's callback is held in a ref: a caller that passes a
  // new function each render would otherwise re-read this account's keys on
  // every render of the shell.
  const refreshSnapshot = useRef(onRefreshSnapshot);
  refreshSnapshot.current = onRefreshSnapshot;
  const catalogRecovery = useRef<Promise<AgentSnapshot> | null>(null);
  const recovered = useRef(new Set<string>());
  const recoverCatalog = useCallback((): Promise<AgentSnapshot> => {
    if (!catalogRecovery.current) {
      const pending = refreshSnapshot.current().finally(() => {
        if (catalogRecovery.current === pending) catalogRecovery.current = null;
      });
      catalogRecovery.current = pending;
    }
    return catalogRecovery.current;
  }, []);

  // The facts on this page the catalog does not carry: the keys this account
  // holds, and which of them this Mac is authenticated with. This page lists
  // no connected card, so it does not drive the card reader to find one.
  const selectedId = selected?.id;
  const selectedProfile = selected?.server;
  const accessStopped = stopped.stopped;
  useEffect(() => {
    let alive = true;
    setLists(NO_DEVICES);
    setKeysFailed(false);
    setSheet(null);
    if (!selectedId || !selectedProfile || accessStopped) {
      setLoadingKeys(false);
      return;
    }
    setLoadingKeys(true);
    const load = (): Promise<DeviceLists> =>
      readAccountAndProfileKeys(bridge, selectedProfile, selectedId, {
        cards: false,
      });
    void (async () => {
      try {
        let result: DeviceLists;
        try {
          result = await load();
        } catch (error) {
          if (!alive) return;
          if (normalizeCommandError(error).code !== 'catalog-required')
            throw error;
          const already = recovered.current.has(selectedId);
          if (already && !catalogRecovery.current) throw error;
          recovered.current.add(selectedId);
          await recoverCatalog();
          if (!alive) return;
          result = await load();
        }
        if (!alive) return;
        recovered.current.delete(selectedId);
        setLists(result);
        setLoadingKeys(false);
      } catch (error) {
        if (!alive) return;
        setLoadingKeys(false);
        // The sections say the read failed rather than reporting no keys; the
        // failure itself is the shell's to report.
        setKeysFailed(true);
        onError(error);
      }
    })();
    return () => {
      alive = false;
    };
  }, [
    accessStopped,
    bridge,
    onError,
    recoverCatalog,
    selectedId,
    selectedProfile,
  ]);

  const subtitle = plural(stores.length, 'account');

  return (
    <>
      <PageHeader
        title="Accounts"
        subtitle={subtitle}
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
          <AttentionList
            snapshot={snapshot}
            onNavigate={onNavigate}
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
              onRefresh={() => void onRefresh('Accounts refreshed')}
            />
          ) : selected ? (
            <>
              <SectionLabel id="people-accounts-label">
                Accounts on this Mac
              </SectionLabel>
              <AccountSwitcher
                snapshot={snapshot}
                stores={stores}
                selected={selected}
                labelledBy="people-accounts-label"
                onSwitch={(store) =>
                  onNavigate({ kind: 'people', store: store.id })
                }
              />
              <AccountPanel
                snapshot={snapshot}
                store={selected}
                lists={lists}
                loading={loadingKeys}
                failed={keysFailed}
                stopped={stopped}
                onNavigate={onNavigate}
                onSheet={setSheet}
              />
            </>
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
      {selected && sheet === 'rename' ? (
        <RenamePanel
          bridge={bridge}
          profile={selected.server}
          account={selected.account}
          presentation={{
            title: 'Change username',
            subtitle: accountSubtitle(snapshot, selected),
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
            subtitle: accountSubtitle(snapshot, selected),
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
            subtitle: accountSubtitle(snapshot, selected),
            onClose: () => setSheet(null),
          }}
        />
      ) : null}
      {selected && sheet === 'join' ? (
        <InvitationPanel
          bridge={bridge}
          profile={selected.server}
          account={selected.account}
          presentation={{
            title: 'Join a group',
            subtitle: accountSubtitle(snapshot, selected),
            onClose: () => setSheet(null),
          }}
          onComplete={() => onRefresh('Group membership refreshed')}
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
            subtitle: accountSubtitle(snapshot, selected),
            onClose: () => setSheet(null),
          }}
          onComplete={() => onRefresh('Organization sign-in verified')}
        />
      ) : null}
    </>
  );
}

/** The chosen account: who it is, what it is, and what can be done to it. */
function AccountPanel({
  snapshot,
  store,
  lists,
  loading,
  failed,
  stopped,
  onNavigate,
  onSheet,
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
}): ReactNode {
  const username = usernameOf(snapshot, store);
  const server = snapshot.servers.find((entry) => entry.id === store.server);
  const status = stopped.stopped
    ? storeDescription(snapshot, store)
    : server?.compatibility.status === 'not-required'
      ? 'Check-in not required'
      : 'Available';
  const reason = stopped.stopped ? stopped.reason : undefined;
  const keys = deviceEntries(lists);
  return (
    <>
      {/* The profile head: the account's mark and the four facts that name
          it, on one row. */}
      <div className="phead">
        <AccountMark name={username ?? store.account} size="big" />
        <span className="t">
          <h2>{username ?? 'Identity unavailable'}</h2>
          {/* The alias and the availability qualify the server line, so they
              sit on it rather than at the far end of the band. */}
          <span className="line">
            <small>on {serverName(snapshot, store)}</small>
            <Chip>{store.account}</Chip>
            <Chip tone={stopped.stopped ? 'warn' : 'ok'}>{status}</Chip>
          </span>
        </span>
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
      <TeamsOnAccount
        snapshot={snapshot}
        store={store}
        onNavigate={onNavigate}
      />
      <DeviceSummary
        keys={keys}
        loading={loading}
        failed={failed}
        stopped={stopped.stopped}
        onOpen={() => onNavigate({ kind: 'devices', store: store.id })}
      />
      <KeyList
        keys={keys}
        loading={loading}
        failed={failed}
        stopped={stopped}
        server={serverName(snapshot, store)}
      />
      <SectionLabel>{serverName(snapshot, store)}</SectionLabel>
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
          <small>
            Changing a username is a signed operation on the server; the local
            alias does not change with it.
          </small>
        </InsetRow>
        <InsetRow label="Local alias">
          {store.account}
          <small>
            Set when the account was added. No command changes it, so this row
            has no action.
          </small>
        </InsetRow>
        <InsetRow
          label="Passphrase"
          action={
            <Button
              size="sm"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'credentials',
                  store: store.id,
                })
              }
            >
              Settings › Account
            </Button>
          }
        >
          Set, changed and verified with the other typed credentials.
        </InsetRow>
      </Inset>
      <SectionLabel>Actions on this account</SectionLabel>
      <Inset className="settings-inset wide">
        <InsetRow
          label="Organization sign-in"
          action={
            <Button size="sm" onClick={() => onSheet('sso')}>
              Sign in…
            </Button>
          }
        >
          Sign in through your organization’s identity provider.
        </InsetRow>
        <InsetRow
          label="Bot accounts"
          action={
            <Button size="sm" onClick={() => onSheet('bot')}>
              Manage…
            </Button>
          }
        >
          Device credentials for automation on this account.
        </InsetRow>
        <InsetRow
          label="Web admin"
          action={
            <Button size="sm" onClick={() => onSheet('admin')}>
              Open…
            </Button>
          }
        >
          Opens the host’s administration panel in a private window.
        </InsetRow>
        <InsetRow
          label="Groups"
          action={
            <Button size="sm" onClick={() => onSheet('join')}>
              Join a group…
            </Button>
          }
        >
          Request membership with an invitation from an administrator.
        </InsetRow>
      </Inset>
      <p className="fn">
        Account recovery options (Organization sign-in, Bot accounts, Web admin,
        and Join a group) remain available while access is suspended. Changing a
        username is disabled until access is restored.
      </p>
    </>
  );
}

/**
 * The groups this account belongs to, as the catalog holds them. The page does
 * not manage a group: every row opens the group's own page, where it is
 * managed, and the section's action opens Teams.
 */
function TeamsOnAccount({
  snapshot,
  store,
  onNavigate,
}: {
  snapshot: AgentSnapshot;
  store: AccountStore;
  onNavigate: (location: Location) => void;
}): ReactNode {
  const teams = storeNavigationOrder(snapshot).filter(
    (candidate): candidate is TeamStore =>
      candidate.kind === 'team' &&
      candidate.server === store.server &&
      candidate.account === store.account,
  );
  return (
    <div
      className="settings-section"
      role="region"
      aria-labelledby="people-teams-label"
    >
      <SectionLabel
        id="people-teams-label"
        action={
          <Button
            size="sm"
            onClick={() => onNavigate({ kind: 'teams', store: store.id })}
          >
            All teams
          </Button>
        }
      >
        Teams you’re in
      </SectionLabel>
      <Inset className="settings-inset middle wide">
        {teams.length ? (
          teams.map((team) => {
            const mine = partiesOf(snapshot, team.id).find(
              (party) => party.label === 'you',
            );
            const role = mine ? parseRole(mine.destination_role) : null;
            const description = storeDescription(snapshot, team);
            const abnormal = storeAttentionState(snapshot, team) !== 'normal';
            return (
              <InsetRow
                key={team.id}
                className="devrow"
                action={
                  <>
                    {/* An abnormal state is a chip at the end of the row; the
                        roster summary takes its place when nothing is wrong. */}
                    {abnormal ? <Chip tone="warn">{description}</Chip> : null}
                    {role ? <Chip>{roleName(role)}</Chip> : null}
                    <Button
                      size="sm"
                      // Named for the row it ends, and distinct from the
                      // attention card that may route to the same group.
                      aria-label={`Open ${team.name} in Teams`}
                      onClick={() =>
                        onNavigate({
                          kind: 'group-settings',
                          ref: team.id,
                          tab: 'people',
                        })
                      }
                    >
                      Open
                    </Button>
                  </>
                }
              >
                <GroupMark store={team} size="sm" />
                <span className="t">
                  <b>{team.name}</b>
                  <small>
                    {/* The band above names the server this account is on, so
                        the caption under a group on it does not repeat it. */}
                    {teamCaption(snapshot, team, { server: false })}
                    {abnormal ? '' : ` · ${description}`}
                  </small>
                </span>
              </InsetRow>
            );
          })
        ) : (
          <InsetRow label="None">
            This account belongs to no group this Mac holds.
          </InsetRow>
        )}
      </Inset>
    </div>
  );
}

/** What this account holds, counted, with the page that manages it. */
function DeviceSummary({
  keys,
  loading,
  failed,
  stopped,
  onOpen,
}: {
  keys: readonly DeviceEntry[];
  loading: boolean;
  failed: boolean;
  stopped: boolean;
  onOpen: () => void;
}): ReactNode {
  return (
    <div
      className="settings-section"
      role="region"
      aria-labelledby="people-devices-label"
    >
      <SectionLabel
        id="people-devices-label"
        action={
          <Button size="sm" disabled={stopped} onClick={onOpen}>
            Manage devices
          </Button>
        }
      >
        Devices
      </SectionLabel>
      <Inset className="settings-inset middle wide">
        <InsetRow className="devrow">
          <span className="ic" aria-hidden="true">
            <Icon name="laptop" />
          </span>
          <span className="t">
            <b>
              {stopped
                ? 'Not listed while access is stopped'
                : loading
                  ? 'Reading this account’s keys…'
                  : failed
                    ? 'Devices and keys could not be read.'
                    : plural(keys.length, 'device and key', 'devices and keys')}
            </b>
            <small>
              {stopped
                ? 'Devices and keys are listed again once the server is checked.'
                : loading
                  ? 'Loading devices, paper keys, and security keys…'
                  : failed
                    ? 'Could not retrieve keys from the background service. Please retry or check service status.'
                    : keys.length
                      ? keys.map((entry) => entry.name).join(' · ')
                      : 'Nothing is listed for this account on this Mac.'}
            </small>
          </span>
        </InsetRow>
      </Inset>
    </div>
  );
}

/**
 * The keys themselves: a shortened id per key, what it belongs to, and a check
 * on the one this Mac is authenticated with. Nothing here is revoked: a key is
 * acted on from its own page under Devices.
 */
function KeyList({
  keys,
  loading,
  failed,
  stopped,
  server,
}: {
  keys: readonly DeviceEntry[];
  loading: boolean;
  failed: boolean;
  stopped: { stopped: boolean; reason: string };
  server: string;
}): ReactNode {
  return (
    <div
      className="settings-section"
      role="region"
      aria-labelledby="people-keys-label"
    >
      <SectionLabel id="people-keys-label">Keys</SectionLabel>
      <Inset className="settings-inset middle wide">
        {stopped.stopped ? (
          <InsetRow label="None">
            Keys are unavailable until {server} is checked.
          </InsetRow>
        ) : loading ? (
          <InsetRow label="None">Reading this account’s keys…</InsetRow>
        ) : failed ? (
          <InsetRow label="Unavailable">
            Keys could not be read.
            <small>
              Could not retrieve keys from the background service. Open Devices
              to retry or check service status.
            </small>
          </InsetRow>
        ) : keys.length ? (
          keys.map((entry) => (
            <InsetRow
              key={entry.address}
              className="devrow"
              action={
                entry.current ? (
                  // The only verification this page can state: the agent is
                  // authenticated with this key here, now.
                  <span
                    className="verified"
                    role="img"
                    aria-label="Authenticated on this Mac"
                  >
                    <Icon name="check" />
                  </span>
                ) : null
              }
            >
              <span className="ic" aria-hidden="true">
                <Icon name={entry.icon} />
              </span>
              {/* The id leads, as it does on a proof row; what it belongs to
                  follows it on the same line. */}
              <span className="t keyline">
                <b className="mono" title={entry.keyId}>
                  {entry.keyId ? shortId(entry.keyId, 10) : entry.name}
                </b>
                <small>
                  {/* An enrollment is the server's, so its row says the
                      server; every other row says what the key belongs to. */}
                  {entry.scope === 'profile'
                    ? `${entry.kind} · on ${server}`
                    : `${entry.name} · ${entry.kind}`}
                </small>
              </span>
            </InsetRow>
          ))
        ) : (
          <InsetRow label="None">
            No keys are listed for this account on this Mac.
          </InsetRow>
        )}
      </Inset>
      <p className="fn">
        Key ids are shortened; a key’s own page under Devices carries the full
        id. An enrollment on a card is listed for {server}, not for one account
        on it, and the agent reports no id for one. Only the keys this Mac can
        list are shown, and the check marks the one it is authenticated with
        now.
      </p>
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
            Refresh the catalog
          </Button>
          {stores.map((store) => (
            <Button key={store.id} onClick={() => onSelect(store)}>
              {store.account} · {serverName(snapshot, store)}
            </Button>
          ))}
        </>
      }
    >
      <p>
        This account is no longer available on this Mac. Select another account
        below or refresh the catalog.
      </p>
      {stores.length ? null : (
        <p>
          This Mac has no active account. Add and verify a server, then create
          or recover an account.
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
      title="No available account on this Mac"
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
