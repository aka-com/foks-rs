/**
 * The navigation rail.
 *
 * Six fixed tabs — Files, Chat, Teams, Devices, Account, Settings — under an
 * account header that names the active account and opens the account menu, and
 * over the agent light at the foot. The rail's top is the window's traffic-light
 * strip: there is no title bar above it. The rail does not enumerate stores;
 * Files and Teams list them on their own pages. Control-Tab cycles through the
 * six tabs. Every tab's indicator sits in one trailing slot: Chat and Teams
 * carry a
 * muted count, Devices and Settings a dot, since neither has a number to
 * substantiate, and Chat a spinner while its counts load.
 * The rail is 200px open and 46px collapsed. The width is selected by the user
 * from the rail itself: it never expands on hover or focus.
 */

import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Menu, Popover, anyDialogOpen } from '/kit/overlay-primitives';
import { Icon } from '../components';
import {
  chatAvailable,
  localAliasOf,
  serverDisplayName,
  serverName,
  storeAvailability,
  storeDescription,
  storeDescriptionState,
  storeHues,
  storeNavigationOrder,
} from '../model';
import type { AccountStore, AgentSnapshot, StoreRef } from '../model';
import type { AgentLifecycle } from '../agent-lifecycle';
import type { FoksIconName } from '../icons';
import { useSidebarInbox } from '../chat/inbox-provider';
import { teamUnread } from '../chat/unread';
import {
  accountAtLocation,
  chatTabLocation,
  parentLocation,
  railTabOf,
} from '../location';
import type { Location, RailTab } from '../location';
import { filesFolderCrumb } from '../screens/scope';

interface RailTabSpec {
  id: RailTab;
  label: string;
  icon: FoksIconName;
  location: Location;
}

const RAIL_TABS: readonly RailTabSpec[] = [
  { id: 'files', label: 'Files', icon: 'folder', location: { kind: 'files' } },
  { id: 'chat', label: 'Chat', icon: 'chat', location: { kind: 'chat' } },
  { id: 'teams', label: 'Teams', icon: 'people', location: { kind: 'teams' } },
  {
    id: 'devices',
    label: 'Devices',
    icon: 'shield',
    location: { kind: 'devices' },
  },
  {
    id: 'people',
    label: 'Account',
    icon: 'person',
    location: { kind: 'people' },
  },
  {
    id: 'settings',
    label: 'Settings',
    icon: 'gear',
    location: { kind: 'settings' },
  },
];

/**
 * Where a tab goes. Chat returns to the team and channel it last had open, so
 * leaving Chat and coming back does not re-run the first-team fallback.
 */
function tabLocation(tab: RailTabSpec): Location {
  return tab.id === 'chat' ? chatTabLocation() : tab.location;
}

/** Control-Tab destinations in rail order. */
export function sidebarCycleLocations(): Location[] {
  return RAIL_TABS.map(tabLocation);
}

export function nextSidebarCycleLocation(
  current: Location,
  delta: 1 | -1,
): Location | null {
  const places = sidebarCycleLocations();
  // Every location a tab owns answers `railTabOf`, so the tab it belongs to is
  // the only thing the walk needs; first run belongs to none and starts at an
  // end.
  const here = railTabOf(current);
  const index = RAIL_TABS.findIndex((tab) => tab.id === here);
  if (index < 0) return delta > 0 ? places[0] : places[places.length - 1];
  return places[(index + delta + places.length) % places.length];
}

/* --------------------------------------------------------- window controls -- */

/**
 * The strip the window is dragged by, at the top of the rail. macOS draws its
 * own controls over the native window's strip, so the fake lights are the web
 * mock's alone; either way the strip reserves the height they occupy.
 */
export function TrafficStrip({
  native = false,
  children,
}: {
  native?: boolean;
  children?: ReactNode;
}): ReactNode {
  return (
    <div className="traffic" data-tauri-drag-region="">
      {native ? null : (
        <span className="lights" aria-hidden="true">
          <span className="light" />
          <span className="light" />
          <span className="light" />
        </span>
      )}
      {children}
    </div>
  );
}

/* ------------------------------------------------------------ agent light -- */

/** Connection states displayed in the rail footer. */
export type RailAgentState = 'ready' | 'starting' | 'stopped' | 'locked';

const AGENT_LABEL: Readonly<Record<RailAgentState, string>> = {
  ready: 'Connected',
  starting: 'Connecting…',
  stopped: 'Offline',
  locked: 'Locked',
};

