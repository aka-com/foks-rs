import { createElement } from 'react';
import type { ReactNode } from 'react';
import type { AgentSnapshot } from '../../src/model/types';
import { loadTestShell } from './test-shell';

export async function workflowScope(
  children: ReactNode,
  snapshot?: AgentSnapshot,
) {
  const TestShell = await loadTestShell();
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
  return createElement(TestShell, { snapshot: ready, children });
}
