import { inventoryReadiness, itemsReadiness } from '../model';
/**
 * Application sidebar: a source list rendered on the window background.
 *
 * Layout from top to bottom:
 * - Window drag strip with traffic lights, product name, and sidebar collapse toggle;
 * - Kind filter rows: All Items, Passwords, and Documents;
 * - Vaults and Teams groups with each store's folder tree (`files-tree.tsx`) and All Teams;
 * - Chat and Activity;
 * - Devices and Settings;
 * - Footer containing the "Finish setting up" card and sync status control
 *   above a divider, with the account switcher below.
 *
 * Rows render icons and counts in muted sidebar text (`--side-muted`) and use a
 * low-contrast selection fill (`--side-sel`). Counts display plain numbers, while
 * status alerts use colored dots. Control-Tab cycles through top-level sidebar
 * sections in display order.
 *
 * When collapsed, the sidebar stays mounted (`hidden`) so its state and
 * Control-Tab listener remain active, and the panel topbar renders the expand
 * button after the traffic lights.
 */

import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type FocusEvent as ReactFocusEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  type RefObject,
} from 'react';
import { Menu, Popover, anyDialogOpen } from '/kit/overlay-primitives';
import { Icon } from '../components';
import { ariaShortcut } from '../components/menus';
import { useSidebarResize } from './use-sidebar-resize';
import { FilesTree, type FilesTreeProps } from './files-tree';
import {
  catalog,
  chatAvailable,
  kindOf,
  localAliasOf,
  serverDisplayLabelOrLoading,
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
import type { KindFilter, Location, RailTab } from '../location';
import { AccountMark } from '../screens/account-switcher';
import { ALL_ITEMS, folderSelection, listsItems } from '../screens/scope';

/**
 * Top-level sections visited by Control-Tab, in sidebar display order. `files`
 * opens All Items, `teams` opens the Teams list, and `chat` resumes the last
 * open team and channel.
 */
const CYCLE: readonly RailTab[] = [
  'files',
  'teams',
  'chat',
  'activity',
  'devices',
  'settings',
];

function placeLocation(tab: RailTab): Location {
  switch (tab) {
    case 'chat':
      return chatTabLocation();
    case 'files':
      return { kind: 'files' };
    case 'teams':
      return { kind: 'teams' };
    case 'activity':
      return { kind: 'activity' };
    case 'devices':
      return { kind: 'devices' };
    case 'settings':
      return { kind: 'settings' };
  }
}

/** Returns Control-Tab target locations in sidebar order. */
export function sidebarCycleLocations(): Location[] {
  return CYCLE.map(placeLocation);
}

export function nextSidebarCycleLocation(
  current: Location,
  delta: 1 | -1,
): Location | null {
  const places = sidebarCycleLocations();
  // Map the current location to its owning sidebar section; first-run locations
  // have no section and start at the first or last section.
  const here = railTabOf(current);
  const index = CYCLE.findIndex((tab) => tab === here);
  if (index < 0) return delta > 0 ? places[0] : places[places.length - 1];
  return places[(index + delta + places.length) % places.length];
}

/* --------------------------------------------------------- window controls -- */

/**
 * 52px window drag strip at the top of the sidebar, aligning the traffic lights
 * (`trafficLightPosition {x: 14, y: 24}`) and the 44px panel topbars (inset 8px)
 * on a shared y=30 centerline. On macOS, native window controls are drawn by the
 * OS; in the web mock, simulated traffic lights are rendered.
 */
export function TrafficStrip({
  native = false,
  children,
}: {
  native?: boolean;
  children?: ReactNode;
}): ReactNode {
  return (
    <div className="traffic side-head" data-tauri-drag-region="">
      {native ? (
        <span className="lights-space" aria-hidden="true" />
      ) : (
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

/** Connection states the sidebar can report for the agent. */
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
 * Agent status indicator (colored dot and label) used as a fallback in the
 * sidebar footer when no reconciliation service status line is provided (e.g.
 * during first-run setup or when the shell is blocked).
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
  title?: string;
  tail?: ReactNode;
  /** Disables the row while the shell is in a blocking state. */
  disabled?: boolean;
  /** Identifier set on `data-place` for styling and tests. */
  place?: string;
  onSelect: () => void;
}

/** Sidebar navigation row with leading icon, label, and optional trailing count or status dot. */
export function NavRow({
  active,
  glyph,
  name,
  title,
  tail,
  disabled = false,
  place,
  onSelect,
}: NavRowProps): ReactNode {
  return (
    <button
      type="button"
      className={active ? 'nav on' : 'nav'}
      title={title}
      disabled={disabled || undefined}
      aria-current={active ? 'page' : undefined}
      data-place={place}
      onClick={onSelect}
    >
      {glyph}
      <span className="t">{name}</span>
      {tail ? <TailSeparator /> : null}
      {tail}
    </button>
  );
}

/**
 * Offscreen comma separator between a row's name and its trailing count or
 * status dot so VoiceOver announces "Activity, 3 requests" instead of "Activity3".
 */
export function TailSeparator(): ReactNode {
  return <span className="offscreen">, </span>;
}

/**
 * Manages roving-tabindex keyboard navigation for the sidebar source list:
 * a single Tab stop, ArrowUp/ArrowDown/Home/End to move focus between rows,
 * and Enter or Space to activate a row. Moving focus does not trigger
 * navigation because opening a location is guarded and may prompt or initiate
 * reads; store tree rows add ArrowRight and ArrowLeft (`files-tree.tsx`).
 * The active Tab stop is the focused row, or the row with `aria-current`, or
 * the first enabled row.
 */
function useSourceListKeys(root: RefObject<HTMLElement | null>): {
  onKeyDown: (event: ReactKeyboardEvent<HTMLElement>) => void;
  onFocus: (event: ReactFocusEvent<HTMLElement>) => void;
} {
  const rows = (): HTMLButtonElement[] =>
    Array.from(
      root.current?.querySelectorAll<HTMLButtonElement>(
        'button.nav:not(:disabled)',
      ) ?? [],
    );
  const rove = (focused?: Element | null): void => {
    const all = rows();
    const current =
      all.find((row) => row === focused) ??
      all.find((row) => row === document.activeElement) ??
      all.find((row) => row.hasAttribute('aria-current')) ??
      all[0];
    for (const row of all) row.tabIndex = row === current ? 0 : -1;
  };
  // Recompute the active Tab stop whenever rows mount/unmount (such as
  // expanding a tree or adding a store) or `aria-current` / `disabled` changes.
  useLayoutEffect(() => {
    const element = root.current;
    if (!element) return;
    rove();
    const observer = new MutationObserver(() => rove());
    observer.observe(element, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ['aria-current', 'disabled'],
    });
    return () => observer.disconnect();
  });
  return {
    onFocus: (event) => {
      if (
        event.target instanceof HTMLButtonElement &&
        event.target.matches('button.nav')
      )
        rove(event.target);
    },
    onKeyDown: (event) => {
      if (
        !['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key) ||
        event.altKey ||
        event.metaKey ||
        event.ctrlKey ||
        !(event.target instanceof HTMLElement) ||
        !event.target.matches('button.nav')
      )
        return;
      const all = rows();
      const at = all.indexOf(event.target as HTMLButtonElement);
      const next =
        event.key === 'Home'
          ? all[0]
          : event.key === 'End'
            ? all[all.length - 1]
            : all[at + (event.key === 'ArrowDown' ? 1 : -1)];
      event.preventDefault();
      if (!next) return;
      next.focus();
      next.scrollIntoView?.({ block: 'nearest' });
    },
  };
}

/** Configuration and action callbacks for the sidebar setup card. */
export interface RailSetup {
  /** Number of completed steps. */
  done: number;
  total: number;
  /** Short label for the next setup step. */
  next: string;
  onContinue: () => void;
  /** Dismisses the card for the current session ("Later"); when omitted, the Later button is hidden. */
  onLater?: () => void;
}

/** Files selection state read and updated by the sidebar's item filter rows and store tree. */
export interface SidebarFiles {
  /** `LocationState.folder`. */
  folder: string;
  /** `LocationState.kind`. */
  kind: KindFilter;
  /**
   * Opens a Files view: `folder` in `all`-location format (`''` for All Items,
   * `store|/path` for a store or folder) and an item `kind` filter. Bound to
   * `LocationStore.openFiles` by the shell.
   */
  onOpen: (folder: string, kind: KindFilter) => void;
  folderActions?: FilesTreeProps['folderActions'];
  accessNow?: () => number;
}

export interface SidebarProps {
  /** Omitted while the agent is starting; the sidebar still renders its frame. */
  snapshot?: AgentSnapshot;
  location: Location;
  account?: StoreRef;
  /**
   * Count of unrouted notices (such as retryable catalog read failures or
   * notices without a specific destination). When non-zero, displays an
   * attention dot on the account avatar and on the Settings row.
   */
  attention?: number;
  /** Pending membership request badge for the All Teams row. */
  teamRequests?: { label: string; description: string } | null;
  /**
   * Badge count for the Activity row: known membership requests plus activity
   * notices (`activityBadge`).
   */
  activity?: { label: string; description: string } | null;
  /** Warning dot for the Devices row (open pairing offer or missing recovery phrase). */
  devicesAlert?: { description: string } | null;
  /**
   * Warning dot for the Settings row (expired check-in or unverified server).
   * Combined with `attention` when both are present.
   */
  settingsAlert?: { description: string } | null;
  onNavigate: (location: Location) => void;
  onTabNavigate?: (tab: RailTab) => void;
  /** Files selection state; when omitted, item and store rows navigate by route. */
  files?: SidebarFiles;
  /** Optional status content rendered above the setup card (used for first-run invitation notes). */
  status?: ReactNode;
  /** Incomplete first-run progress rendered as the "Finish setting up" card. */
  setup?: RailSetup;
  /**
   * Re-enters the active first-run session from "Add Account or Server…"
   * when already on a first-run screen.
   */
  onReenter?: () => void;
  /** Locks the application; omitted when locking is unavailable. */
  onLock?: () => void;
  /** Collapses the sidebar (`hidden`); the panel topbar renders the expand button. */
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
  /** Agent connection state used when `statusLine` is omitted. */
  agent?: RailAgentState;
  /** Sync status control rendered above the account divider. */
  statusLine?: ReactNode;
  /** True when the OS draws native window controls over the drag strip. */
  nativeChrome?: boolean;
  /** Dims and disables sidebar controls during a blocking state. */
  blocked?: boolean;
}

/**
 * The account switcher and its menu, at the foot of the sidebar: the
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
  const accountReadiness = inventoryReadiness(snapshot, 'accounts');
  const username = active
    ? usernameOf(active)
    : accountReadiness === 'loading'
      ? 'Loading accounts…'
      : accounts.length || accountReadiness === 'unavailable'
        ? 'Account unavailable'
        : 'No account';
  const activeServer = active
    ? snapshot.servers.find(
        (candidate) => candidate.profileName === active.server,
      )
    : undefined;
  const server = active
    ? serverDisplayLabelOrLoading(activeServer)
    : accountReadiness === 'loading'
      ? 'Reading this device’s accounts'
      : accounts.length || accountReadiness === 'unavailable'
        ? 'Choose an account'
        : 'None on this device';
  // The foot names the host the account lives on. The reader's own label for
  // that server is the accessible name and the tooltip, where a second line
  // costs nothing; the row itself has one line for it and a hostname is what
  // distinguishes two accounts of the same name.
  const host = active ? (activeServer?.configuredEndpoint ?? server) : server;
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
      onNavigate({ kind: 'teams', store: store.id });
    else onNavigate({ kind: 'settings', section: 'account', store: store.id });
  };
  const servers = [...new Set(accounts.map((store) => store.server))];
  // Account marks use the same username-derived hue as the Files vault row.
  const close = (): void => setOpen(false);
  return (
    <div className={blocked ? 'side-account is-blocked' : 'side-account'}>
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
          <Icon name="chevronsUpDown" />
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
          // Use `inset={0}` because the sidebar is flush with the window's left edge.
          inset={0}
          onClose={close}
        >
          <Menu
            className="menu account-menu"
            anchorRef={anchorRef}
            onClose={close}
            aria-label="Accounts on this device"
          >
            {servers.map((serverId) => {
              const serverLabel = serverDisplayLabelOrLoading(
                snapshot.servers.find(
                  (entry) => entry.profileName === serverId,
                ),
              );
              return (
                <div key={serverId} role="group" aria-label={serverLabel}>
                  {/* Server group header for the accounts listed below. */}
                  <div className="mh" aria-hidden="true">
                    <Icon name="server" />
                    {serverLabel}
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
                            role="menuitemradio"
                            aria-checked={store.id === active?.id}
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
                            {/* Leading checkmark on the active account row. */}
                            <span
                              className={
                                store.id === active?.id ? 'tick' : 'tick off'
                              }
                              aria-hidden="true"
                            >
                              <Icon name="check" />
                            </span>
                            <span className="t">
                              {usernameOf(store)}
                              <small>
                                {localAliasOf(snapshot, store)} · {serverLabel}
                              </small>
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
                                  {checkIn ? 'Check in' : 'Settings'}
                                </button>
                              </span>
                            </div>
                          ) : null}
                        </div>
                      );
                    })}
                </div>
              );
            })}
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
              Add Account or Server…
            </button>
            {onLock ? (
              <button
                type="button"
                onClick={() => {
                  close();
                  onLock();
                }}
              >
                <Icon name="lock" />
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
 * reported by the sync status line.
 *
 * A team whose synchronization failed or arrived incomplete marks the badge
 * rather than replacing it: an inbox poll fails and retries on a backoff, and
 * a count that vanished on each failed poll would read as messages going away.
 * The count that is known is still drawn, in the warning colour, and the
 * description says what is unaccounted for. Only when no team could be counted
 * at all does the badge fall back to a warning dot with nothing to show.
 */
function chatUnread(
  snapshot: AgentSnapshot,
  inbox: ReturnType<typeof useSidebarInbox>,
): NavCount | { warning: true; description: string } | null {
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

type NavCount = { label: string; description: string; warning?: boolean };

/**
 * Trailing indicator for a sidebar row: a muted count, an amber warning count
 * (when a count is a known lower bound), or a status dot.
 */
function NavTail({
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
      className={`nav-tail ${kind}`}
      role="img"
      aria-label={description}
      title={description}
    >
      {children}
    </span>
  );
}

/**
 * "Finish setting up" card in the sidebar footer: clicking the heading or
 * Continue resumes setup at the next incomplete step, and Later dismisses the
 * card for the session. Below 720px window height, CSS collapses the card to a
 * single row containing the heading and dismiss button.
 */
function SetupCard({
  setup,
  disabled,
}: {
  setup?: RailSetup;
  disabled: boolean;
}): ReactNode {
  if (!setup) return null;
  const { done, total, next, onContinue, onLater } = setup;
  return (
    <div
      className="setup-card"
      role="group"
      aria-label={`Finish setting up: ${done} of ${total} steps done`}
    >
      <button
        type="button"
        className="h"
        disabled={disabled}
        onClick={onContinue}
      >
        <Icon name="flag" />
        <span className="lbl">Finish setting up</span>
      </button>
      <span className="step">
        <i aria-hidden="true" />
        {next}
      </span>
      <span className="setup-acts">
        <button
          type="button"
          className="lnk"
          disabled={disabled}
          onClick={onContinue}
        >
          Continue
        </button>
        {onLater ? (
          <button
            type="button"
            className="lnk muted"
            disabled={disabled}
            onClick={onLater}
          >
            Later
          </button>
        ) : null}
      </span>
      {onLater ? (
        <button
          type="button"
          className="tbtn x"
          aria-label="Later"
          title="Later"
          disabled={disabled}
          onClick={onLater}
        >
          <Icon name="close" />
        </button>
      ) : null}
    </div>
  );
}

const KIND_ROWS: readonly {
  kind: KindFilter;
  label: string;
  icon: FoksIconName;
  place: string;
}[] = [
  { kind: 'All', label: 'All Items', icon: 'grid', place: 'files' },
  { kind: 'Password', label: 'Passwords', icon: 'key', place: 'passwords' },
  {
    kind: 'Document',
    label: 'Documents',
    icon: 'fileText',
    place: 'documents',
  },
];

export function Sidebar({
  snapshot,
  location,
  account,
  attention = 0,
  teamRequests = null,
  activity = null,
  devicesAlert = null,
  settingsAlert = null,
  onNavigate,
  onTabNavigate,
  files,
  status,
  setup,
  onReenter,
  onLock,
  collapsed = false,
  onToggleCollapsed,
  agent = 'ready',
  statusLine,
  nativeChrome = false,
  blocked = false,
}: SidebarProps): ReactNode {
  const resize = useSidebarResize(!collapsed && !blocked);
  const sourceList = useSourceListKeys(resize.ref);
  // Track whether the navigation list overflows above the footer (for example,
  // in a 640px window with an expanded tree) to show a bottom divider and
  // scroll the active row into view.
  const scrollRef = useRef<HTMLDivElement>(null);
  const [overflows, setOverflows] = useState(false);
  const measureOverflow = (): void => {
    const element = scrollRef.current;
    if (!element) return;
    setOverflows(
      element.scrollHeight - element.scrollTop - element.clientHeight > 1,
    );
  };
  useLayoutEffect(measureOverflow);
  const currentFolder = files?.folder;
  useEffect(() => {
    // Scroll only when the active location or folder changes so badge updates
    // do not reset the user's scroll position.
    scrollRef.current
      ?.querySelector<HTMLElement>('.nav[aria-current]')
      ?.scrollIntoView?.({ block: 'nearest' });
  }, [location, currentFolder]);
  const chatInbox = useSidebarInbox();
  const unread = snapshot ? chatUnread(snapshot, chatInbox) : null;
  const here = railTabOf(location);
  const go = (tab: RailTab): void => {
    if (onTabNavigate) onTabNavigate(tab);
    else onNavigate(placeLocation(tab));
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
      // Disable cycling while a dialog is open so the view under it does not
      // change.
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

  // Catalog-wide item counts, shown once all stores have loaded.
  const items = snapshot ? catalog(snapshot) : [];
  const countsReady = snapshot ? itemsReadiness(snapshot) === 'ready' : false;
  const kindCount = (kind: KindFilter): number =>
    kind === 'All'
      ? items.length
      : items.filter((item) => kindOf(item) === kind).length;
  const allItemsHere =
    listsItems(location) &&
    (files
      ? folderSelection(location, files.folder).store === ALL_ITEMS
      : location.kind !== 'store');
  const kindRow = (row: (typeof KIND_ROWS)[number]): ReactNode => (
    <NavRow
      key={row.kind}
      place={row.place}
      active={allItemsHere && (files?.kind ?? 'All') === row.kind}
      glyph={<Icon name={row.icon} />}
      name={row.label}
      disabled={blocked}
      tail={
        countsReady ? <span className="n">{kindCount(row.kind)}</span> : null
      }
      onSelect={() => {
        if (files) files.onOpen('', row.kind);
        else go('files');
      }}
    />
  );

  const settingsReasons = [
    settingsAlert?.description,
    attention > 0
      ? `${attention} ${attention === 1 ? 'notice needs' : 'notices need'} attention`
      : undefined,
  ].filter((reason): reason is string => Boolean(reason));

  const chatTail = !unread ? undefined : !('label' in unread) ? (
    <NavTail kind="dot warn" description={unread.description} />
  ) : (
    <NavTail
      kind={unread.warning ? 'count warn' : 'count'}
      description={unread.description}
    >
      {unread.label}
    </NavTail>
  );

  const allTeams = (
    <NavRow
      place="teams"
      active={here === 'teams'}
      glyph={<Icon name="users" />}
      name="All Teams"
      disabled={blocked}
      tail={
        teamRequests ? (
          <NavTail kind="count" description={teamRequests.description}>
            {teamRequests.label}
          </NavTail>
        ) : undefined
      }
      // Navigate directly to the Teams list rather than the last open team page.
      onSelect={() => onNavigate({ kind: 'teams' })}
    />
  );

  return (
    <nav
      ref={resize.ref}
      className={['side', resize.dragging ? 'is-resizing' : '']
        .filter(Boolean)
        .join(' ')}
      hidden={collapsed}
      aria-label="Main Navigation"
      onKeyDown={sourceList.onKeyDown}
      onFocus={sourceList.onFocus}
    >
      {resize.handle}
      <TrafficStrip native={nativeChrome}>
        <span className="brand">FOKS</span>
        {onToggleCollapsed ? (
          <button
            type="button"
            className="tbtn side-collapse"
            // Share `data-focus-key` with the topbar expand button so focus
            // transfers to it when collapsing the sidebar (`focus-keeper`).
            data-focus-key="sidebar-toggle"
            aria-keyshortcuts={ariaShortcut('⌃⌘S')}
            aria-expanded="true"
            aria-label="Collapse sidebar"
            title="Collapse sidebar"
            disabled={blocked}
            onClick={onToggleCollapsed}
          >
            <Icon name="panelLeft" />
          </button>
        ) : null}
      </TrafficStrip>
      <div
        ref={scrollRef}
        className={[
          'side-scroll',
          blocked ? 'is-blocked' : '',
          overflows ? 'overflows' : '',
        ]
          .filter(Boolean)
          .join(' ')}
        onScroll={measureOverflow}
      >
        <div className="nav-group">{KIND_ROWS.map(kindRow)}</div>
        {snapshot ? (
          <FilesTree
            snapshot={snapshot}
            location={location}
            folder={files?.folder ?? ''}
            onOpen={(folder) => {
              if (files) {
                files.onOpen(folder, 'All');
                return;
              }
              const store = folder.slice(0, folder.indexOf('|'));
              onNavigate({ kind: 'store', ref: store });
            }}
            onNavigate={onNavigate}
            folderActions={files?.folderActions}
            accessNow={files?.accessNow}
            teamsTail={allTeams}
            disabled={blocked}
          />
        ) : (
          <div className="nav-group">{allTeams}</div>
        )}
        <div className="nav-group">
          <NavRow
            place="chat"
            active={here === 'chat'}
            glyph={<Icon name="chat" />}
            name="Chat"
            disabled={blocked}
            tail={chatTail}
            onSelect={() => go('chat')}
          />
          <NavRow
            place="activity"
            active={here === 'activity'}
            glyph={<Icon name="activity" />}
            name="Activity"
            disabled={blocked}
            tail={
              activity ? (
                <NavTail kind="count" description={activity.description}>
                  {activity.label}
                </NavTail>
              ) : undefined
            }
            onSelect={() => go('activity')}
          />
        </div>
        <div className="nav-group">
          <NavRow
            place="devices"
            active={here === 'devices'}
            glyph={<Icon name="shield" />}
            name="Devices"
            disabled={blocked}
            tail={
              devicesAlert ? (
                <NavTail
                  kind="dot warn"
                  description={devicesAlert.description}
                />
              ) : undefined
            }
            onSelect={() => go('devices')}
          />
          <NavRow
            place="settings"
            active={here === 'settings'}
            glyph={<Icon name="settings" />}
            name="Settings"
            disabled={blocked}
            tail={
              settingsReasons.length ? (
                <NavTail
                  kind="dot warn"
                  description={settingsReasons.join('; ')}
                />
              ) : undefined
            }
            onSelect={() => go('settings')}
          />
        </div>
      </div>
      <div className={blocked ? 'side-foot is-blocked' : 'side-foot'}>
        {status}
        <SetupCard setup={setup} disabled={blocked} />
        <div className="side-status">
          {statusLine ?? <AgentLight state={agent} />}
        </div>
        <div className="foot-rule" aria-hidden="true" />
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
          // Reserve the account switcher height before accounts load so the
          // rows above do not shift when the snapshot arrives.
          <div className="side-account">
            <div className="who who-empty">
              <span className="avatar" aria-hidden="true" />
            </div>
          </div>
        )}
      </div>
    </nav>
  );
}
