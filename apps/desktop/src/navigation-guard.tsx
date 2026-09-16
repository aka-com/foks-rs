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
  useState,
} from 'react';
import type { Dispatch, ReactNode, SetStateAction } from 'react';

import { Button, Icon, SheetDialog } from './components';
import type { GuardVerdict, LocationStore, NavigationGuard } from './location';

/** Prompt verdict displayed by `NavigationPrompt`. */
export type NavigationPromptVerdict = Extract<
  GuardVerdict,
  { verdict: 'prompt' }
>;

export interface NavigationGuardApi {
  store?: LocationStore;
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
    () => ({ store, registerGuard: (guard) => store.registerGuard(guard) }),
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
  restoreOnTab?: boolean,
): void {
  const api = useContext(NavigationGuardContext);
  const restorable = useRef(restoreOnTab);
  restorable.current = restoreOnTab;
  const restorationId = useRef(Symbol('sheet-restoration'));
  useEffect(() => {
    const id = restorationId.current;
    api?.store?.setSheetRestorable(id, restoreOnTab);
    return () => api?.store?.setSheetRestorable(id, undefined);
  }, [api, restoreOnTab]);
  useEffect(() => {
    if (!api || !guard) return;
    return api.registerGuard((intent) => {
      const verdict = guard(intent);
      return intent.kind === 'navigate' &&
        intent.tab &&
        restorable.current &&
        verdict?.verdict !== 'refuse'
        ? null
        : verdict;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, guard, ...deps]);
}

/**
 * Registers a stable guard that returns the current verdict without
 * re-registering after each state change. Pass `null` when navigation does
 * not require protection.
 */
export function useSheetGuard(
  verdict: GuardVerdict,
  restoreOnTab?: boolean,
): void {
  const answer = useRef(verdict);
  answer.current = verdict;
  const guard = useCallback<NavigationGuard>(() => answer.current, []);
  useNavigationGuard(guard, [], restoreOnTab);
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

/** State explicitly opted into tab suspension by the owning screen. */
export function useTabSheetState<T>(
  key: string,
  initial: T | (() => T),
  persist: boolean | ((value: T) => boolean) = true,
  suppliedStore?: LocationStore,
): [T, Dispatch<SetStateAction<T>>] {
  const api = useContext(NavigationGuardContext);
  const store = suppliedStore ?? api?.store;
  const owner = useRef(store?.getSnapshot().location);
  const initialRef = useRef(initial);
  const defaultValue = (): T =>
    typeof initialRef.current === 'function'
      ? (initialRef.current as () => T)()
      : initialRef.current;
  const read = (): T => {
    const saved = store?.getSnapshot().sheet;
    return saved && key in saved ? (saved[key] as T) : defaultValue();
  };
  const [value, setValue] = useState<T>(read);
  const wasPersisted = useRef(false);
  const currentLocation = store?.getSnapshot().location;
  const renderedOwner = owner.current;
  const persistence = useRef(persist);
  persistence.current = persist;
  useEffect(() => {
    if (currentLocation === owner.current) return;
    owner.current = currentLocation;
    wasPersisted.current = false;
    setValue(read());
    // The new location's saved draft, not the old screen's current state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentLocation]);
  useEffect(() => {
    if (!store || store.getSnapshot().location !== renderedOwner) return;
    const accept = persistence.current;
    if (typeof accept === 'function') {
      if (!accept(value)) {
        if (wasPersisted.current) store.clearSheet();
        wasPersisted.current = false;
        return;
      }
    } else if (!accept) return;
    wasPersisted.current = true;
    store.setSheetField(key, value);
  }, [key, renderedOwner, store, value]);
  return [value, setValue];
}
