import { useLayoutEffect, useMemo } from 'react';

/**
 * Timestamp until which channel navigation preserves current focus so keyboard
 * navigation in the channel list keeps focus on the active row.
 */
let keepFocusUntil = 0;

/** Suppresses composer autofocus for the next channel navigation within 500ms. */
export function keepFocusOnNextChannelVisit(): void {
  keepFocusUntil = Date.now() + 500;
}

/** One focus opportunity per visit, including time spent discovering the channel. */
export function useComposerAutofocus(channel: string | undefined) {
  const visit = useMemo(
    () => ({ channel, pending: Date.now() > keepFocusUntil, active: false }),
    [channel],
  );
  useLayoutEffect(() => {
    visit.active = true;
    const cancel = () => {
      visit.pending = false;
    };
    // Listen above the thread: it may not exist yet, or may remount as its
    // readable status changes. Navigation's initiating event has already run.
    document.addEventListener('pointerdown', cancel, true);
    document.addEventListener('keydown', cancel, true);
    document.addEventListener('focusin', cancel, true);
    return () => {
      visit.active = false;
      document.removeEventListener('pointerdown', cancel, true);
      document.removeEventListener('keydown', cancel, true);
      document.removeEventListener('focusin', cancel, true);
    };
  }, [visit]);
  return useMemo(
    () => (element: HTMLTextAreaElement) => {
      if (!visit.active || !visit.pending || !element.isConnected) return;
      visit.pending = false;
      // A restored sheet or a dialog may already own focus before this visit.
      if (document.querySelector('[aria-modal="true"]')) return;
      element.focus({ preventScroll: true });
    },
    [visit],
  );
}
