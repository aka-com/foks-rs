import { useCallback, useEffect, useRef, useState } from 'react';
import { anyDialogOpen } from '/kit/overlay-primitives';
import type { ToastController } from '/kit/toasts';
import {
  sceneHref,
  sceneOf,
  type LocationStore,
  type Scene,
} from '../location';
import type { NavigationPromptVerdict } from '../navigation-guard';
import { mountSwipeBack } from '../shell/swipe-back';
import { mountBackInputs } from '../shell/back-inputs';

/** A `prompt` verdict on screen, with the promise the store is waiting on. */
interface PendingPrompt {
  verdict: NavigationPromptVerdict;
  settle: (confirmed: boolean) => void;
}

export function useShellNavigation({
  locations,
  state,
  lease,
  toasts,
  blocked = false,
}: {
  locations: LocationStore;
  state: ReturnType<LocationStore['getSnapshot']>;
  lease: Scene['lease'];
  toasts: ToastController;
  blocked?: boolean;
}) {
  // Handles navigation guard outcomes: `prompt` renders the confirmation dialog
  // below, and `refuse` displays an alert toast. The active pending navigation is
  // stored in a ref as well as in state so a newer intent can cancel it without
  // a stale closure.
  const [prompt, setPrompt] = useState<PendingPrompt | null>(null);
  const promptRef = useRef<PendingPrompt | null>(null);
  const settlePrompt = useCallback((confirmed: boolean) => {
    const open = promptRef.current;
    if (!open) return;
    promptRef.current = null;
    setPrompt(null);
    open.settle(confirmed);
  }, []);
  useEffect(() => {
    locations.setPrompter(
      (verdict) =>
        new Promise<boolean>((resolve) => {
          const open = promptRef.current;
          const next = { verdict, settle: resolve };
          promptRef.current = next;
          setPrompt(next);
          // Replace any existing confirmation prompt and resolve its promise
          // as cancelled.
          open?.settle(false);
        }),
    );
    locations.setRefusalHandler((reason) => {
      toasts.show(reason, { tone: 'warning' });
    });
    return () => {
      locations.setPrompter(null);
      locations.setRefusalHandler(null);
      // Resolve any pending navigation prompt as cancelled on unmount.
      settlePrompt(false);
    };
  }, [locations, settlePrompt, toasts]);

  // Persist the current scene in the URL so a reload restores it. Only values
  // that differ from the default are written.
  useEffect(() => {
    if (typeof window === 'undefined') return;
    const href = sceneHref(window.location.href, sceneOf(state, lease));
    if (href === window.location.href) return;
    try {
      window.history.replaceState(null, '', href);
    } catch {
      // Non-navigable environments (e.g. file:// or test harnesses) retain state in memory.
    }
  }, [state, lease]);

  // Escape clears search first, then closes the active item and its pane.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key !== 'Escape') return;
      // Ignore Escape events already handled by open modal dialogs.
      if (event.defaultPrevented) return;
      const current = locations.getSnapshot();
      if (current.query) locations.search('');
      else if (current.selection)
        locations.select(null, { closeDetails: true });
    };
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [locations]);

  const here = state.location;
  // Back follows committed navigation history, including cross-tab links.
  useEffect(() => {
    const enabled = () =>
      !blocked &&
      !anyDialogOpen() &&
      locations.getSnapshot().location.kind !== 'first-run' &&
      !document.querySelector('[role="menu"], [role="listbox"]');
    const stopSwipe = mountSwipeBack({
      target: () => locations.backTarget(),
      enabled: () => {
        const target = locations.backTarget();
        return (
          enabled() &&
          target !== null &&
          locations.navigationVerdict({
            kind: 'navigate',
            location: target,
          }) === null
        );
      },
      navigate: () => locations.back(),
    });
    const stopInputs = mountBackInputs({
      enabled,
      back: () => locations.back(),
    });
    return () => {
      stopSwipe();
      stopInputs();
    };
  }, [locations, blocked]);
  // A prompt asks whether to leave the page it was raised on. If the shell
  // left it some other way, the question no longer applies.
  useEffect(() => {
    settlePrompt(false);
  }, [here, settlePrompt]);

  return { prompt, settlePrompt };
}
