/**
 * The navigation rail.
 *
 * Five fixed tabs — Files, Chat, Teams, Devices, Settings — under the window's
 * traffic-light strip and the product mark, over the setup card and the
 * account switcher that names the active account and opens the account menu.
 * There is no title bar above the strip. The rail does not enumerate stores;
 * Files and Teams list them on their own pages. Control-Tab cycles through the
 * five tabs. Every tab's indicator sits in one trailing slot: Chat and Teams
 * carry a count pill, Devices and Settings an alert dot beside whatever number
 * is known, while Chat loading appears in the header Refresh control.
 * The rail defaults to 150px open and 56px collapsed. Its expanded width is
 * resizable and persisted; it never expands on hover or focus.
 */

import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Menu, Popover, anyDialogOpen } from '/kit/overlay-primitives';
import { Icon } from '../components';
import { useSidebarResize } from './use-sidebar-resize';
import {
  chatAvailable,
  localAliasOf,
  serverLocalAlias,
  storeAvailability,
  storeDescription,
  storeDescriptionState,
  storeNavigationOrder,
} from '../model';
import type { AccountStore, AgentSnapshot, StoreRef } from '../model';
import type { AgentLifecycle } from '../agent-lifecycle';
import type { FoksIconName } from '../icons';
import { useSidebarInbox } from '../chat/inbox-provider';
import { teamUnread } from '../chat/unread';
import { accountAtLocation, chatTabLocation, railTabOf } from '../location';
import type { Location, RailTab } from '../location';
import { AccountMark } from '../screens/account-switcher';

interface RailTabSpec {
  id: RailTab;
  label: string;
  icon: FoksIconName;
  location: Location;
}

