/**
 * The sync status line at the foot of the sidebar: one worded button that
 * refreshes on click, with the per-server Refresh status popover it opens on
 * hover or focus.
 *
 * In the normal state it reads as the agent light, "Connected" with a green
 * dot (design A-10), and says the state in words otherwise: Syncing…,
 * Check-in expired, Refresh failed or Disconnected, since each of those asks
 * for something. The popover holds the per-server sync detail. A blocked
 * shell, where the agent is gone or busy, reads Disconnected.
 */

import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';

import { Icon } from '../components';
import type { DesktopReconciliation } from '../desktop-reconciliation';
import { serverAvailability } from '../model/lease';
import type { AgentSnapshot } from '../model';
import { SYNC_STATUS_ID, SyncPopover, useSyncSummary } from './sync-popover';

/** How long the status popover survives the pointer leaving it. */
const SYNC_HOVER_CLOSE_MS = 180;

/** The sync control's four states, as a class, an icon and a word. */
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
 * The status line without a reconciliation service: the plain manual
 * refresh, worded the same way.
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
