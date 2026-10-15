import { useCallback, useMemo } from 'react';
import type { Bridge } from '../bridge';
import type { ShellServices } from '../shell-services';
import {
  useMutationError,
  type CatalogRuntime,
  type CommandErrorHandler,
} from './catalog-runtime';

/**
 * Builds the services `VaultShell` provides to its screens and sheets.
 *
 * `commandError` calls `commandErrorRef.current`, which the shell runtime
 * assigns on every render, so the reported handler is always the current one
 * while the function itself keeps one identity. `mutationError` is built on
 * it and keeps its identity for the same reason. `VaultShell` passes the same
 * two handlers to its own panels and runtimes.
 */
export function useShellServicesRuntime({
  bridge,
  accessNow,
  catalog,
}: {
  bridge: Bridge;
  accessNow: () => number;
  catalog: Pick<
    CatalogRuntime,
    | 'commandErrorRef'
    | 'latestRef'
    | 'profileRefresh'
    | 'refresh'
    | 'refreshSnapshot'
    | 'reloadCatalog'
  >;
}): ShellServices {
  const { commandErrorRef, refresh, refreshSnapshot, reloadCatalog } = catalog;
  const commandError = useCallback<CommandErrorHandler>(
    (error, item, draft) => commandErrorRef.current(error, item, draft),
    [commandErrorRef],
  );
  const mutationError = useMutationError(commandError, catalog);
  return useMemo(
    () => ({
      bridge,
      accessNow,
      commandError,
      mutationError,
      refresh,
      refreshSnapshot,
      reloadCatalog,
    }),
    [
      bridge,
      accessNow,
      commandError,
      mutationError,
      refresh,
      refreshSnapshot,
      reloadCatalog,
    ],
  );
}
