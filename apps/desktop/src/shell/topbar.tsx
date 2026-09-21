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

import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';

import { Button, Icon } from '../components';
import type { DesktopReconciliation } from '../desktop-reconciliation';
import { SYNC_STATUS_ID, SyncPopover, useSyncSummary } from './sync-popover';
import { useSidebarInbox } from '../chat/inbox-provider';
import { channelLabel } from '../chat/presentation';
import { serverLocalAlias, storeOf } from '../model';
import type { AgentSnapshot, DeviceLabel } from '../model';
import {
  SETTINGS_SECTION_LABEL,
  parentLocation,
  railTabOf,
  settingsSectionOf,
} from '../location';
import type { Location, RailTab } from '../location';
import { filesFolderCrumb } from '../screens/scope';

/** The word each tab is called, as the rail labels it. */
const TAB_LABEL: Readonly<Record<RailTab, string>> = {
  chat: 'Chat',
  files: 'Files',
  teams: 'Teams',
  devices: 'Devices',
  settings: 'Settings',
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
      if (section === 'servers' && location.profile)
        trail.push(
          serverLocalAlias(
            snapshot?.servers.find((entry) => entry.id === location.profile),
          ),
        );
      break;
    }
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
  /** The Files tree's selected folder (`LocationState.folder`). */
  folder?: string;
  onNavigate: (location: Location) => void;
  /** Steps the Files tree's selection back one folder. Omitted where the
   *  tree's selection is not reachable, in which case Back only ever
   *  navigates a location. */
  onSetFolder?: (folder: string) => void;
  /** Opens the search palette. Omitted where no palette is mounted. */
  onSearch?: () => void;
  collapsed: boolean;
  onToggleCollapsed?: () => void;
  refreshing?: boolean;
  onRefresh?: () => void;
  /**
   * The reconciliation service, when the shell has one. With it the refresh
   * button carries a state badge and a Refresh status popover; without it the
   * button is the plain manual refresh.
   */
  syncService?: DesktopReconciliation;
  /** Opens Settings › Servers on one server, from the popover. */
  onOpenServers?: (profile: string) => void;
  /** A blocking state: the bar is drawn, and nothing on it acts. */
  blocked?: boolean;
}

/** How long the status popover survives the pointer leaving it. */
const SYNC_HOVER_CLOSE_MS = 180;

/**
 * The refresh button with its status: one square button that refreshes on
 * click, a spinner in the icon's place while any server is refreshing and an
 * amber dot when one could not be refreshed. Pointing at the button — or
 * reaching it with the keyboard — opens the per-server popover; there is no
 * separate trigger for it. The spinner and the dot are decorative; the button
 * carries the accessible name.
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
    on.current[key] = value;
    settle();
  };
  const hideNow = (): void => {
    cancelClose();
    on.current = { pointer: false, focus: false };
    setOpen(false);
  };
  const spinning = summary.refreshing;
  const badge = summary.failed ? 'failed' : null;
  return (
    <span
      className="global-refresh-wrap"
      ref={wrapRef}
      onMouseEnter={track('pointer', true)}
      onMouseLeave={track('pointer', false)}
      onFocus={track('focus', true)}
      onBlur={track('focus', false)}
    >
      <Button
        variant="quiet"
        className="global-refresh"
        icon={spinning ? undefined : 'again'}
        busy={spinning}
        aria-label={
          spinning ? 'Refreshing vaults, teams, chat, and devices' : 'Refresh'
        }
        aria-describedby={open ? SYNC_STATUS_ID : undefined}
        disabled={refreshing || blocked}
        onClick={onRefresh}
      />
      {badge ? (
        <span className={`sync-badge ${badge}`} aria-hidden="true" />
      ) : null}
      {open ? (
        <SyncPopover
          snapshot={snapshot}
          service={service}
          summary={summary}
          refreshDisabled={refreshing || blocked}
          anchorRef={wrapRef}
          onClose={hideNow}
          onPointerEnter={track('pointer', true)}
          onPointerLeave={track('pointer', false)}
          onOpenServers={onOpenServers}
          onRefresh={blocked ? undefined : onRefresh}
        />
      ) : null}
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
  onSearch,
  collapsed,
  onToggleCollapsed,
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
  const trail = crumbTrail(
    location,
    snapshot,
    open ? channelLabel(open) : undefined,
    deviceLabel,
    folder,
  );
  const parent = blocked ? null : parentLocation(location);
  // One step back can be a narrower folder within the same page, not just a
  // different location — the tree's own selection, which `parentLocation`
  // does not see. `null` here means there is no folder narrower than the
  // location's own root, so Back falls through to `parent`.
  const folderBack = blocked
    ? null
    : filesFolderCrumb(snapshot, location, folder).back;
  const canGoBack = folderBack !== null || Boolean(parent);
  const goBack = (): void => {
    if (folderBack !== null) onSetFolder?.(folderBack);
    else if (parent) onNavigate(parent);
  };
  return (
    <div className="topbar" data-tauri-drag-region="">
      {collapsed ? (
        <button
          type="button"
          className="back"
          title="Back"
          aria-label="Back"
          disabled={!canGoBack}
          onClick={goBack}
        >
          <Icon name="back" />
        </button>
      ) : null}
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
          aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          title={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          disabled={blocked}
          onClick={onToggleCollapsed}
        />
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
        <Button
          variant="quiet"
          className="global-refresh"
          icon={refreshing ? undefined : 'again'}
          busy={refreshing}
          aria-label={
            refreshing ? 'Refreshing vaults, teams, and devices' : 'Refresh'
          }
          title={
            refreshing
              ? 'Refreshing vaults, teams, and devices'
              : 'Refresh vaults and teams'
          }
          disabled={refreshing || blocked}
          onClick={onRefresh}
        />
      ) : null}
    </div>
  );
}
