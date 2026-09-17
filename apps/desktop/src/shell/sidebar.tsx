/**
 * The navigation rail.
 *
 * Six fixed tabs — People, Chat, Files, Teams, Devices, Settings — under an
 * account header that names the active account and opens the account menu.
 * The rail does not enumerate stores; Files and Teams list them on their own
 * pages. Control-Tab walks the six tabs.
 */

import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Menu, Popover } from '/kit/overlay-primitives';
import { Chip, Icon } from '../components';
import {
  serverChatAvailable,
  storeAvailability,
  storeDescription,
  storeHues,
  storeNavigationOrder,
  storeReadable,
} from '../model';
import type { AccountStore, AgentSnapshot, Store } from '../model';
import type { FoksIconName } from '../icons';
import { useSidebarInbox } from '../chat/inbox-provider';
import { teamUnread } from '../chat/unread';
import { railTabOf } from '../location';
import type { Location, RailTab } from '../location';

/** Whether a store is a team whose server offers chat this account can read. */
export function chatAvailable(snapshot: AgentSnapshot, store: Store): boolean {
  return (
    store.kind === 'team' &&
    store.team_kind === 'named' &&
    store.active !== false &&
    storeReadable(snapshot, store.id) &&
    snapshot.servers.some(
      (server) =>
        server.id === store.server && serverChatAvailable(snapshot, server),
    )
  );
}

interface RailTabSpec {
  id: RailTab;
  label: string;
  icon: FoksIconName;
  location: Location;
}

