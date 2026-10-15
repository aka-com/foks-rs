import { createElement } from 'react';
import type { ReactNode } from 'react';
import type { AgentSnapshot } from '../../src/model/types';
import type { ShellServices } from '../../src/shell-services';
import { loadTestShell } from './test-shell';

/**
 * Mounts `children` under `TestShell` with `snapshot`, or with the fixture
 * whose servers are all verified and available, and with the given services.
 */
export async function workflowScope(
  children: ReactNode,
  snapshot?: AgentSnapshot,
  services: Partial<ShellServices> = {},
) {
  const TestShell = await loadTestShell();
  const { FIXTURE } = await import('../../src/mock/fixture');
  const ready: AgentSnapshot = snapshot ?? {
    ...FIXTURE,
    servers: ['personal', 'acme', 'local', 'host'].map((profileName) => ({
      ...FIXTURE.servers[0],
      profileName,
      configuredEndpoint: profileName,
      trust: { status: 'verified' },
      compatibility: { status: 'not-required' },
      passiveStatus: { status: 'available', source: 'signed-server-status' },
      connectivity: { status: 'unknown' },
      restrictions: [],
    })),
    observedExpiredLeases: [],
  };
  return createElement(TestShell, { ...services, snapshot: ready, children });
}
