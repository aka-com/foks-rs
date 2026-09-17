import assert from 'node:assert/strict';
import test from 'node:test';

import {
  AgentLifecycleController,
  maintenanceOutcomeMessage,
} from '../src/agent-lifecycle';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';
import type { AgentStatus } from '../src/model';

test('maintenance operation outcomes remain distinct from service restoration', () => {
  assert.equal(
    maintenanceOutcomeMessage({ status: 'completed' }),
    'State maintenance completed.',
  );
  assert.equal(
    maintenanceOutcomeMessage({ status: 'cancelled' }),
    'State maintenance was cancelled.',
  );
  assert.equal(
    maintenanceOutcomeMessage({
      status: 'failed',
      error: {
        code: 'archive-write',
        message: 'Destination full.',
        retryable: false,
        ambiguous: false,
        fatal: false,
      },
    }),
    'State maintenance failed: Destination full.',
  );
});

test('reconnect consumes bootstrap status and initializes before ready', async () => {
  const calls: string[] = [];
  const bridge = {
    ...mockBridge(FIXTURE),
    retryAgentConnection: async (): Promise<AgentStatus> => {
      calls.push('retry');
      return { state: 'bootstrap', step: 'descriptive wording' };
    },
    initializeClientState: async (): Promise<AgentStatus> => {
      calls.push('initialize');
      return { state: 'ready' };
    },
  };
  const controller = new AgentLifecycleController(bridge);
  const observed: string[] = [];
  controller.subscribe((state) => observed.push(state.state));
  assert.deepEqual(await controller.establish(true), { state: 'ready' });
  assert.deepEqual(calls, ['retry', 'initialize']);
  assert.deepEqual(observed, ['checking', 'checking', 'initializing', 'ready']);
});

test('concurrent readiness requests share one initialization', async () => {
  let initializeCalls = 0;
  let release!: () => void;
  const blocked = new Promise<void>((resolve) => {
    release = resolve;
  });
  const bridge = {
    ...mockBridge(FIXTURE),
    agentStatus: async (): Promise<AgentStatus> => ({
      state: 'bootstrap',
      step: 'initialize-state',
    }),
    initializeClientState: async (): Promise<AgentStatus> => {
      initializeCalls++;
      await blocked;
      return { state: 'ready' };
    },
  };
  const controller = new AgentLifecycleController(bridge);
  const first = controller.establish();
  const second = controller.establish();
  assert.equal(first, second);
  release();
  await first;
  assert.equal(initializeCalls, 1);
});

test('late status from a disconnected generation cannot restore ready', async () => {
  let release!: (status: AgentStatus) => void;
  const late = new Promise<AgentStatus>((resolve) => {
    release = resolve;
  });
  const bridge = {
    ...mockBridge(FIXTURE),
    agentStatus: () => late,
  };
  const controller = new AgentLifecycleController(bridge);
  const first = controller.establish();
  controller.disconnect('socket closed');
  release({ state: 'ready' });
  await assert.rejects(first);
  assert.equal(controller.snapshot().state, 'disconnected');
});

test('queued recovery is generation-bound across multiple invalidations', async () => {
  let release!: (status: AgentStatus) => void;
  const late = new Promise<AgentStatus>((resolve) => {
    release = resolve;
  });
  let statusCalls = 0;
  const bridge = {
    ...mockBridge(FIXTURE),
    agentStatus: async (): Promise<AgentStatus> => {
      statusCalls++;
      return statusCalls === 1 ? late : { state: 'ready' };
    },
  };
  const controller = new AgentLifecycleController(bridge);
  const original = controller.establish();
  controller.requireBootstrap('initialize-state');
  const obsoleteRecovery = controller.establish();
  controller.disconnect('socket closed again');
  release({ state: 'ready' });
  const obsolete = await Promise.allSettled([original, obsoleteRecovery]);
  assert.deepEqual(
    obsolete.map((result) => result.status),
    ['rejected', 'rejected'],
  );
  assert.equal(controller.snapshot().state, 'disconnected');
  assert.equal(statusCalls, 1);

  const latest = controller.establish();
  assert.equal(controller.establish(), latest);
  assert.deepEqual(await latest, { state: 'ready' });
  assert.equal(statusCalls, 2);
  assert.equal(controller.snapshot().state, 'ready');
});

