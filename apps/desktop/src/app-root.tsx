/**
 * Root application component for the FOKS desktop vault shell.
 *
 * Integrates sidebar navigation, item screens, details panels, and modal
 * workflows with bridge IPC and URL location state.
 */

import type { ReactNode } from 'react';
import type { Bridge } from './bridge';
import type { LocationStore } from './location';
import type { AgentSnapshot } from './model';
import type { LeaseExpiryClock } from './scheduling/lease-expiry';
import { useAppBootstrap } from './app/app-bootstrap';
import { shellBlock } from './app/blocking-shell';
import { BootstrapScreen } from './app/bootstrap-screen';
import { VaultShell } from './app/vault-shell';

export interface AppProps {
  /** Supplying a snapshot makes render tests synchronous. Production omits it. */
  snapshot?: AgentSnapshot;
  /** A command seam for tests; production selects the Tauri or mock bridge. */
  bridge?: Bridge;
  /** Injected by the render tests so navigation is observable. */
  store?: LocationStore;
  /** Controlled wall clock for expiry lifecycle tests. */
  leaseClock?: LeaseExpiryClock;
  /** Shortens the first-paint deadline so boot tests do not wait it out. */
  firstPaintDeadlineMs?: number;
}

export function App({
  snapshot: agentSnapshot,
  bridge,
  store,
  leaseClock,
  firstPaintDeadlineMs,
}: AppProps): ReactNode {
  const boot = useAppBootstrap(agentSnapshot, bridge, firstPaintDeadlineMs);
  const { loaded, activeBridge, agentController } = boot;
  const block = shellBlock(boot.agentLifecycle, {
    locked: Boolean(boot.lockState && activeBridge),
    error: boot.loadError,
    pending: !loaded || !activeBridge || !agentController,
    progress: boot.bootProgress,
  });
  if (block || !loaded || !activeBridge || !agentController)
    return (
      <BootstrapScreen boot={boot} block={block ?? { kind: 'starting' }} />
    );
  return (
    <VaultShell
      snapshot={loaded}
      bridge={activeBridge}
      store={store}
      firstRunStart={boot.firstRunStart}
      managedProfile={boot.managedProfile}
      onLock={boot.lockNow}
      retireBoot={boot.retireBoot}
      currentBootSnapshot={boot.currentBootSnapshot}
      agentController={agentController}
      maintenanceOwnership={boot.maintenanceOwnership}
      leaseClock={leaseClock}
    />
  );
}
