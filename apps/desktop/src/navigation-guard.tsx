/**
 * React integration for `LocationStore` navigation guards.
 *
 * This module provides guard registration context, screen hooks, and the
 * confirmation dialog for `prompt` verdicts. Guards only evaluate navigation;
 * they do not perform navigation or modify application state.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
} from 'react';
import type { ReactNode } from 'react';

import { Button, Icon, SheetDialog } from './components';
import type { GuardVerdict, LocationStore, NavigationGuard } from './location';

/** Prompt verdict displayed by `NavigationPrompt`. */
export type NavigationPromptVerdict = Extract<
  GuardVerdict,
  { verdict: 'prompt' }
>;

export interface NavigationGuardApi {
  /** Registers a guard and returns its cleanup function. */
  registerGuard: (guard: NavigationGuard) => () => void;
}

/**
 * Provides optional navigation-guard registration. Outside the provider,
 * `useNavigationGuard` is a no-op so isolated screens and pre-shell renders
 * do not throw.
 */
export const NavigationGuardContext = createContext<NavigationGuardApi | null>(
  null,
);

/** Publishes `store`'s registration seam to the screens under it. */
export function NavigationGuardProvider({
  store,
  children,
}: {
  store: LocationStore;
  children: ReactNode;
}): ReactNode {
  const api = useMemo<NavigationGuardApi>(
    () => ({ registerGuard: (guard) => store.registerGuard(guard) }),
    [store],
  );
  return (
    <NavigationGuardContext.Provider value={api}>
      {children}
    </NavigationGuardContext.Provider>
  );
}

/**
 * Registers `guard` while the component is mounted.
 *
 * Guard identity changes cause re-registration. Callers can keep a stable
 * callback and read current state from a ref, or pass `null` when no guard is
 * required. Guards run in registration order and the first non-null verdict
 * determines the result.
 *
 * `deps` adds values captured by the guard to the registration effect.
 */
export function useNavigationGuard(
  guard: NavigationGuard | null,
  deps: readonly unknown[] = [],
): void {
  const api = useContext(NavigationGuardContext);
  useEffect(() => {
    if (!api || !guard) return;
    return api.registerGuard(guard);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, guard, ...deps]);
}

/**
 * Registers a stable guard that returns the current verdict without
 * re-registering after each state change. Pass `null` when navigation does
 * not require protection.
 */
export function useSheetGuard(verdict: GuardVerdict): void {
  const answer = useRef(verdict);
  answer.current = verdict;
  const guard = useCallback<NavigationGuard>(() => answer.current, []);
  useNavigationGuard(guard);
}

/**
 * Displays the confirmation dialog for a `prompt` verdict. The confirm action
 * uses the verdict's label; Cancel, Escape, and backdrop dismissal preserve
 * the current location. Initial focus is placed on Cancel.
 */
export function NavigationPrompt({
  verdict,
  onConfirm,
  onCancel,
}: {
  verdict: NavigationPromptVerdict;
  onConfirm: () => void;
  onCancel: () => void;
}): ReactNode {
  return (
    <SheetDialog
      danger
      glyph={
        <span className="kico md danger">
          <Icon name="alert" />
        </span>
      }
      title={verdict.title}
      onClose={onCancel}
      footer={
        <>
          <Button data-dialog-autofocus="true" onClick={onCancel}>
            Cancel
          </Button>
          <Button variant="primary" danger onClick={onConfirm}>
            {verdict.confirm}
          </Button>
        </>
      }
    >
      <p>{verdict.body}</p>
    </SheetDialog>
  );
}
