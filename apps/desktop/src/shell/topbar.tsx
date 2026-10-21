/**
 * The panel's first bar: 44px, on the traffic lights' centreline.
 *
 * On Files, Chat and Activity it is the list column's bar: the search field
 * and the column's add control. On every other page it names the page with
 * the crumbs, led by Back on a page inside an area (a team, a device), and
 * ends with the page's own commands (`BarActions`). Its empty parts are the
 * window's drag region. While the sidebar is hidden the bar
 * also carries the control that shows it again, after the traffic lights.
 */

import { Fragment } from 'react';
import type { ReactNode } from 'react';

import { Button, Icon, SearchField, StoreMark } from '../components';
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
  /** Files: opens the New folder sheet where New items would be saved. */
  onNewFolder?: () => void;
  /** Controls drawn before the add control: the Files view menu. */
  actions?: ReactNode;
  /**
   * Receives the element a page's `BarActions` portal into, drawn after
   * `actions`. The shell passes its state setter.
   */
  actionsSlot?: (element: HTMLElement | null) => void;
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
  /** A blocking state: the bar is drawn, and nothing on it acts. */
  blocked?: boolean;
  /** The OS draws the traffic lights; the web mock draws them itself. */
  nativeChrome?: boolean;
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
  onNewChat,
  chatTeamCount,
  blocked = false,
  nativeChrome = false,
}: TopbarProps): ReactNode {
  const inbox = useSidebarInbox();
  // The channel's own name, where the shell already knows it: the inbox is
  // loaded for the sidebar's badge, and the crumb reads it rather than
  // loading anything of its own.
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
  // Files, Chat and Activity are list columns: their bar is the search field
  // and the column's add control. Every other page names itself with the
  // crumbs, and a page inside an area (a team, a device) leads with Back.
  const listBar = tab === 'files' || tab === 'chat' || tab === 'activity';
  const parent =
    !listBar && tab !== 'settings' ? parentLocation(location) : null;
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
          aria-expanded="false"
          aria-label="Expand sidebar"
          title="Expand sidebar"
          disabled={blocked}
          onClick={onToggleCollapsed}
        >
          <Icon name="panelLeft" />
        </button>
      ) : null}
      {parent ? (
        <button
          type="button"
          className="tbtn back"
          aria-label="Back"
          title="Back"
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
          {segments.map((segment, index) => {
            const last = index === segments.length - 1;
            return (
              // Keyed by position as well as text: a store named after its
              // own tab ("Files › Files") would otherwise collide.
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
          shortcut={false}
          kbd
        />
      ) : (
        <span className="grow" data-tauri-drag-region="" />
      )}
      {actions}
      {actionsSlot ? <span className="bar-acts" ref={actionsSlot} /> : null}
      {tab === 'files' && onNew ? (
        <NewItemButton
          onNew={onNew}
          onNewFolder={onNewFolder}
          reason={newBlocked}
          disabled={blocked}
        />
      ) : noChatTeams ? (
        // Chat starts in a team: with none, the bar's own action is the team,
        // created on the Teams page where that sheet lives.
        <Button
          variant="quiet"
          icon="plus"
          className="new-chat"
          aria-label="New team"
          disabled={blocked}
          title="Create a team on the Teams page"
          onClick={() => onNavigate({ kind: 'teams', open: 'create' })}
        />
      ) : tab === 'chat' && onNewChat ? (
        <Button
          variant="quiet"
          icon="plus"
          className="new-chat"
          aria-label="New chat"
          title="New chat"
          disabled={blocked}
          onClick={onNewChat}
        />
      ) : null}
    </div>
  );
}