/**
 * What the light says about a lifecycle. The internal step names — bootstrap,
 * initializing, the maintenance operation — are all one word to the reader:
 * the agent is starting.
 */
export function railAgentState(
  lifecycle: AgentLifecycle['state'],
  snapshotAgent?: AgentSnapshot['agent']['state'],
): RailAgentState {
  if (snapshotAgent === 'bootstrap') return 'starting';
  switch (lifecycle) {
    case 'ready':
      return 'ready';
    case 'checking':
    case 'bootstrap':
    case 'initializing':
    case 'maintenance':
      return 'starting';
    default:
      return 'stopped';
  }
}

/**
 * The light itself. Exported because first run's step list is the same rail
 * and closes with the same row.
 */
export function AgentLight({ state }: { state: RailAgentState }): ReactNode {
  const label = AGENT_LABEL[state];
  return (
    <div className={`status agent-${state}`} title={label}>
      <i aria-hidden="true" />
      <span className="t">{label}</span>
    </div>
  );
}

/* -------------------------------------------------------------------- rows -- */

export interface NavRowProps {
  active: boolean;
  glyph?: ReactNode;
  name: string;
  /** Tooltip. The rail sets it only while collapsed, where the label is gone. */
  title?: string;
  tail?: ReactNode;
  /** A blocked shell draws its tabs but does not let them be used. */
  disabled?: boolean;
  onSelect: () => void;
}

export function NavRow({
  active,
  glyph,
  name,
  title,
  tail,
  disabled = false,
  onSelect,
}: NavRowProps): ReactNode {
  return (
    <button
      type="button"
      className={active ? 'nav on' : 'nav'}
      title={title}
      disabled={disabled || undefined}
      aria-current={active ? 'page' : undefined}
      onClick={onSelect}
    >
      {glyph}
      <span className="t">{name}</span>
      {tail}
    </button>
  );
}

export interface SidebarProps {
  /** Absent while the agent is starting: the rail draws its frame regardless. */
  snapshot?: AgentSnapshot;
  location: Location;
  /** The Files tree's selected folder (`LocationState.folder`). */
  folder?: string;
  account?: StoreRef;
  /**
   * Notices no tab's own badge can resolve — a catalog read that only a
   * retry can fix, or a note whose place could not be derived. Draws the dot
   * on the account avatar; every note type with a tab of its own reaches it
   * through that tab's badge instead.
   */
  attention?: number;
  /** The Teams tab's badge: pending membership requests this session knows about. */
  teamRequests?: { label: string; description: string } | null;
  /** The Devices tab's dot: an open pairing offer, or an account with no paper key. */
  devicesAlert?: { description: string } | null;
  /** The Settings tab's dot: a lapsed check-in or an unverified server. */
  settingsAlert?: { description: string } | null;
  onNavigate: (location: Location) => void;
  /** Steps the Files tree's selection back one folder. Omitted where the
   *  tree's selection is not reachable, in which case Back only ever
   *  navigates a location. */
  onSetFolder?: (folder: string) => void;
  onTabNavigate?: (tab: RailTab) => void;
  /**
   * Rows between the tabs and the foot. First run puts its progress there;
   * the shell has no additional status to display at that point.
   */
  status?: ReactNode;
  /**
   * First run is already in the flow, so the account menu's "Add an account or
   * server…" re-enters it rather than navigating to the flow's first step.
   */
  onReenter?: () => void;
  /** Arms the application lock. Omitted where no lock command is reachable. */
  onLock?: () => void;
  /** Collapsed to the 46px icon-only track. The rail carries the toggle. */
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
  /** Connection status displayed in the rail footer. */
  agent?: RailAgentState;
  /** The OS draws the window controls over the strip, so no lights are faked. */
  nativeChrome?: boolean;
  /** A blocking state: the header and the tabs are dimmed and inert. */
  blocked?: boolean;
}

/**
 * The rail's account header and its menu: the accounts on this Mac grouped by
 * server, then the two commands that are not a place — adding an account and
 * locking the app. The dot rides the avatar for a note no tab's own badge can
 * resolve, and opens the Account tab, where that note is still listed.
 * `compact`, used by the Account tab's own "Switch account" button, opens the
 * same menu from a plain button rather than the avatar — it is the same
 * component so there is exactly one switcher, not a second one repeating it.
 */
