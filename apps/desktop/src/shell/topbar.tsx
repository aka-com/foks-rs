/**
 * The topbar: the bar at the top of the content column, where the title bar
 * used to be.
 *
 * It carries the back chevron and the crumbs that say where the reader is, and
 * at its right end the three shell-wide controls — the search trigger, the rail
 * collapse toggle and the catalog refresh. Its empty parts are the window's
 * drag region, because the rail's strip alone does not reach across the window.
 *
 * The page's own header (`PageHeader`) sits directly under it and carries the
 * title, the page actions and the per-page search field.
 */

import type { ReactNode } from 'react';

import { Button, Icon } from '../components';
import { useSidebarInbox } from '../chat/inbox-provider';
import { channelTitle } from '../chat/presentation';
import { storeOf } from '../model';
import type { AgentSnapshot, DeviceLabel } from '../model';
import { parentLocation, railTabOf } from '../location';
import type { Location, RailTab } from '../location';

/** The word each tab is called, as the rail labels it. */
const TAB_LABEL: Readonly<Record<RailTab, string>> = {
  people: 'Accounts',
  chat: 'Chat',
  files: 'Files',
  teams: 'Teams',
  devices: 'Devices',
  settings: 'Settings',
};

/** The words the Settings page uses for the sections an address can name. */
const SETTINGS_SECTION_LABEL: Readonly<Record<string, string>> = {
  servers: 'Servers',
  credentials: 'Account',
  notifications: 'Notifications',
  about: 'About',
};

const DEVICES_SECTION_LABEL: Readonly<Record<string, string>> = {
  macs: 'Macs',
  keys: 'Security keys',
};

/**
 * The trail the crumbs draw: the tab, then the page under it when the address
 * names one. The tab is always first, so the crumbs answer "where am I" before
 * they answer "in what".
 */
export function crumbTrail(
  location: Location,
  snapshot?: AgentSnapshot,
  channelName?: string,
  deviceLabel?: DeviceLabel | null,
): string[] {
  const tab = railTabOf(location);
  if (!tab) return [];
  const trail = [TAB_LABEL[tab]];
  const named = (ref: string): string =>
    (snapshot && storeOf(snapshot, ref)?.name) ?? 'Unknown';
  switch (location.kind) {
    case 'all':
      trail.push('All items');
      break;
    case 'store':
      trail.push(named(location.ref));
      break;
    case 'group-settings':
      trail.push(named(location.ref));
      break;
    case 'chat':
      if (location.ref)
        trail.push(
          channelName
            ? `${named(location.ref)} ${channelName}`
            : named(location.ref),
        );
      break;
    case 'devices':
      if (location.device) {
        trail.push(
          deviceLabel &&
            deviceLabel.store === location.store &&
            deviceLabel.address === location.device
            ? deviceLabel.name
            : 'Key',
        );
      } else if (location.section)
        trail.push(DEVICES_SECTION_LABEL[location.section]);
      break;
    case 'settings':
      if (location.section)
        trail.push(SETTINGS_SECTION_LABEL[location.section]);
      if (location.section === 'servers' && location.profile)
        trail.push(
          snapshot?.servers.find((entry) => entry.id === location.profile)
            ?.name ?? location.profile,
        );
      break;
    default:
      break;
  }
  return trail;
}

export interface TopbarProps {
  /** Absent while the agent is starting; the crumbs then name the tab alone. */
  snapshot?: AgentSnapshot;
  deviceLabel?: DeviceLabel | null;
  location: Location;
  onNavigate: (location: Location) => void;
  /** Opens the search palette. Omitted where no palette is mounted. */
  onSearch?: () => void;
  collapsed: boolean;
  onToggleCollapsed?: () => void;
  refreshing?: boolean;
  onRefresh?: () => void;
  /** A blocking state: the bar is drawn, and nothing on it acts. */
  blocked?: boolean;
}

export function Topbar({
  snapshot,
  deviceLabel,
  location,
  onNavigate,
  onSearch,
  collapsed,
  onToggleCollapsed,
  refreshing = false,
  onRefresh,
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
  const trail = crumbTrail(
    location,
    snapshot,
    open ? channelTitle(open) : undefined,
    deviceLabel,
  );
  const parent = blocked ? null : parentLocation(location);
  return (
    <div className="topbar" data-tauri-drag-region="">
      <button
        type="button"
        className="back"
        title="Back"
        aria-label="Back"
        disabled={!parent}
        onClick={() => {
          if (parent) onNavigate(parent);
        }}
      >
        <Icon name="back" />
      </button>
      <div className="crumbs" data-tauri-drag-region="">
        {trail.map((crumb, index) => (
          // Keyed by position as well as text: a store named after its own tab
          // ("Files › Files") would otherwise collide.
          <span key={`${index}-${crumb}`} data-tauri-drag-region="">
            {index === 0 ? <b>{crumb}</b> : crumb}
            {index < trail.length - 1 ? <i className="sep">›</i> : null}
          </span>
        ))}
      </div>
      <span className="grow" data-tauri-drag-region="" />
      <button
        type="button"
        className="topsearch"
        disabled={!onSearch || blocked}
        title={onSearch ? 'Search everything' : 'Search is not available yet'}
        onClick={onSearch}
      >
        <Icon name="search" />
        <span className="t">Search everything</span>
        <kbd>⌘K</kbd>
      </button>
      {onToggleCollapsed ? (
        <Button
          variant="quiet"
          className="side-collapse"
          icon={collapsed ? 'panel-hollow' : 'panel-filled'}
          aria-expanded={!collapsed}
          aria-label={collapsed ? 'Expand rail' : 'Collapse rail'}
          title={collapsed ? 'Expand rail' : 'Collapse rail'}
          disabled={blocked}
          onClick={onToggleCollapsed}
        />
      ) : null}
      {onRefresh ? (
        <Button
          variant="quiet"
          className="global-refresh"
          icon="again"
          aria-label={refreshing ? 'Refreshing vaults and groups' : 'Refresh'}
          title={
            refreshing
              ? 'Refreshing vaults and groups'
              : 'Refresh vaults and groups'
          }
          disabled={refreshing || blocked}
          onClick={onRefresh}
        />
      ) : null}
    </div>
  );
}
