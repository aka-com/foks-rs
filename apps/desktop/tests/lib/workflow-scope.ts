import { createElement } from 'react';
import type { ReactNode } from 'react';
import type { ViteDevServer } from 'vite';
import type { AgentSnapshot } from '../../src/model/types';

export async function workflowScope(
  vite: ViteDevServer,
  children: ReactNode,
  snapshot?: AgentSnapshot,
) {
  const { WorkflowProvider } = (await vite.ssrLoadModule(
    '/src/workflow-context.tsx',
  )) as typeof import('../../src/workflow-context');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../../src/fixture');
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
  return createElement(WorkflowProvider, { snapshot: ready, children });
}
