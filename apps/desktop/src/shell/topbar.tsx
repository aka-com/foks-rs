/**
 * Primary 44px topbar in the main panel, aligned with the traffic lights'
 * centerline.
 *
 * On Files, Chat, Activity, and Devices, it renders the list column's search
 * field and add button (Chat renders the bar above its resizable channel column
 * and passes its add control via `add`). On all other screens, it renders
 * breadcrumbs (preceded by a Back button on nested pages such as a team page)
 * and trailing page actions (`BarActions`). Unoccupied regions act as a window
 * drag handle. When the sidebar is collapsed, the bar also renders the traffic
 * lights and the sidebar expand button at the leading edge.
 */

import { Fragment, useId } from 'react';
import type { ReactNode } from 'react';

import { Icon, SearchField, StoreMark } from '../components';
import { ariaShortcut } from '../components/menus';
import { NewItemButton } from './toolbar';
import { useSidebarInbox } from '../chat/inbox-provider';
import { channelLabel } from '../chat/presentation';
import { serverDisplayLabelOrLoading, storeOf } from '../model';
import type { AgentSnapshot, DeviceLabel, Store } from '../model';
import {
  SETTINGS_SECTION_LABEL,
  parentLocation,
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
  activity: 'Activity',
  devices: 'Devices',
  settings: 'Settings',
};