export function AccountHeader({
  snapshot,
  location,
  account,
  attention = 0,
  compact = false,
  onNavigate,
  onTabNavigate,
  onReenter,
  onLock,
}: {
  snapshot: AgentSnapshot;
  location: Location;
  account?: StoreRef;
  attention?: number;
  compact?: boolean;
  onNavigate: (location: Location) => void;
  onTabNavigate?: (tab: RailTab) => void;
  onReenter?: () => void;
  onLock?: () => void;
}): ReactNode {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const active = accountAtLocation(snapshot.stores, location, account);
  const accounts = snapshot.stores.filter(
    (store): store is AccountStore => store.kind === 'account',
  );
  const usernameOf = (store: AccountStore): string =>
    snapshot.accounts.find((entry) => entry.store === store.id)?.username ??
    store.account;
  const username = active ? usernameOf(active) : 'No account';
  const server = active ? serverName(snapshot, active) : 'None on this device';
  // Preserve account-scoped locations when selecting an account; otherwise,
  // open Account.
  const selectAccount = (store: AccountStore): void => {
    if (location.kind === 'devices') {
      onNavigate({
        kind: 'devices',
        store: store.id,
        section: location.section,
      });
    } else if (location.kind === 'settings') {
      onNavigate({
        kind: 'settings',
        section: location.section,
        store: store.id,
      });
    } else if (
      // Teams is account-scoped, so preserve the Teams location.
      location.kind === 'teams'
    )
      onNavigate({ ...location, store: store.id });
    else onNavigate({ kind: 'people', store: store.id });
  };
  const servers = [...new Set(accounts.map((store) => store.server))];
  // The same hue the Files and Teams rows draw each store's mark in, so an
  // account's initial is white on its own colour rather than on nothing.
  const hues = storeHues(storeNavigationOrder(snapshot));
  const close = (): void => setOpen(false);
  return (
    <div className={compact ? 'account-switch' : 'rail-head'}>
      {compact ? (
        // The Account tab already names the account in its own heading, so
        // this trigger's job is only to say what it opens — not to repeat the
        // identity the rail header states.
        <button
          type="button"
          ref={anchorRef}
          className="btn"
          aria-haspopup="menu"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
        >
          Switch account
          <Icon name="chev" />
        </button>
      ) : (
        <button
          type="button"
          ref={anchorRef}
          className="who"
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label={`${username} · ${server}`}
          title={`${username} · ${server}`}
          onClick={() => setOpen(!open)}
        >
          <span className="avatar" aria-hidden="true">
            {username.slice(0, 1).toUpperCase()}
          </span>
          <span className="t">
            <b>{username}</b>
            <small>{server}</small>
          </span>
          <span className="chev">
            <Icon name="chev" />
          </span>
        </button>
      )}
      {attention > 0 ? (
        <button
          type="button"
          className="attn"
          aria-label="Needs attention"
          title="Needs attention"
          onClick={() => {
            if (onTabNavigate) onTabNavigate('people');
            else onNavigate({ kind: 'people' });
          }}
        />
      ) : null}
      {open ? (
        <Popover
          anchorRef={anchorRef}
          className="menu-portal"
          align="start"
          gap={4}
          onClose={close}
        >
          <Menu
            className="menu rail-account-menu"
            anchorRef={anchorRef}
            onClose={close}
            aria-label="Accounts on this device"
          >
            {servers.map((serverId) => (
              <div key={serverId}>
                <div className="cap">
                  {(() => {
                    const entry = snapshot.servers.find(
                      (candidate) => candidate.id === serverId,
                    );
                    if (!entry) return serverId;
                    const name = serverDisplayName(entry);
                    // Only the label a reader gave the server is a section
                    // heading. The host keeps its own case: uppercasing a
                    // hostname reads wrong, and it is what wrapped this caption
                    // onto a second line.
                    return entry.label ? (
                      <>
                        {name}
                        <span className="host">{` ${entry.name}`}</span>
                      </>
                    ) : (
                      name
                    );
                  })()}
                </div>
                {accounts
                  .filter((store) => store.server === serverId)
                  .map((store) => {
                    const state = storeDescriptionState(snapshot, store);
                    const stopped = !storeAvailability(snapshot, store)
                      .available;
                    const checkIn =
                      state === 'check-in-expired' ||
                      state === 'check-in-unavailable';
                    return (
                      <div key={store.id}>
                        <button
                          type="button"
                          className={[
                            'acct',
                            store.id === active?.id ? 'on' : '',
                            stopped ? 'off' : '',
                          ]
                            .filter(Boolean)
                            .join(' ')}
                          title={
                            stopped
                              ? storeDescription(snapshot, store)
                              : usernameOf(store)
                          }
                          onClick={() => {
                            close();
                            selectAccount(store);
                          }}
                        >
                          <span
                            className="av team"
                            style={{ background: hues.get(store.id) }}
                            aria-hidden="true"
                          >
                            {usernameOf(store).slice(0, 1).toUpperCase()}
                          </span>
                          <span className="t">
                            {usernameOf(store)}
                            <small>{localAliasOf(snapshot, store)}</small>
                          </span>
                          <span
                            className={
                              store.id === active?.id ? 'tick' : 'tick off'
                            }
                            aria-label={
                              store.id === active?.id
                                ? 'Current account'
                                : undefined
                            }
                          >
                            <Icon name="check" />
                          </span>
                        </button>
                        {stopped ? (
                          <div className="warnline">
                            <Icon name="alert" />
                            {/* Place the recovery action after the explanation.
                                A separate flex column leaves insufficient room
                                for the reason at this width. */}
                            <span className="t">
                              {checkIn
                                ? 'Stores are locked until you check this server.'
                                : storeDescription(snapshot, store)}{' '}
                              <button
                                type="button"
                                className="lnk"
                                onClick={() => {
                                  close();
                                  onNavigate({
                                    kind: 'settings',
                                    section: 'servers',
                                    profile: store.server,
                                    store: store.id,
                                  });
                                }}
                              >
                                {checkIn ? 'Check in' : 'Server settings'}
                              </button>
                            </span>
                          </div>
                        ) : null}
                      </div>
                    );
                  })}
              </div>
            ))}
            <div className="menu-separator" role="separator" />
            <button
              type="button"
              onClick={() => {
                close();
                if (onReenter) onReenter();
                else onNavigate({ kind: 'first-run', step: 'who' });
              }}
            >
              <Icon name="plus" />
              Add account or server…
            </button>
            {onLock ? (
              <button
                type="button"
                onClick={() => {
                  close();
                  onLock();
                }}
              >
                <Icon name="shield" />
                Lock
              </button>
            ) : null}
          </Menu>
        </Popover>
      ) : null}
    </div>
  );
}

