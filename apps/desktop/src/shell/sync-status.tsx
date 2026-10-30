/**
 * Sidebar footer sync status control: a button that triggers a refresh on
 * click and opens the per-server Refresh status popover on hover or focus.
 *
 * When all servers are current, it displays a green dot and "Connected"
 * (design A-10). Otherwise, it displays the active status label: Syncing…,
 * Check-in expired, Refresh failed, or Disconnected. The popover shows
 * per-server sync details. When the shell is blocked (agent disconnected or
 * busy), the button displays Disconnected.
 */

import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';

import { Icon } from '../components';
import type { DesktopReconciliation } from '../desktop-reconciliation';
import { serverAvailability } from '../model/lease';
import type { AgentSnapshot } from '../model';
import { SYNC_STATUS_ID, SyncPopover, useSyncSummary } from './sync-popover';

/** Delay in milliseconds before closing the status popover after the pointer leaves. */
const SYNC_HOVER_CLOSE_MS = 180;

/** Maps a sync state to its CSS class, icon, and display label. */
export function syncFace(
  state: 'synced' | 'syncing' | 'lapsed' | 'failed' | 'lost',
): {
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
      return { className: 'sync', icon: 'refresh', label: 'Connected' };
  }
}

/**
 * Sync status control: a button that triggers a manual refresh on click, shows
 * a spinning icon while any server is refreshing, and displays failure states
 * as text. Hovering or focusing the button opens the per-server status popover.
 *
 * Because the popover is portaled outside the button wrapper, pointer and focus
 * state are tracked across both elements, and closing is delayed by
 * `SYNC_HOVER_CLOSE_MS` so the pointer can move between the button and popover.
 */
export function SyncControls({
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
  // Tracks whether pointer or keyboard focus is inside the button or popover.
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
  /** Opens the popover immediately or schedules it to close based on current hover/focus flags. */
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
  // Suppresses focus tracking during programmatic focus restoration so focusing
  // the trigger button does not immediately reopen the closed popover.
  const movingFocus = useRef(false);
  const hideNow = (): void => {
    cancelClose();
    on.current = { pointer: false, focus: false };
    setOpen(false);
    // Restore focus to the trigger button when Escape closes the popover from
    // inside it (since `anchorRef` points to the non-focusable `<span>` wrapper).
    const active = document.activeElement;
    if (active instanceof HTMLElement && !wrapRef.current?.contains(active)) {
      movingFocus.current = true;
      buttonRef.current?.focus();
      movingFocus.current = false;
    }
  };
  // ArrowDown on the trigger button opens the popover and focuses its first
  // interactive control (or focuses it immediately if already open).
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
  // Treat both background scheduler refreshes and manual shell refreshes as a
  // single spinning state.
  const spinning = summary.refreshing || refreshing;
  // Prioritize `blocked` (agent disconnected) over server lease or refresh states.
  // Treat both expired and unavailable leases as a lapsed check-in.
  const lapsed = snapshot.servers.some((server) => {
    const availability = serverAvailability(snapshot, server);
    return (
      !availability.available &&
      (availability.reason === 'check-in-expired' ||
        availability.reason === 'check-in-unavailable')
    );
  });
  // Prioritize a refresh failure on any server over a lapsed check-in on
  // another, since clicking the control retries failed refreshes; the popover
  // lists each server's individual state.
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
        // Include both the current status label and the refresh action in `aria-label`.
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
        {face.label === 'Connected' ? (
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
 * Manual refresh status button used when no reconciliation service is active
 * (e.g. in the web mock).
 */
export function PlainSyncStatus({
  refreshing,
  blocked,
  onRefresh,
}: {
  refreshing: boolean;
  blocked: boolean;
  onRefresh: () => void;
}): ReactNode {
  const face = syncFace(blocked ? 'lost' : refreshing ? 'syncing' : 'synced');
  return (
    <button
      type="button"
      className={`global-refresh ${face.className}`}
      aria-label={`${face.label} — ${
        refreshing
          ? 'refreshing vaults, teams and devices'
          : 'refresh vaults, teams and devices'
      }`}
      title={
        refreshing
          ? 'Refreshing vaults, teams, and devices'
          : 'Refresh vaults and teams'
      }
      // Busy, not disabled: a disabled button drops the keyboard focus.
      aria-busy={refreshing || undefined}
      aria-disabled={refreshing || undefined}
      disabled={blocked}
      onClick={refreshing ? undefined : onRefresh}
    >
      {face.label === 'Connected' ? (
        <span className="dot ok" aria-hidden="true" />
      ) : face.icon === 'refresh' ? null : (
        <Icon name={face.icon} />
      )}
      <span className="t">{face.label}</span>
    </button>
  );
}
