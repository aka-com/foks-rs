/**
 * The header row: one 48px band across the top of the content column.
 *
 * It replaced the two bands the shell used to draw — the title strip and the
 * page's own header — so the breadcrumb, search field, primary New control
 * and sync status share one line. Its empty parts are the window's drag
 * region, because the rail's
 * strip alone does not reach across the window.
 */

import { Fragment, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';

import { Button, Icon, SearchField } from '../components';
import type { DesktopReconciliation } from '../desktop-reconciliation';
import { SYNC_STATUS_ID, SyncPopover, useSyncSummary } from './sync-popover';
import { serverAvailability } from '../model/lease';
import { NewItemButton } from './toolbar';
import { useSidebarInbox } from '../chat/inbox-provider';
import { channelLabel } from '../chat/presentation';
import {
  initials,
  hue as accountHue,
  usernameOf,
  serverDisplayLabelOrLoading,
  storeHues,
  storeNavigationOrder,
  storeOf,
} from '../model';
import type { AgentSnapshot, DeviceLabel, Store } from '../model';
import {
  SETTINGS_SECTION_LABEL,
  railTabOf,
  settingsSectionOf,
} from '../location';
import type { KindFilter, Location, RailTab } from '../location';
import {
  ALL_ITEMS,
  filesFolderCrumb,
  folderKey,
  folderSelection,
} from '../screens/scope';

/** The word each tab is called, as the rail labels it. */
const TAB_LABEL: Readonly<Record<RailTab, string>> = {
  chat: 'Chat',
  files: 'Files',
  teams: 'Teams',
  devices: 'Devices',
  settings: 'Settings',
};

/** Root location for each navigation tab used when clicking its breadcrumb segment. */
const TAB_LOCATION: Readonly<Record<RailTab, Location>> = {
  chat: { kind: 'chat' },
  files: { kind: 'files' },
  teams: { kind: 'teams' },
  devices: { kind: 'devices' },
  settings: { kind: 'settings' },
};

/**
 * The trail the crumbs draw: the tab, then the page under it when the address
 * names one. The tab is always first, so the crumbs answer "where am I" before
 * they answer "in what".
 *
 * `folder` is the Files tree's own selection (`LocationState.folder`), not
 * part of `Location` — selecting a folder there is client-side state, not a
 * navigation. `all`, `store` and `files` all read it through
 * `filesFolderCrumb`, the same selection `ItemsScreen` reads for its own
 * breadcrumb, so the two never disagree.
 */
export function crumbTrail(
  location: Location,
  snapshot?: AgentSnapshot,
  channelName?: string,
  deviceLabel?: DeviceLabel | null,
  folder = '',
): string[] {
  const tab = railTabOf(location);
  if (!tab) return [];
  const trail = [TAB_LABEL[tab]];
  const named = (ref: string): string =>
    (snapshot && storeOf(snapshot, ref)?.name) ?? 'Unknown';
  switch (location.kind) {
    case 'all':
    case 'files':
      trail.push(...filesFolderCrumb(snapshot, location, folder).labels);
      break;
    case 'store': {
      const labels = filesFolderCrumb(snapshot, location, folder).labels;
      trail.push(...(labels.length ? labels : [named(location.ref)]));
      break;
    }
    case 'group-settings':
      trail.push(named(location.ref));
      break;
    case 'chat':
      if (location.ref) {
        trail.push(named(location.ref));
        // The team and the channel are two steps, not one label: the crumbs
        // read "Chat › Engineering › design".
        if (channelName) trail.push(channelName);
      }
      break;
    case 'devices':
      // `section` only scrolls the one list to an anchor now; it does not
      // name a page of its own, so it carries no crumb of its own either.
      if (location.device) {
        trail.push(
          deviceLabel &&
            deviceLabel.store === location.store &&
            deviceLabel.address === location.device
            ? deviceLabel.name
            : 'Key',
        );
      }
      break;
    case 'settings': {
      // The sub-navigation always has one page open, so the crumb always
      // names one — the address's own section, or the page it defaults to.
      const section = settingsSectionOf(location);
      trail.push(SETTINGS_SECTION_LABEL[section]);
      if (section === 'account' && location.profile)
        trail.push(
          serverDisplayLabelOrLoading(
            snapshot?.servers.find(
              (entry) => entry.profileName === location.profile,
            ),
          ),
        );
      break;
    }
    default:
      break;
  }
  return trail;
}

/** Represents a single breadcrumb segment, including label, optional store icon, and click handler. */
interface CrumbSegment {
  label: string;
  /** Associated store when the segment represents a store, used to render its icon. */
  store?: Store;
  onClick?: () => void;
}

/** Computes the scope badge label for the search input based on the active location (e.g. current folder in Files, active channel in Chat, or tab name). */
export function scopeLabel(
  location: Location,
  snapshot?: AgentSnapshot,
  folder = '',
  channelName?: string,
): string {
  const tab = railTabOf(location);
  if (!tab) return '';
  if (tab !== 'files') {
    if (tab === 'chat' && channelName) return `#${channelName}`;
    return TAB_LABEL[tab];
  }
  const labels = filesFolderCrumb(snapshot, location, folder).labels;
  return labels.length ? labels[labels.length - 1] : TAB_LABEL[tab];
}

export interface TopbarProps {
  /** Absent while the agent is starting; the crumbs then name the tab alone. */
  snapshot?: AgentSnapshot;
  deviceLabel?: DeviceLabel | null;
  location: Location;
  /** The Files tree's selected folder (`LocationState.folder`). */
  folder?: string;
  onNavigate: (location: Location) => void;
  /** Steps the Files tree's selection. Omitted where the tree's selection is
   *  not reachable, in which case only the tab's own crumb navigates. */
  onSetFolder?: (folder: string) => void;
  /** The current view's filter text, and the way to set it. */
  query?: string;
  onQuery?: (query: string) => void;
  collapsed: boolean;
  onToggleCollapsed?: () => void;
  /** Files: opens the new-item sheet for one kind. */
  onNew?: (kind: Exclude<KindFilter, 'All'>) => void;
  /** Why New cannot act here, as its tooltip; null when it can. */
  newBlocked?: string | null;
  /** Chat: opens the New chat sheet. */
  onNewChat?: () => void;
  /**
   * Chat: how many teams have chat. With none there is no conversation to
   * create and nothing to search, so the header offers the team the tab needs
   * first and draws its field inert.
   */
  chatTeamCount?: number;
  /** Files: whether the details panel is open, and the way to toggle it. */
  detailsOpen?: boolean;
  onToggleDetails?: () => void;
  refreshing?: boolean;
  onRefresh?: () => void;
  /**
   * The reconciliation service, when the shell has one. With it the sync
   * control carries a worded state and a Refresh status popover; without it
   * it is the plain manual refresh.
   */
  syncService?: DesktopReconciliation;
  /** Opens one server under Settings › Account, from the popover. */
  onOpenServers?: (profile: string) => void;
  /** A blocking state: the bar is drawn, and nothing on it acts. */
  blocked?: boolean;
}

/** How long the status popover survives the pointer leaving it. */
const SYNC_HOVER_CLOSE_MS = 180;

/** The sync control's four states, as a class, an icon and a word. */
function syncFace(state: 'synced' | 'syncing' | 'lapsed' | 'failed' | 'lost'): {
  className: string;
  icon: 'refresh' | 'alert' | 'plug';
  label: string;
} {
  switch (state) {
    case 'syncing':
      return { className: 'sync busy', icon: 'refresh', label: 'Syncing…' };
    case 'lapsed':
      return {
        className: 'sync warn',
        icon: 'alert',
        label: 'Check-in expired',
      };
    case 'failed':
      return { className: 'sync warn', icon: 'alert', label: 'Refresh failed' };
    case 'lost':
      return { className: 'sync bad', icon: 'plug', label: 'Disconnected' };
    default:
      return { className: 'sync', icon: 'refresh', label: 'Synced' };
  }
}

/**
 * The sync control with its status: one worded button that refreshes on
 * click, a spinning icon while any server is refreshing and the failure said
 * in words rather than by a dot. Pointing at the button — or reaching it with
 * the keyboard — opens the per-server popover; there is no separate trigger
 * for it.
 *
 * The popover is portaled, so it is not a descendant of the button's wrapper:
 * the pointer and the focus are tracked on both, and the popover only closes
 * once neither is on either of them. Closing is deferred by
 * `SYNC_HOVER_CLOSE_MS` so the pointer can cross the gap between the two.
 */
function SyncControls({
  snapshot,
  service,
  refreshing,
  blocked,
  onRefresh,
  onOpenServers,
}: {
  snapshot: AgentSnapshot;
  service: DesktopReconciliation;
  refreshing: boolean;
  blocked: boolean;
  onRefresh?: () => void;
  onOpenServers?: (profile: string) => void;
}): ReactNode {
  const summary = useSyncSummary(snapshot, service);
  const wrapRef = useRef<HTMLSpanElement>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  // Whether the pointer or the focus is still on the button or the popover.
  const on = useRef({ pointer: false, focus: false });
  const [open, setOpen] = useState(false);
  const cancelClose = (): void => {
    if (closeTimer.current !== undefined) clearTimeout(closeTimer.current);
    closeTimer.current = undefined;
  };
  useEffect(
    () => () => {
      if (closeTimer.current !== undefined) clearTimeout(closeTimer.current);
    },
    [],
  );
  /** Opens or schedules the close, from whatever the two flags now say. */
  const settle = (): void => {
    cancelClose();
    if ((on.current.pointer || on.current.focus) && !blocked) {
      setOpen(true);
      return;
    }
    closeTimer.current = setTimeout(() => setOpen(false), SYNC_HOVER_CLOSE_MS);
  };
  const track = (key: 'pointer' | 'focus', value: boolean) => (): void => {
    if (key === 'focus' && movingFocus.current) return;
    on.current[key] = value;
    settle();
  };
  const buttonRef = useRef<HTMLButtonElement>(null);
  // Set while the focus is moved programmatically, so the button's own focus
  // event does not count as the reader arriving and reopen what just closed.
  const movingFocus = useRef(false);
  const hideNow = (): void => {
    cancelClose();
    on.current = { pointer: false, focus: false };
    setOpen(false);
    // Escape from inside the popover would otherwise drop the focus on the
    // body, since the anchor the popover restores to is the inert wrapper.
    const active = document.activeElement;
    if (active instanceof HTMLElement && !wrapRef.current?.contains(active)) {
      movingFocus.current = true;
      buttonRef.current?.focus();
      movingFocus.current = false;
    }
  };
  // ArrowDown on the button opens the popover and moves the focus onto its
  // first control; a popover that is already open takes the focus directly.
  const [focusRequest, setFocusRequest] = useState(0);
  const focusFirstControl = (): boolean => {
    const first = document
      .getElementById(SYNC_STATUS_ID)
      ?.querySelector<HTMLElement>('button:not([disabled]), a[href]');
    if (!first) return false;
    first.focus();
    return true;
  };
  useEffect(() => {
    if (!focusRequest || !open) return;
    focusFirstControl();
  }, [focusRequest, open]);
  // Refreshing is one state whichever side reports it: the scheduler's own
  // activity, or the shell's manual refresh. The face, the busy state and
  // what the button will accept are all read off the same word.
  const spinning = summary.refreshing || refreshing;
  // A blocked shell is one whose agent has stopped answering, whatever the
  // servers last reported: the control says that before it says anything
  // about a lease.
  // A lapsed check-in is a lease fact, not a refresh result. Either of the
  // two lease states is a lapse: a check-in that expired and one that was
  // never available both leave the server unusable until it is checked again.
  const lapsed = snapshot.servers.some((server) => {
    const availability = serverAvailability(snapshot, server);
    return (
      !availability.available &&
      (availability.reason === 'check-in-expired' ||
        availability.reason === 'check-in-unavailable')
    );
  });
  // A lapse on one server must not hide a refresh that failed on another:
  // the failure is the one a refresh from here would act on, so it is said
  // first, and the popover lists which server is in which state.
  const face = syncFace(
    blocked
      ? 'lost'
      : spinning
        ? 'syncing'
        : summary.failed
          ? 'failed'
          : lapsed
            ? 'lapsed'
            : 'synced',
  );
  return (
    <span
      className="global-refresh-wrap"
      ref={wrapRef}
      onMouseEnter={track('pointer', true)}
      onMouseLeave={track('pointer', false)}
      onFocus={track('focus', true)}
      onBlur={track('focus', false)}
    >
      <button
        ref={buttonRef}
        type="button"
        className={`global-refresh ${face.className}`}
        onKeyDown={(event) => {
          if (event.key !== 'ArrowDown' || blocked) return;
          event.preventDefault();
          if (open && focusFirstControl()) return;
          on.current.focus = true;
          setOpen(true);
          setFocusRequest((n) => n + 1);
        }}
        // The accessible name carries the state the button shows as well as
        // what pressing it does, so what is read and what is drawn agree.
        aria-label={`${face.label} — ${
          spinning
            ? 'refreshing vaults, teams, chat and devices'
            : 'refresh vaults, teams, chat and devices'
        }`}
        title="Refresh vaults, teams, chat and devices"
        aria-busy={spinning || undefined}
        aria-describedby={open ? SYNC_STATUS_ID : undefined}
        aria-disabled={spinning || undefined}
        disabled={blocked}
        onClick={spinning ? undefined : onRefresh}
      >
        {face.icon === 'refresh' ? null : <Icon name={face.icon} />}
        {face.label === 'Synced' ? (
          <span className="dot ok" aria-hidden="true" />
        ) : null}
        {face.label === 'Refresh failed' ? null : (
          <span className="t">{face.label}</span>
        )}
      </button>
      {open ? (
        <SyncPopover
          snapshot={snapshot}
          service={service}
          summary={summary}
          refreshDisabled={spinning || blocked}
          anchorRef={wrapRef}
          onClose={hideNow}
          onPointerEnter={track('pointer', true)}
          onPointerLeave={track('pointer', false)}
          onFocusEnter={track('focus', true)}
          onFocusLeave={track('focus', false)}
          onOpenServers={onOpenServers}
          onRefresh={blocked ? undefined : onRefresh}
        />
      ) : null}
    </span>
  );
}

/**
 * The breadcrumb's segments. Files builds its own from the tree's selection,
 * so a folder step navigates the tree rather than the location; the other
 * tabs take `crumbTrail`'s words, of which the tab itself and the one step
 * its address still names — a channel's team, a server detail's section —
 * are links.
 */
function crumbSegments(
  location: Location,
  trail: readonly string[],
  snapshot: AgentSnapshot | undefined,
  folder: string,
  onNavigate: (location: Location) => void,
  onSetFolder?: (folder: string) => void,
): CrumbSegment[] {
  const tab = railTabOf(location);
  if (!tab || !trail.length) return [];
  const root: CrumbSegment = {
    label: trail[0],
    onClick: () => {
      if (tab === 'files') onSetFolder?.('');
      onNavigate(TAB_LOCATION[tab]);
    },
  };
  if (tab !== 'files') {
    // The steps a non-Files tab can go back to are the places its address
    // still names: the team a channel belongs to, and the Settings section a
    // server detail was opened from. The last crumb is the page itself, and
    // the renderer draws it as current whether or not it carries a click.
    const inner = trail.slice(1).map((label, index): CrumbSegment => {
      if (tab === 'chat' && index === 0 && location.kind === 'chat') {
        const ref = location.ref;
        if (ref)
          return { label, onClick: () => onNavigate({ kind: 'chat', ref }) };
      }
      if (tab === 'settings' && index === 0 && location.kind === 'settings') {
        const section = settingsSectionOf(location);
        return {
          label,
          onClick: () => onNavigate({ kind: 'settings', section }),
        };
      }
      return { label };
    });
    return [root, ...inner];
  }
  const selected = folderSelection(location, folder);
  if (selected.store === ALL_ITEMS || !snapshot)
    return [root, ...trail.slice(1).map((label) => ({ label }))];
  const store = storeOf(snapshot, selected.store);
  if (!store) return [root, ...trail.slice(1).map((label) => ({ label }))];
  const storePage = location.kind === 'store';
  // `crumbTrail` puts the store first and then one word per folder, which is
  // exactly the sequence of tree selections to step back through.
  const parts = trail.slice(2);
  const segments: CrumbSegment[] = [
    root,
    {
      label: store.name,
      store,
      onClick: () => onSetFolder?.(folderKey(storePage, store.id, '/')),
    },
  ];
  parts.forEach((label, index) => {
    const path = `/${parts.slice(0, index + 1).join('/')}`;
    segments.push({
      label,
      onClick: () => onSetFolder?.(folderKey(storePage, store.id, path)),
    });
  });
  return segments;
}

/** Renders a store avatar badge using its assigned color. */
function CrumbMark({
  store,
  hue,
  accountName,
}: {
  store: Store;
  hue: string | undefined;
  accountName?: string;
}): ReactNode {
  return (
    <span
      className={`crumb-mark${store.kind === 'account' ? ' account' : ''}`}
      style={{ background: accountName ? accountHue(accountName) : hue }}
      aria-hidden="true"
    >
      {accountName
        ? accountName.slice(0, 1).toUpperCase()
        : initials(store.name)}
    </span>
  );
}

export function Topbar({
  snapshot,
  deviceLabel,
  location,
  folder = '',
  onNavigate,
  onSetFolder,
  query,
  onQuery,
  onNew,
  newBlocked = null,
  onNewChat,
  chatTeamCount,
  refreshing = false,
  onRefresh,
  syncService,
  onOpenServers,
  blocked = false,
}: TopbarProps): ReactNode {
  const inbox = useSidebarInbox();
  // The channel's own name, where the shell already knows it: the inbox is
  // loaded for the rail's badge, and the crumb reads it rather than loading
  // anything of its own.
  const open =
    location.kind === 'chat' && location.ref && location.channel
      ? inbox
          .get(location.ref)
          ?.data?.conversations.find(
            (conversation) => conversation.channel.id === location.channel,
          )?.channel
      : undefined;
  const channelName = open ? channelLabel(open) : undefined;
  const trail = crumbTrail(
    location,
    snapshot,
    channelName,
    deviceLabel,
    folder,
  );
  const segments = crumbSegments(
    location,
    trail,
    snapshot,
    folder,
    onNavigate,
    onSetFolder,
  );
  const hues = snapshot
    ? storeHues(storeNavigationOrder(snapshot))
    : new Map<string, string>();
  const tab = railTabOf(location);
  const scope = scopeLabel(location, snapshot, folder, channelName);
  // Only the views that filter accept a query; the rest draw the field inert
  // so the header keeps its shape from page to page.
  const noChatTeams = tab === 'chat' && chatTeamCount === 0;
  const searchable = Boolean(onQuery) && !blocked && !noChatTeams;
  return (
    <div className="topbar" data-tauri-drag-region="">
      <nav className="crumbs" aria-label="Breadcrumb" data-tauri-drag-region="">
        {segments.map((segment, index) => {
          const last = index === segments.length - 1;
          return (
            // Keyed by position as well as text: a store named after its own
            // tab ("Files › Files") would otherwise collide.
            <Fragment key={`${index}-${segment.label}`}>
              {index > 0 ? (
                <span className="sep" aria-hidden="true">
                  <Icon name="chevronDown" />
                </span>
              ) : null}
              <button
                type="button"
                className={last ? 'cur' : undefined}
                aria-current={last ? 'page' : undefined}
                disabled={blocked || last || !segment.onClick}
                onClick={segment.onClick}
              >
                {segment.store ? (
                  <CrumbMark
                    store={segment.store}
                    hue={hues.get(segment.store.id)}
                    accountName={
                      segment.store.kind === 'account' && snapshot
                        ? (usernameOf(snapshot, segment.store) ??
                          segment.store.account)
                        : undefined
                    }
                  />
                ) : null}
                <span className="t">{segment.label}</span>
              </button>
            </Fragment>
          );
        })}
      </nav>
      <span className="grow" data-tauri-drag-region="" />
      {tab === 'files' || tab === 'chat' ? (
        <SearchField
          className="topsearch"
          scope={scope}
          value={query ?? ''}
          onChange={onQuery ?? (() => undefined)}
          placeholder="Search"
          disabled={!searchable}
          shortcut={false}
          kbd
        />
      ) : null}
      {tab === 'files' && onNew ? (
        <NewItemButton onNew={onNew} reason={newBlocked} disabled={blocked} />
      ) : noChatTeams ? (
        // Chat starts in a team: with none, the header's own action is the
        // team, created on the Teams page where that sheet lives.
        <Button
          variant="primary"
          icon="plus"
          className="new-chat"
          disabled={blocked}
          title="Create a team on the Teams page"
          onClick={() => onNavigate({ kind: 'teams', open: 'create' })}
        >
          New team
        </Button>
      ) : tab === 'chat' && onNewChat ? (
        <Button
          variant="primary"
          icon="plus"
          className="new-chat"
          disabled={blocked}
          onClick={onNewChat}
        >
          New chat
        </Button>
      ) : null}
      {onRefresh && syncService && snapshot ? (
        <SyncControls
          snapshot={snapshot}
          service={syncService}
          refreshing={refreshing}
          blocked={blocked}
          onRefresh={onRefresh}
          onOpenServers={onOpenServers}
        />
      ) : onRefresh ? (
        <button
          type="button"
          className={`global-refresh ${syncFace(refreshing ? 'syncing' : 'synced').className}`}
          aria-label={
            refreshing
              ? 'Syncing… — refreshing vaults, teams and devices'
              : 'Synced — refresh vaults, teams and devices'
          }
          title={
            refreshing
              ? 'Refreshing vaults, teams, and devices'
              : 'Refresh vaults and teams'
          }
          disabled={refreshing || blocked}
          onClick={onRefresh}
        >
          {refreshing ? null : <span className="dot ok" aria-hidden="true" />}
          <span className="t">{refreshing ? 'Syncing…' : 'Synced'}</span>
        </button>
      ) : null}
    </div>
  );
}