/**
 * The total unread across the teams whose chat this Mac can read. `teamUnread`
 * reports one team at a time, including the states that have no number
 * (loading, unavailable); those contribute nothing to the sum but do keep the
 * badge from disappearing while a team is still loading.
 */
function railChatUnread(
  snapshot: AgentSnapshot,
  inbox: ReturnType<typeof useSidebarInbox>,
): RailCount | { loading: true; description: string } | null {
  let total = 0;
  let known = false;
  let pending = false;
  for (const store of storeNavigationOrder(snapshot)) {
    if (!chatAvailable(snapshot, store)) continue;
    const unread = teamUnread(inbox.get(store.id));
    if (!unread) continue;
    const count = Number.parseInt(unread.label, 10);
    if (Number.isNaN(count)) pending = true;
    else {
      total += count;
      known = true;
    }
  }
  if (known) return { label: String(total), description: `${total} unread` };
  return pending
    ? { loading: true, description: 'Loading unread counts' }
    : null;
}

type RailCount = { label: string; description: string };

/**
 * Standard trailing status slot for each tab. Unread counts use neutral text,
 * dots indicate actionable status without a numeric value, `warn` uses amber,
 * and `loading` renders a ring spinner.
 */
function RailTail({
  kind,
  description,
  children,
}: {
  kind: 'count' | 'dot' | 'dot warn' | 'loading';
  description: string;
  children?: ReactNode;
}): ReactNode {
  return (
    <span
      className={`rail-tail ${kind}`}
      role={kind === 'loading' ? 'status' : undefined}
      aria-label={description}
      title={description}
    >
      {kind === 'loading' ? (
        <i className="spin" aria-hidden="true" />
      ) : (
        children
      )}
    </span>
  );
}

