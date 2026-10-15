import { createElement, useState } from 'react';
import type { ComponentType, ReactNode } from 'react';
import type { AgentSnapshot } from '../../src/model/types';
import type { ShellServices } from '../../src/shell-services';

export interface TestShellProps extends Partial<ShellServices> {
  /** The shell's snapshot; the fixture when omitted. */
  snapshot?: AgentSnapshot;
  children: ReactNode;
}

/**
 * Loads `TestShell`, which wraps its children in the providers `VaultShell`
 * mounts for its screens and sheets: the shell services and the shown
 * snapshot. Each service a test does not name is the mock bridge over the
 * snapshot, a clock in seconds, an error handler that does nothing, or a
 * catalog reload that resolves with the snapshot.
 *
 * The modules load on call, so a render test can import this file before
 * `installDom`.
 */
export async function loadTestShell(): Promise<ComponentType<TestShellProps>> {
  const { ShellServicesProvider, ShellSnapshotProvider } =
    await import('../../src/shell-services');
  const { FIXTURE } = await import('../../src/fixture');
  const { mockBridge } = await import('../../src/mock-bridge');
  const accessNow = () => Date.now() / 1000;
  const commandError = () => undefined;
  const mutationError = async () => undefined;
  const refresh = async () => undefined;
  return function TestShell({
    snapshot = FIXTURE,
    bridge,
    children,
    ...services
  }: TestShellProps): ReactNode {
    const [fallback] = useState(() => mockBridge(snapshot));
    return createElement(ShellServicesProvider, {
      services: {
        accessNow,
        commandError,
        mutationError,
        refresh,
        refreshSnapshot: async () => snapshot,
        reloadCatalog: async () => snapshot,
        ...services,
        bridge: bridge ?? fallback,
      },
      children: createElement(ShellSnapshotProvider, { snapshot, children }),
    });
  };
}
