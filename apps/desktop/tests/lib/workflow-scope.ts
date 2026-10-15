import { createElement } from 'react';
import type { ReactNode } from 'react';
import type { AgentSnapshot } from '../../src/model/types';

export async function workflowScope(
  children: ReactNode,
  snapshot?: AgentSnapshot,
) {
  const { ShellSnapshotProvider } = await import('../../src/shell-services');
  const { FIXTURE } = await import('../../src/fixture');
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
  return createElement(ShellSnapshotProvider, { snapshot: ready, children });
}