test('maintenance generations suppress stale events and distinguish terminal dispositions', () => {
  const controller = new AgentLifecycleController(mockBridge(FIXTURE), {
    state: 'ready',
  });
  controller.applyMaintenance({
    state: 'active',
    generation: 4,
    revision: 3,
    kind: 'verify',
    phase: 'running',
  });
  assert.deepEqual(controller.snapshot(), {
    state: 'maintenance',
    generation: 4,
    kind: 'verify',
    phase: 'running',
  });
  controller.applyMaintenance({
    state: 'complete',
    generation: 3,
    revision: 2,
    kind: 'export',
    operation: { status: 'completed' },
    disposition: { status: 'continue-current-root' },
  });
  assert.equal(controller.snapshot().state, 'maintenance');
  controller.applyMaintenance({
    state: 'complete',
    generation: 4,
    revision: 4,
    kind: 'verify',
    operation: {
      status: 'failed',
      error: {
        code: 'verification-failed',
        message: 'wording is not a state transition',
        retryable: false,
        ambiguous: false,
        fatal: false,
      },
    },
    disposition: { status: 'recovery-required', root: '/tmp/imported' },
  });
  assert.equal(
    controller.applyMaintenance({
      state: 'active',
      generation: 4,
      revision: 3,
      kind: 'verify',
      phase: 'restoring',
    }),
    false,
  );
  assert.deepEqual(controller.snapshot(), {
    state: 'recovery-required',
    generation: 4,
    operation: {
      status: 'failed',
      error: {
        code: 'verification-failed',
        message: 'wording is not a state transition',
        retryable: false,
        ambiguous: false,
        fatal: false,
      },
    },
    root: '/tmp/imported',
  });
});

test('operation failure and restoration failure remain independently visible', () => {
  const operationError = {
    code: 'export-failed',
    message: 'archive write failed',
    retryable: false,
    ambiguous: false,
    fatal: false,
  };
  const restorationError = {
    code: 'agent-start-failed',
    message: 'service did not start',
    retryable: true,
    ambiguous: false,
    fatal: false,
  };
  const controller = new AgentLifecycleController(mockBridge(FIXTURE), {
    state: 'ready',
  });
  controller.applyMaintenance({
    state: 'complete',
    generation: 9,
    revision: 20,
    kind: 'export',
    operation: { status: 'failed', error: operationError },
    disposition: {
      status: 'restoration-failed',
      root: '/tmp/current',
      error: restorationError,
    },
  });
  assert.deepEqual(controller.snapshot(), {
    state: 'restoration-failed',
    generation: 9,
    operation: { status: 'failed', error: operationError },
    error: restorationError,
  });
  assert.equal(
    controller.applyMaintenance({
      state: 'complete',
      generation: 9,
      revision: 21,
      kind: 'export',
      operation: { status: 'failed', error: operationError },
      disposition: { status: 'continue-current-root' },
    }),
    true,
  );
  assert.equal(controller.snapshot().state, 'checking');
});

test('cancelled maintenance reconciles status instead of manufacturing ready', async () => {
  let statusCalls = 0;
  const bridge = {
    ...mockBridge(FIXTURE),
    agentStatus: async (): Promise<AgentStatus> => {
      statusCalls++;
      return { state: 'ready' };
    },
  };
  const controller = new AgentLifecycleController(bridge, {
    state: 'bootstrap',
    step: 'create-state',
  });
  controller.applyMaintenance({
    state: 'complete',
    generation: 1,
    revision: 2,
    kind: 'export',
    operation: { status: 'cancelled' },
    disposition: { status: 'continue-current-root' },
  });
  assert.equal(controller.snapshot().state, 'checking');
  await controller.establish();
  assert.equal(statusCalls, 1);
  assert.equal(controller.snapshot().state, 'ready');
});
