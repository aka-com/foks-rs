/**
 * The Restart agent sheet rendered from its own module under
 * features/settings: when it asks the bridge to take over the agent, and that
 * a restart made for a refused action runs that action afterwards.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';

import type {
  AgentProcessInfo,
  Bridge,
  MaintenanceSnapshot,
} from '../src/bridge';
import type { RestartRequest } from '../src/features/settings/restart-agent-sheet';
import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});

let ui: typeof import('@testing-library/react');

test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());

const OWNED: AgentProcessInfo = {
  pid: 4242,
  executable: '/Applications/FOKS.app/Contents/MacOS/foks-agent',
  startedAt: null,
  owned: true,
};

/** Renders the sheet and presses Restart agent; returns what it asked for. */
async function restart(
  request: RestartRequest,
  process: AgentProcessInfo,
): Promise<{ title: string; takeovers: boolean[]; closed: number }> {
  const { RestartAgentSheet } =
    await import('../src/features/settings/restart-agent-sheet');
  const { FIXTURE } = await import('../src/mock/fixture');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { OverlayProvider } = await import('../kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const takeovers: boolean[] = [];
  let closed = 0;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    restartAgent: async (takeover) => {
      takeovers.push(takeover);
      return {} as MaintenanceSnapshot;
    },
  };
  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(RestartAgentSheet, {
        bridge,
        request,
        process,
        processReadiness: 'ready',
        onRetryInfo: () => {},
        onClose: () => {
          closed += 1;
        },
        onError: (error) => assert.fail(String(error)),
      }),
    }),
  );
  const title = rendered.getByRole('heading').textContent ?? '';
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Restart agent' }));
  });
  return { title, takeovers, closed };
}

test('a plain restart of an agent this app owns does not take it over', async () => {
  const outcome = await restart({ purpose: 'restart' }, OWNED);
  assert.deepEqual(outcome, {
    title: 'Restart the agent?',
    takeovers: [false],
    closed: 1,
  });
});

test('a plain restart of an agent this app did not start takes it over', async () => {
  const outcome = await restart(
    { purpose: 'restart' },
    { ...OWNED, owned: false },
  );
  assert.deepEqual(outcome.takeovers, [true]);
});

test('a restart for a refused export takes the agent over, then runs the export', async () => {
  const ran: string[] = [];
  const outcome = await restart(
    {
      purpose: 'export',
      then: async () => {
        ran.push('export');
      },
    },
    { ...OWNED, owned: false },
  );
  assert.deepEqual(outcome, {
    title: 'Restart the agent to export?',
    takeovers: [true],
    closed: 1,
  });
  assert.deepEqual(ran, ['export']);
});