export function Sidebar({
  snapshot,
  location,
  folder = '',
  account,
  attention = 0,
  teamRequests = null,
  devicesAlert = null,
  settingsAlert = null,
  onNavigate,
  onSetFolder,
  onTabNavigate,
  status,
  onReenter,
  onLock,
  collapsed = false,
  onToggleCollapsed,
  agent = 'ready',
  nativeChrome = false,
  blocked = false,
}: SidebarProps): ReactNode {
  const chatInbox = useSidebarInbox();
  const unread = snapshot ? railChatUnread(snapshot, chatInbox) : null;
  const here = railTabOf(location);
  /**
   * A tab's own indicator: a muted count for Chat and Teams, a spinner for
   * Chat while its counts load, a dot for Devices, and an amber one for
   * Settings. In collapsed mode, indicators render as overlay badges on the
   * icon's upper corner.
   */
  const railTail = (tab: RailTab): ReactNode => {
    if (tab === 'chat') {
      if (!unread) return undefined;
      if ('loading' in unread)
        return <RailTail kind="loading" description={unread.description} />;
      return (
        <RailTail kind="count" description={unread.description}>
          {unread.label}
        </RailTail>
      );
    }
    if (tab === 'teams')
      return teamRequests ? (
        <RailTail kind="count" description={teamRequests.description}>
          {teamRequests.label}
        </RailTail>
      ) : undefined;
    if (tab === 'devices')
      return devicesAlert ? (
        <RailTail kind="dot" description={devicesAlert.description} />
      ) : undefined;
    if (tab === 'settings')
      return settingsAlert ? (
        <RailTail kind="dot warn" description={settingsAlert.description} />
      ) : undefined;
    return undefined;
  };
  useEffect(() => {
    if (blocked) return;
    const onKeyDown = (event: KeyboardEvent): void => {
      if (
        event.key !== 'Tab' ||
        !event.ctrlKey ||
        event.metaKey ||
        event.altKey
      )
        return;
      // This listener runs on document, beyond a dialog's event boundary.
      // Disable sidebar tab cycling while a dialog is open so the underlying
      // view does not change.
      if (anyDialogOpen()) return;
      const next = nextSidebarCycleLocation(location, event.shiftKey ? -1 : 1);
      if (!next) return;
      event.preventDefault();
      const tab = railTabOf(next);
      if (onTabNavigate && tab) onTabNavigate(tab);
      else onNavigate(next);
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [blocked, location, onNavigate, onTabNavigate]);

  // One step back can be a narrower folder within the same page — the Files
  // tree's own selection, which `parentLocation` does not see — before it is
  // a different location. Matches the topbar's identical chevron, drawn
  // there instead once the rail collapses too narrow to carry its own.
  const folderBack = blocked
    ? null
    : filesFolderCrumb(snapshot, location, folder).back;
  const parent = blocked ? null : parentLocation(location);
  const canGoBack = folderBack !== null || Boolean(parent);
  const goBack = (): void => {
    if (folderBack !== null) onSetFolder?.(folderBack);
    else if (parent) onNavigate(parent);
  };

  return (
    <nav
      className={['side', 'rail', collapsed ? 'is-narrow' : '']
        .filter(Boolean)
        .join(' ')}
      aria-label="Main Navigation"
    >
      <TrafficStrip native={nativeChrome}>
        {!collapsed ? (
          <button
            type="button"
            className="rail-back"
            aria-label="Back"
            title="Back"
            disabled={!canGoBack}
            onClick={goBack}
          >
            <Icon name="back" />
          </button>
        ) : null}
      </TrafficStrip>
      <div className={blocked ? 'rail-body is-blocked' : 'rail-body'}>
        {snapshot ? (
          <AccountHeader
            snapshot={snapshot}
            location={location}
            account={account}
            attention={attention}
            onNavigate={onNavigate}
            onTabNavigate={onTabNavigate}
            onReenter={onReenter}
            onLock={onLock}
          />
        ) : (
          // No account is known yet. The header keeps its height so the tabs
          // below it do not move once one is.
          <div className="rail-head">
            <div className="who who-empty">
              <span className="avatar" aria-hidden="true" />
            </div>
          </div>
        )}
        <div className="rail-tabs">
          {RAIL_TABS.map((tab) => (
            <NavRow
              key={tab.id}
              active={here === tab.id}
              glyph={<Icon name={tab.icon} />}
              name={tab.label}
              // The expanded rail already reads the label; a tooltip repeating
              // it is noise.
              title={collapsed ? tab.label : undefined}
              disabled={blocked}
              tail={railTail(tab.id)}
              onSelect={() => {
                if (onTabNavigate) onTabNavigate(tab.id);
                else onNavigate(tabLocation(tab));
              }}
            />
          ))}
        </div>
      </div>
      <div className="side-bottom">
        {status}
        {onToggleCollapsed ? (
          <button
            type="button"
            className="nav side-collapse"
            aria-expanded={!collapsed}
            aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
            title={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
            disabled={blocked}
            onClick={onToggleCollapsed}
          >
            <Icon name={collapsed ? 'panel-hollow' : 'panel-filled'} />
            <span className="t">
              {collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
            </span>
          </button>
        ) : null}
        <div className="foot">
          <AgentLight state={agent} />
        </div>
      </div>
    </nav>
  );
}
