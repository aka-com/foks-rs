/**
 * The services the vault shell provides to every screen and sheet under it:
 * the bridge, the shell's lease clock, the error handlers, the catalog
 * refreshes, and the snapshot the shell shows.
 *
 * Components read them here instead of taking them as props from each level
 * above. Purely presentational components stay prop-driven.
 */

import { createContext, useContext } from 'react';
import type { ReactNode } from 'react';
import type { ToastOptions } from '../kit/toasts';
import type { CommandErrorHandler } from './app/catalog-runtime';
import type { Bridge } from './bridge';
import type { AgentSnapshot } from './model/types';
import type { MutationFailureHandler } from './data/mutation-recovery';

/**
 * Every member keeps its identity while the shell is mounted, unless the
 * bridge or the catalog runtime is replaced, so a component may list one in
 * an effect's dependencies without the effect running on every refresh.
 */
export interface ShellServices {
  bridge: Bridge;
  /** The shell's lease clock, in seconds. */
  accessNow: () => number;
  /**
   * Reports a command failure. It calls the shell's current handler, so its
   * own identity does not change when that handler's dependencies do.
   */
  commandError: CommandErrorHandler;
  /** Reports a write failure and reconciles the catalog after it. */
  mutationError: MutationFailureHandler;
  /**
   * Reloads the catalog after a write, of `profile` when given, then shows
   * `message`; a failed reload is reported as a stale view.
   */
  refresh: (
    message: string,
    profile?: string,
    options?: ToastOptions,
  ) => Promise<void>;
  /** Reloads every profile's catalog and resolves with the new snapshot. */
  refreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
  /**
   * Reloads the catalog of `profile`, or of every profile without one, and
   * resolves with the snapshot that includes it.
   */
  reloadCatalog: (profile?: string) => Promise<AgentSnapshot>;
}

const ShellServicesContext = createContext<ShellServices | null>(null);

/**
 * The shell's snapshot, after its lease and fixture overrides: the same value
 * the shell passes to the screens it renders. A context rather than an
 * external store, so a component that reads it and a parent that received it
 * as a prop see the same snapshot in the same commit.
 */
export const ShellSnapshotContext = createContext<AgentSnapshot | undefined>(
  undefined,
);

export function ShellServicesProvider({
  services,
  children,
}: {
  services: ShellServices;
  children: ReactNode;
}): ReactNode {
  return (
    <ShellServicesContext.Provider value={services}>
      {children}
    </ShellServicesContext.Provider>
  );
}

export function ShellSnapshotProvider({
  snapshot,
  children,
}: {
  snapshot: AgentSnapshot;
  children: ReactNode;
}): ReactNode {
  return (
    <ShellSnapshotContext.Provider value={snapshot}>
      {children}
    </ShellSnapshotContext.Provider>
  );
}

/** The shell's services. Throws outside a `ShellServicesProvider`. */
export function useShellServices(): ShellServices {
  const services = useContext(ShellServicesContext);
  if (!services)
    throw new Error('useShellServices is used outside ShellServicesProvider');
  return services;
}

/** The snapshot the shell shows. Throws outside a `ShellSnapshotProvider`. */
export function useShellSnapshot(): AgentSnapshot {
  const snapshot = useContext(ShellSnapshotContext);
  if (!snapshot)
    throw new Error('useShellSnapshot is used outside ShellSnapshotProvider');
  return snapshot;
}