const RAIL_TABS: readonly RailTabSpec[] = [
  {
    id: 'people',
    label: 'People',
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

/** Control-Tab destinations in rail order. */
export function sidebarCycleLocations(): Location[] {
  return RAIL_TABS.map((tab) => tab.location);
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

export interface NavRowProps {
  active: boolean;
  glyph?: ReactNode;
  name: string;
  /** The amber issue dot. Survives the collapsed rail, where labels do not. */
  dot?: boolean;
  /** Tooltip. The rail sets it only while collapsed, where the label is gone. */
  title?: string;
  tail?: ReactNode;
  onSelect: () => void;
}

export function NavRow({
  active,
  glyph,
  name,
  dot = false,
  title,
  tail,
  onSelect,
}: NavRowProps): ReactNode {
  return (
    <button
      type="button"
      className={active ? 'nav on' : 'nav'}
      title={title}
      aria-current={active ? 'page' : undefined}
      onClick={(event) => {
        onSelect();
        // A mouse click reports `detail > 0`; blur so `:focus-within` does not
        // hold a collapsed rail open. Keyboard activation reports 0 and keeps
        // focus.
        if (event.detail > 0) event.currentTarget.blur();
      }}
    >
      {glyph}
      <span className="t">{name}</span>
      {dot ? <span className="dot" /> : null}
      {tail}
    </button>
  );
}

export interface SidebarProps {
  snapshot: AgentSnapshot;
  location: Location;
  /** How many things need attention. Draws the People tab's dot. */
  attention?: number;
  onNavigate: (location: Location) => void;
  /**
   * Rows between the tabs and the footer. First run puts its progress there;
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
  /** Collapsed to the icon-only rail. */
  collapsed?: boolean;
  /** A width the reader chose: hover no longer expands the rail, focus still does. */
  pinned?: boolean;
  /** Renders the collapse toggle as the footer's last row when provided. */
  onToggleCollapsed?: () => void;
}

/** The footer row that collapses and expands the rail. */
function CollapseToggle({
  collapsed,
  onToggle,
}: {
  collapsed: boolean;
  onToggle: () => void;
}): ReactNode {
  const name = collapsed ? 'Expand' : 'Collapse';
  return (
    <button
      type="button"
      className="nav side-collapse"
      aria-expanded={!collapsed}
      title={name}
      onClick={(event) => {
        onToggle();
        // Blur on mouse activation so the rail does not stay open through
        // `:focus-within`.
        if (event.detail > 0) event.currentTarget.blur();
      }}
    >
      <Icon name={collapsed ? 'panel-hollow' : 'panel-filled'} />
      <span className="t">{name}</span>
    </button>
  );
}

/** The account a `store` parameter names, else the first account store. */
function activeAccount(
  snapshot: AgentSnapshot,
  location: Location,
): AccountStore | undefined {
  const accounts = snapshot.stores.filter(
    (store): store is AccountStore => store.kind === 'account',
  );
  const named =
    'store' in location && location.store
      ? accounts.find((store) => store.id === location.store)
      : undefined;
  return named ?? accounts[0];
}

/**
 * The rail's account header and its menu: the accounts on this Mac grouped by
 * server, then the two commands that are not a place — adding an account and
 * locking the app.
 */
function AccountHeader({
  snapshot,
  location,
  onNavigate,
  onReenter,
  onLock,
}: {
  snapshot: AgentSnapshot;
  location: Location;
  onNavigate: (location: Location) => void;
  onReenter?: () => void;
  onLock?: () => void;
}): ReactNode {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const active = activeAccount(snapshot, location);
  const accounts = snapshot.stores.filter(
    (store): store is AccountStore => store.kind === 'account',
  );
  const usernameOf = (store: AccountStore): string =>
    snapshot.accounts.find((entry) => entry.store === store.id)?.username ??
    store.account;
  const serverNameOf = (store: AccountStore): string =>
    snapshot.servers.find((entry) => entry.id === store.server)?.name ??
    store.server;
  const username = active ? usernameOf(active) : 'No account';
  const server = active ? serverNameOf(active) : 'None on this Mac';
  // Selecting an account keeps the page the reader is on when that page acts
  // on one account, and opens People otherwise.
  const selectAccount = (store: AccountStore): void => {
    if (
      location.kind === 'devices' ||
      location.kind === 'settings' ||
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
  /**
   * Closes the menu. A mouse click leaves the header focused — the menu hands
   * focus back to its anchor once it has unmounted — and a focused header holds
   * a collapsed rail open through `:focus-within`, so mouse activation ends
   * with a blur, as `NavRow` does. The menu queues that restore as a microtask
   * while the click is still being handled, so the blur is queued from inside a
   * microtask of its own to land after it. Keyboard activation (`detail === 0`)
   * keeps focus where the reader put it.
   */
  const close = (event?: { detail: number }): void => {
    setOpen(false);
    if (!event || event.detail === 0) return;
    queueMicrotask(() => {
      queueMicrotask(() => anchorRef.current?.blur());
    });
  };
  return (
    <>
      <button
        type="button"
        ref={anchorRef}
        className="who"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`${username} · ${server}`}
        title={`${username} · ${server}`}
        onClick={(event) => {
          if (open) close(event);
          else setOpen(true);
        }}
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
      {open ? (
        <Popover
          anchorRef={anchorRef}
          className="menu-portal"
          align="start"
          gap={4}
          onClose={() => close()}
        >
          <Menu
            className="menu rail-account-menu"
            anchorRef={anchorRef}
            onClose={() => close()}
            aria-label="Accounts on this Mac"
          >
            {servers.map((serverId) => (
              <div key={serverId}>
                <div className="cap">
                  {snapshot.servers.find((entry) => entry.id === serverId)
                    ?.name ?? serverId}
                </div>
                {accounts
                  .filter((store) => store.server === serverId)
                  .map((store) => {
                    const stopped = !storeAvailability(snapshot, store)
                      .available;
                    return (
                      <button
                        type="button"
                        key={store.id}
                        className={
                          store.id === active?.id ? 'on' : stopped ? 'off' : ''
                        }
                        title={
                          stopped
                            ? storeDescription(snapshot, store)
                            : usernameOf(store)
                        }
                        onClick={(event) => {
                          close(event);
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
                        {stopped ? (
                          <Chip tone="warn">
                            {storeDescription(snapshot, store)}
                          </Chip>
                        ) : null}
                      </button>
                    );
                  })}
              </div>
            ))}
            <div className="menu-separator" />
            <button
              type="button"
              onClick={(event) => {
                close(event);
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
                onClick={(event) => {
                  close(event);
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
    </>
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
  attention = 0,
  onNavigate,
  status,
  onReenter,
  onLock,
  collapsed = false,
  pinned = false,
  onToggleCollapsed,
}: SidebarProps): ReactNode {
  const chatInbox = useSidebarInbox();
  const unread = railChatUnread(snapshot, chatInbox);
  const here = railTabOf(location);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (
        event.key !== 'Tab' ||
        !event.ctrlKey ||
        event.metaKey ||
        event.altKey
      )
        return;
      const next = nextSidebarCycleLocation(location, event.shiftKey ? -1 : 1);
      if (!next) return;
      event.preventDefault();
      onNavigate(next);
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [location, onNavigate]);

  return (
    <nav
      className={[
        'side',
        'rail',
        collapsed ? 'is-narrow' : '',
        pinned ? 'is-pinned' : '',
      ]
        .filter(Boolean)
        .join(' ')}
      aria-label="Main Navigation"
    >
      <AccountHeader
        snapshot={snapshot}
        location={location}
        onNavigate={onNavigate}
        onReenter={onReenter}
        onLock={onLock}
      />
      <div className="rail-tabs">
        {RAIL_TABS.map((tab) => (
          <NavRow
            key={tab.id}
            active={here === tab.id}
            glyph={<Icon name={tab.icon} />}
            name={tab.label}
            // The expanded rail already reads the label; a tooltip repeating it
            // is noise.
            title={collapsed ? tab.label : undefined}
            dot={tab.id === 'people' && attention > 0}
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
              onNavigate(tab.location);
            }}
          />
        ))}
      </div>
      <div className="side-bottom">
        {status}
        {onToggleCollapsed ? (
          <div className="foot">
            <CollapseToggle
              collapsed={collapsed}
              onToggle={onToggleCollapsed}
            />
          </div>
        ) : null}
      </div>
    </nav>
  );
}