const RAIL_TABS: readonly RailTabSpec[] = [
  { id: 'files', label: 'Files', icon: 'folder', location: { kind: 'files' } },
  { id: 'chat', label: 'Chat', icon: 'chat', location: { kind: 'chat' } },
  { id: 'teams', label: 'Teams', icon: 'users', location: { kind: 'teams' } },
  {
    id: 'devices',
    label: 'Devices',
    icon: 'shield',
    location: { kind: 'devices' },
  },
  {
    id: 'settings',
    label: 'Settings',
    icon: 'settings',
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

/** Configuration and action callbacks for the sidebar setup progress card. */
export interface RailSetup {
  /** Number of completed steps. */
  done: number;
  total: number;
  /** Short label for the next setup step. */
  next: string;
  onContinue: () => void;
  /** Optional callback invoked when dismissing the card with "Later". When omitted, the "Later" button is hidden. */
  onLater?: () => void;
}

export interface SidebarProps {
  /** Absent while the agent is starting: the rail draws its frame regardless. */
  snapshot?: AgentSnapshot;
  location: Location;
  account?: StoreRef;
  /**
   * Notices no tab's own badge can resolve — a catalog read that only a
   * retry can fix, or a note whose place could not be derived. Draws the dot
   * on the account avatar and on the Settings tab, whose Account section lists
   * them; every note type with a tab of its own reaches it through that tab's
   * badge instead.
   */
  attention?: number;
  /** The Teams tab's badge: pending membership requests this session knows about. */
  teamRequests?: { label: string; description: string } | null;
  /** The Devices tab's dot: an open pairing offer, or an account with no paper key. */
  devicesAlert?: { description: string } | null;
  /**
   * The Settings tab's dot: a lapsed check-in or an unverified server. The
   * tab also draws the dot for `attention`, with both reasons in its label.
   */
  settingsAlert?: { description: string } | null;
  onNavigate: (location: Location) => void;
  onTabNavigate?: (tab: RailTab) => void;
  /**
   * Rows between the tabs and the foot. First run puts its progress there;
   * the shell has no additional status to display at that point.
   */
  status?: ReactNode;
  /**
   * First run's progress, drawn in the same slot as the setup card rather
   * than as a row. Omitted once setup is done, or by a caller that has only
   * the row to give.
   */
  setup?: RailSetup;
  /**
   * First run is already in the flow, so the account menu's "Add an account or
   * server…" re-enters it rather than navigating to the flow's first step.
   */
  onReenter?: () => void;
  /** Arms the application lock. Omitted where no lock command is reachable. */
  onLock?: () => void;
  /**
   * Collapsed to the 56px icon-only track. The traffic strip carries the toggle.
   */
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
 * The rail's account switcher and its menu, at the foot of the rail: the
 * accounts on this Mac grouped by
 * server, then the two commands that are not a place — adding an account and
 * locking the app. The row names the account and the host it lives on; no
 * connection state is reported here. This is the application's only account switcher; the
 * Account section of Settings names the account in its own header and has no
 * switcher of its own. The dot rides the avatar for a note no tab's own badge
 * can resolve, and opens Settings › Account, where that note is still listed.
 */
export function AccountHeader({
  snapshot,
  location,
  account,
  attention = 0,
  blocked = false,
  onNavigate,
  onReenter,
  onLock,
}: {
  snapshot: AgentSnapshot;
  location: Location;
  account?: StoreRef;
  attention?: number;
  /** Whether the sidebar footer is disabled and dimmed during a blocking operation. */
  blocked?: boolean;
  onNavigate: (location: Location) => void;
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
  const activeServer = active
    ? snapshot.servers.find((candidate) => candidate.id === active.server)
    : undefined;
  const server = active
    ? serverLocalAlias(activeServer)
    : 'None on this device';
  // The foot names the host the account lives on. The reader's own label for
  // that server is the accessible name and the tooltip, where a second line
  // costs nothing; the row itself has one line for it and a hostname is what
  // distinguishes two accounts of the same name.
  const host = active ? (activeServer?.name ?? server) : server;
  // Preserve account-scoped locations when selecting an account; otherwise,
  // open the account's own page, Settings › Account.
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
    else onNavigate({ kind: 'settings', section: 'account', store: store.id });
  };
  const servers = [...new Set(accounts.map((store) => store.server))];
  // Account marks use the same username-derived hue as the Files vault row.
  const close = (): void => setOpen(false);
  return (
    <div className={blocked ? 'rail-foot is-blocked' : 'rail-foot'}>
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
        <AccountMark name={username} size="round" className="avatar" />
        <span className="t">
          <b>{username}</b>
          <small>{host}</small>
        </span>
        <span className="chev">
          <Icon name="chevronDown" />
        </span>
      </button>
      {attention > 0 ? (
        <button
          type="button"
          className="attn"
          aria-label="Needs attention"
          title="Needs attention"
          // The notices are listed on the Account section, so the dot opens
          // that section by address rather than the tab's remembered page.
          onClick={() => onNavigate({ kind: 'settings', section: 'account' })}
        />
      ) : null}
      {open ? (
        <Popover
          anchorRef={anchorRef}
          className="menu-portal"
          align="start"
          gap={4}
          // The rail runs to the window's left edge, so the usual viewport
          // margin would push the menu off the edge it aligns with.
          inset={0}
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
                          <AccountMark name={usernameOf(store)} />
                          <span className="t">
                            {usernameOf(store)}
                            <small>
                              {localAliasOf(snapshot, store)} ·{' '}
                              {serverLocalAlias(
                                snapshot.servers.find(
                                  (entry) => entry.id === serverId,
                                ),
                              )}
                            </small>
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
                                    section: 'account',
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
 * (loading, unavailable); those contribute nothing to the sum. Loading is
 * reported by the header Refresh control.
 *
 * A team whose synchronization failed or arrived incomplete marks the badge
 * rather than replacing it: an inbox poll fails and retries on a backoff, and
 * a count that vanished on each failed poll would read as messages going away.
 * The count that is known is still drawn, in the warning colour, and the
 * description says what is unaccounted for. Only when no team could be counted
 * at all does the badge fall back to a warning dot with nothing to show.
 */
function railChatUnread(
  snapshot: AgentSnapshot,
  inbox: ReturnType<typeof useSidebarInbox>,
): RailCount | { warning: true; description: string } | null {
  let total = 0;
  let known = false;
  const warnings = new Set<string>();
  for (const store of storeNavigationOrder(snapshot)) {
    if (!chatAvailable(snapshot, store)) continue;
    const entry = inbox.get(store.id);
    if (!entry || (entry.state === 'loading' && !entry.data && !entry.error)) {
      continue;
    }
    const unread = teamUnread(entry);
    if (!unread) continue;
    if (
      !entry.data ||
      entry.state === 'blocked' ||
      entry.state === 'unavailable' ||
      entry.stale ||
      entry.error ||
      entry.data.degraded
    )
      warnings.add(unread.description);
    const count = Number.parseInt(unread.label, 10);
    if (!Number.isNaN(count)) {
      total += count;
      known = true;
    }
  }
  if (warnings.size) {
    const description = `${known ? `${total} known unread; ` : ''}${[...warnings].join('; ')}`;
    return known
      ? { label: String(total), description, warning: true }
      : { warning: true, description };
  }
  if (known) return { label: String(total), description: `${total} unread` };
  return null;
}

type RailCount = { label: string; description: string; warning?: boolean };

/**
 * Status indicator for each tab. Unread counts trail the label as an orange
 * pill; orange dots indicate actionable status and stand beside whatever
 * number is known rather than replacing it.
 * A count carrying `warn` is a total that is known to be short of something,
 * drawn in orange rather than dropped.
 */
function RailTail({
  kind,
  description,
  children,
}: {
  kind: 'count' | 'count warn' | 'dot' | 'dot warn';
  description: string;
  children?: ReactNode;
}): ReactNode {
  return (
    <span
      className={`rail-tail ${kind}`}
      aria-label={description}
      title={description}
    >
      {children}
    </span>
  );
}

/**
 * Setup progress card displayed above the account switcher. Shows completion progress, the next setup action, and buttons to continue or postpone setup. When collapsed, renders a compact fraction icon with full details in the tooltip.
 *
 * The "Later" button is only rendered when an onLater callback is provided.
 */
function SetupCard({ setup }: { setup?: RailSetup }): ReactNode {
  if (!setup) return null;
  const { done, total, next, onContinue, onLater } = setup;
  const share = total > 0 ? Math.min(1, Math.max(0, done / total)) : 0;
  return (
    <div
      className="setup-card"
      data-short={`${done}/${total}`}
      title={`Finish setting up: ${done} of ${total} steps done`}
    >
      <b>Finish setting up</b>
      <small>{next}</small>
      <div
        className="bar"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={total}
        aria-valuenow={done}
        aria-label="Setup progress"
      >
        <i style={{ width: `${Math.round(share * 100)}%` }} />
      </div>
      <div className="act">
        <button type="button" onClick={onContinue}>
          Continue
        </button>
        {onLater ? (
          <button type="button" className="dim" onClick={onLater}>
            Later
          </button>
        ) : null}
      </div>
    </div>
  );
}

export function Sidebar({
  snapshot,
  location,
  account,
  attention = 0,
  teamRequests = null,
  devicesAlert = null,
  settingsAlert = null,
  onNavigate,
  onTabNavigate,
  status,
  setup,
  onReenter,
  onLock,
  collapsed = false,
  onToggleCollapsed,
  nativeChrome = false,
  blocked = false,
}: SidebarProps): ReactNode {
  const resize = useSidebarResize(!collapsed && !blocked);
  const chatInbox = useSidebarInbox();
  const unread = snapshot ? railChatUnread(snapshot, chatInbox) : null;
  const here = railTabOf(location);
  /**
   * A tab's own indicator: a muted count for Chat and Teams, and orange dots
   * for Devices and Settings. Dots overlay the icon's upper corner; counts
   * also become icon dots when the rail is collapsed.
   */
  const railTail = (tab: RailTab): ReactNode => {
    if (tab === 'chat') {
      if (!unread) return undefined;
      if (!('label' in unread))
        return <RailTail kind="dot warn" description={unread.description} />;
      return (
        <RailTail
          kind={unread.warning ? 'count warn' : 'count'}
          description={unread.description}
        >
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
        <RailTail kind="dot warn" description={devicesAlert.description} />
      ) : undefined;
    if (tab === 'settings') {
      const reasons = [
        settingsAlert?.description,
        attention > 0
          ? `${attention} ${attention === 1 ? 'notice needs' : 'notices need'} attention`
          : undefined,
      ].filter((reason): reason is string => Boolean(reason));
      return reasons.length ? (
        <RailTail kind="dot warn" description={reasons.join('; ')}>
          {/* The dot's own number, where one exists: the notices the Account
              section lists. A server's lapsed check-in has none. */}
          {attention > 0 ? <span className="num">{attention}</span> : undefined}
        </RailTail>
      ) : undefined;
    }
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

  return (
    <nav
      ref={resize.ref}
      className={[
        'side',
        'rail',
        collapsed ? 'is-narrow' : '',
        resize.dragging ? 'is-resizing' : '',
      ]
        .filter(Boolean)
        .join(' ')}
      aria-label="Main Navigation"
    >
      {resize.handle}
      <TrafficStrip native={nativeChrome}>
        {onToggleCollapsed && !collapsed ? (
          <button
            type="button"
            className="side-collapse"
            aria-expanded="true"
            aria-label="Collapse sidebar"
            title="Collapse sidebar"
            disabled={blocked}
            onClick={onToggleCollapsed}
          >
            <Icon name="panelLeftClose" />
          </button>
        ) : null}
      </TrafficStrip>
      <div className={blocked ? 'rail-body is-blocked' : 'rail-body'}>
        {collapsed && onToggleCollapsed ? (
          <button
            type="button"
            className="rail-brand"
            aria-expanded="false"
            aria-label="Expand sidebar"
            title="Expand sidebar"
            disabled={blocked}
            onClick={onToggleCollapsed}
          >
            <span className="mark" aria-hidden="true">
              <Icon name="pawPrint" />
            </span>
            <span className="lab">foks-rs</span>
          </button>
        ) : (
          <div className="rail-brand">
            <span className="mark" aria-hidden="true">
              <Icon name="pawPrint" />
            </span>
            <span className="lab">foks-rs</span>
          </div>
        )}
        <div className="rail-tabs">
          {RAIL_TABS.map((tab) => (
            <NavRow
              key={tab.id}
              active={here === tab.id}
              glyph={
                <Icon
                  name={tab.icon}
                  className={
                    tab.id === 'chat'
                      ? 'rail-chat-icon'
                      : tab.id === 'files'
                        ? 'rail-files-icon'
                        : undefined
                  }
                />
              }
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
        <SetupCard setup={setup} />
      </div>
      {snapshot ? (
        <AccountHeader
          snapshot={snapshot}
          location={location}
          account={account}
          attention={attention}
          blocked={blocked}
          onNavigate={onNavigate}
          onReenter={onReenter}
          onLock={onLock}
        />
      ) : (
        // No account is known yet. The foot keeps its height so nothing above
        // it moves once one is.
        <div className="rail-foot">
          <div className="who who-empty">
            <span className="avatar" aria-hidden="true" />
          </div>
        </div>
      )}
    </nav>
  );
}
