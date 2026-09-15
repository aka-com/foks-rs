/**
 * The navigation rail.
 *
 * Six fixed tabs — Accounts, Chat, Files, Teams, Devices, Settings — under an
 * account header that names the active account and opens the account menu, and
 * over the agent light at the foot. The rail's top is the window's traffic-light
 * strip: there is no title bar above it. The rail does not enumerate stores;
 * Files and Teams list them on their own pages. Control-Tab walks the six tabs.
 *
 * The rail is 200px open and 56px collapsed, and collapsing is a width the
 * reader chooses from the topbar: it never expands on hover or focus.
 */

import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Menu, Popover, anyDialogOpen } from '/kit/overlay-primitives';
import { Icon } from '../components';
import {
  chatAvailable,
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
import { accountAtLocation, chatTabLocation, railTabOf } from '../location';
import type { Location, RailTab } from '../location';

interface RailTabSpec {
  id: RailTab;
  label: string;
  icon: FoksIconName;
  location: Location;
}

const RAIL_TABS: readonly RailTabSpec[] = [
  {
    id: 'people',
    label: 'Accounts',
    icon: 'person',
    location: { kind: 'people' },
  },
  { id: 'chat', label: 'Chat', icon: 'chat', location: { kind: 'chat' } },
  { id: 'files', label: 'Files', icon: 'folder', location: { kind: 'files' } },
  { id: 'teams', label: 'Teams', icon: 'people', location: { kind: 'teams' } },
  {
    id: 'devices',
    label: 'Devices',
    icon: 'shield',
    location: { kind: 'devices' },
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
}: {
  native?: boolean;
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

function AgentLight({ state }: { state: RailAgentState }): ReactNode {
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
  account?: StoreRef;
  /** How many things need attention. Draws the dot on the account avatar. */
  attention?: number;
  onNavigate: (location: Location) => void;
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
  /** Collapsed to the 56px icon-only track. The topbar owns the toggle. */
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
 * locking the app. The attention dot rides the avatar and opens the Accounts
 * tab, which is where the list of things to attend to lives.
 */
function AccountHeader({
  snapshot,
  location,
  account,
  attention,
  onNavigate,
  onTabNavigate,
  onReenter,
  onLock,
}: {
  snapshot: AgentSnapshot;
  location: Location;
  account?: StoreRef;
  attention: number;
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
  const server = active ? serverName(snapshot, active) : 'None on this Mac';
  // Preserve account-scoped locations when selecting an account; otherwise,
  // open Accounts.
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
    <div className="rail-head">
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
      {attention > 0 ? (
        <button
          type="button"
          className="attn"
          aria-label={
            attention === 1
              ? '1 thing needs attention'
              : `${attention} things need attention`
          }
          title={
            attention === 1
              ? '1 thing needs attention'
              : `${attention} things need attention`
          }
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
            aria-label="Accounts on this Mac"
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
                        <span className="host">{` · ${entry.name}`}</span>
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
                            <small>{store.account}</small>
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
            <div className="menu-separator" />
            <button
              type="button"
              onClick={() => {
                close();
                if (onReenter) onReenter();
                else onNavigate({ kind: 'first-run', step: 'who' });
              }}
            >
              <Icon name="plus" />
              Add an account or server…
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
): { label: string; description: string } | null {
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
  return pending ? { label: '…', description: 'Loading unread counts' } : null;
}

export function Sidebar({
  snapshot,
  location,
  account,
  attention = 0,
  onNavigate,
  onTabNavigate,
  status,
  onReenter,
  onLock,
  collapsed = false,
  agent = 'ready',
  nativeChrome = false,
  blocked = false,
}: SidebarProps): ReactNode {
  const chatInbox = useSidebarInbox();
  const unread = snapshot ? railChatUnread(snapshot, chatInbox) : null;
  const here = railTabOf(location);
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
      className={['side', 'rail', collapsed ? 'is-narrow' : '']
        .filter(Boolean)
        .join(' ')}
      aria-label="Main Navigation"
    >
      <TrafficStrip native={nativeChrome} />
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
              tail={
                tab.id === 'chat' && unread ? (
                  <span
                    className="chat-unread"
                    aria-label={unread.description}
                    title={unread.description}
                  >
                    {unread.label}
                  </span>
                ) : undefined
              }
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
        <div className="foot">
          <AgentLight state={agent} />
        </div>
      </div>
    </nav>
  );
}