/** Root location for each navigation tab used when clicking its breadcrumb segment. */
const TAB_LOCATION: Readonly<Record<RailTab, Location>> = {
  chat: { kind: 'chat' },
  files: { kind: 'files' },
  teams: { kind: 'teams' },
  activity: { kind: 'activity' },
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

/**
 * Returns the active location's title for the window title and page heading:
 * the folder or store name on Files (or All Items, Passwords, Documents), the
 * team name on a team page, All Teams on the teams index, and the final
 * breadcrumb label elsewhere.
 */
export function placeTitle(
  location: Location,
  snapshot?: AgentSnapshot,
  folder = '',
  kind: KindFilter = 'All',
): string {
  const tab = railTabOf(location);
  if (!tab) return '';
  if (tab === 'files') {
    const labels = filesFolderCrumb(snapshot, location, folder).labels;
    if (labels.length) return labels[labels.length - 1];
    if (location.kind === 'store')
      return (snapshot && storeOf(snapshot, location.ref)?.name) ?? 'Files';
    return kind === 'Password'
      ? 'Passwords'
      : kind === 'Document'
        ? 'Documents'
        : 'All Items';
  }
  if (location.kind === 'teams' && !location.ref) return 'All Teams';
  return crumbTrail(location, snapshot).at(-1) ?? TAB_LABEL[tab];
}

export interface TopbarProps {
  /**
   * Combined badge summary when the sidebar is collapsed (pending requests,
   * device alerts, settings alerts, or unrouted notices). When set, the
   * sidebar expand button renders a warning dot and announces this description.
   */
  sidebarAttention?: string;
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
  /** Files: opens the new-item sheet for the specified kind. */
  onNew?: (kind: Exclude<KindFilter, 'All'>) => void;
  /** Files: opens the New Folder sheet in the target store and folder. */
  onNewFolder?: () => void;
  /** Controls rendered before the add button (e.g. the Files view menu). */
  actions?: ReactNode;
  /**
   * Ref callback receiving the portal target for page-specific `BarActions`,
   * rendered after `actions`.
   */
  actionsSlot?: (element: HTMLElement | null) => void;
  /** Tooltip reason explaining why New is disabled, or `null` when enabled. */
  newBlocked?: string | null;
  /**
   * Custom add control for the list column, rendered in place of the default
   * tab add control (e.g. Chat's New channel or Devices' Add a device).
   */
  add?: ReactNode;
  /**
   * Number of chat-capable teams on the Chat tab; when 0, the search input is
   * disabled.
   */
  chatTeamCount?: number;
  /** Disables all topbar controls during a blocking state. */
  blocked?: boolean;
  /** True when the OS draws native traffic lights; false in the web mock. */
  nativeChrome?: boolean;
}

/**
 * Builds interactive breadcrumb segments. Files builds segments from the folder
 * tree selection so clicking a parent folder updates the tree selection rather
 * than navigating routes; other tabs use `crumbTrail` labels and link the root
 * tab segment (plus the team segment on a Chat channel route).
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
    // On non-Files tabs, link intermediate segments that have an addressable
    // parent route (such as a channel's team). The final segment represents the
    // current page.
    const inner = trail.slice(1).map((label, index): CrumbSegment => {
      if (tab === 'chat' && index === 0 && location.kind === 'chat') {
        const ref = location.ref;
        if (ref)
          return { label, onClick: () => onNavigate({ kind: 'chat', ref }) };
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

export function Topbar({
  snapshot,
  deviceLabel,
  location,
  folder = '',
  onNavigate,
  onSetFolder,
  query,
  onQuery,
  collapsed,
  onToggleCollapsed,
  onNew,
  onNewFolder,
  actions,
  actionsSlot,
  newBlocked = null,
  add,
  chatTeamCount,
  blocked = false,
  nativeChrome = false,
  sidebarAttention,
}: TopbarProps): ReactNode {
  const attentionId = useId();
  const inbox = useSidebarInbox();
  // Read the active channel name from the shared sidebar inbox cache without
  // issuing a separate read.
  const open =
    location.kind === 'chat' && location.ref && location.channel
      ? inbox
          .get(location.ref)
          ?.data?.conversations.find(
            (conversation) => conversation.channel.id === location.channel,
          )?.channel
      : undefined;
  const channelName = open ? channelLabel(open) : undefined;
  const tab = railTabOf(location);
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
  const scope = scopeLabel(location, snapshot, folder, channelName);
  // Only the views that filter accept a query; the rest draw no field.
  const noChatTeams = tab === 'chat' && chatTeamCount === 0;
  const searchable = Boolean(onQuery) && !blocked && !noChatTeams;
  // Files, Chat, Activity, and Devices use the list-column bar (search input
  // and add button); other screens render breadcrumbs and an optional Back button.
  const listBar =
    tab === 'files' ||
    tab === 'chat' ||
    tab === 'activity' ||
    tab === 'devices';
  const parent =
    !listBar && tab !== 'settings' ? parentLocation(location) : null;
  // On Settings, show only "Settings" in the topbar breadcrumb; the
  // sub-navigation highlights the active section, and server detail pages
  // render their own back link to Account.
  const shownSegments = tab === 'settings' ? segments.slice(0, 1) : segments;
  // The chevron navigates to the enclosing page rather than stepping back in
  // history (⌘[), so label it with the destination ("All Teams" or the
  // preceding breadcrumb segment).
  const parentName =
    parent?.kind === 'teams'
      ? 'All Teams'
      : (shownSegments.at(-2)?.label ?? 'Enclosing Page');
  return (
    <div
      className={listBar ? 'topbar bar list-bar' : 'topbar bar'}
      data-tauri-drag-region=""
    >
      {collapsed && onToggleCollapsed ? (
        nativeChrome ? (
          <span className="lights-space" aria-hidden="true" />
        ) : (
          <span className="lights" aria-hidden="true">
            <span className="light" />
            <span className="light" />
            <span className="light" />
          </span>
        )
      ) : null}
      {collapsed && onToggleCollapsed ? (
        <button
          type="button"
          className="tbtn side-expand"
          data-focus-key="sidebar-toggle"
          aria-keyshortcuts={ariaShortcut('⌃⌘S')}
          aria-expanded="false"
          aria-label="Expand sidebar"
          aria-describedby={sidebarAttention ? attentionId : undefined}
          title={
            sidebarAttention
              ? `Expand sidebar: ${sidebarAttention}`
              : 'Expand sidebar'
          }
          disabled={blocked}
          onClick={onToggleCollapsed}
        >
          <Icon name="panelLeft" />
          {sidebarAttention ? (
            <>
              <span className="dot warn" aria-hidden="true" />
              <span id={attentionId} className="offscreen">
                {sidebarAttention}
              </span>
            </>
          ) : null}
        </button>
      ) : null}
      {parent ? (
        <button
          type="button"
          className="tbtn back"
          aria-label={parentName}
          title={parentName}
          disabled={blocked}
          onClick={() => onNavigate(parent)}
        >
          <Icon name="chevronDown" className="chev-left" />
        </button>
      ) : null}
      {listBar ? null : (
        <nav
          className="crumbs"
          aria-label="Breadcrumb"
          data-tauri-drag-region=""
        >
          {shownSegments.map((segment, index) => {
            const last = index === shownSegments.length - 1;
            return (
              // Key by index and label so a store named "Files" does not
              // collide with the root "Files" segment.
              <Fragment key={`${index}-${segment.label}`}>
                {index > 0 ? (
                  <span className="sep" aria-hidden="true">
                    <Icon name="chevronRight" />
                  </span>
                ) : null}
                <button
                  type="button"
                  className={last ? 'cur' : undefined}
                  aria-current={last ? 'page' : undefined}
                  disabled={blocked || last || !segment.onClick}
                  onClick={segment.onClick}
                >
                  {segment.store && snapshot ? (
                    <StoreMark snapshot={snapshot} store={segment.store} />
                  ) : null}
                  <span className="t">{segment.label}</span>
                </button>
              </Fragment>
            );
          })}
        </nav>
      )}
      {listBar ? (
        <SearchField
          className="topsearch"
          scope={scope}
          value={query ?? ''}
          onChange={onQuery ?? (() => undefined)}
          placeholder={tab === 'chat' ? 'Search channels' : 'Search'}
          disabled={!searchable}
          results=".main [role='listbox']"
        />
      ) : (
        <span className="grow" data-tauri-drag-region="" />
      )}
      {actions}
      {actionsSlot ? <span className="bar-acts" ref={actionsSlot} /> : null}
      {add !== undefined ? (
        add
      ) : tab === 'files' && onNew ? (
        <NewItemButton
          onNew={onNew}
          onNewFolder={onNewFolder}
          reason={newBlocked}
          disabled={blocked}
        />
      ) : null}
    </div>
  );
}
