import assert from 'node:assert/strict';
import test from 'node:test';

import {
  AgentLifecycleController,
  maintenanceOutcomeMessage,
} from '../src/agent-lifecycle';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';
import type { AgentStatus } from '../src/model';

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (error: unknown) => void;
} {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((accept, decline) => {
    resolve = accept;
    reject = decline;
  });
  return { promise, resolve, reject };
}

test('automatic establishment returns bootstrap without initialization or manual reconnect', async () => {
  const calls: string[] = [];
  const controller = new AgentLifecycleController(
    {
      ...mockBridge(FIXTURE),
      retryAgentConnection: async () => {
        calls.push('manual');
        return { state: 'ready' };
      },
      initializeClientState: async () => {
        calls.push('initialize');
        return { state: 'ready' };
      },
    },
    { state: 'ready' },
    async () => {
      calls.push('automatic');
      return { state: 'bootstrap', step: 'initialize-state' };
    },
  );
  controller.disconnect('socket closed');
  assert.deepEqual(await controller.establishAutomatic(), {
    state: 'bootstrap',
    step: 'initialize-state',
  });
  assert.deepEqual(calls, ['automatic']);
  assert.deepEqual(controller.snapshot(), {
    state: 'bootstrap',
    step: 'initialize-state',
  });
});

test('automatic establishment defaults to a status probe, not manual retry', async () => {
  let probes = 0;
  let retries = 0;
  const controller = new AgentLifecycleController({
    ...mockBridge(FIXTURE),
    agentStatus: async () => {
      probes++;
      return { state: 'ready' };
    },
    retryAgentConnection: async () => {
      retries++;
      return { state: 'ready' };
    },
  });
  await controller.establishAutomatic();
  assert.equal(probes, 1);
  assert.equal(retries, 0);
});

test('automatic and manual establishment share an active native request', async () => {
  const automatic = deferred<AgentStatus>();
  const controller = new AgentLifecycleController(
    mockBridge(FIXTURE),
    undefined,
    () => automatic.promise,
  );
  const pending = controller.establishAutomatic();
  assert.equal(controller.establish(true), pending);
  automatic.resolve({ state: 'ready' });
  assert.deepEqual(await pending, { state: 'ready' });
});

test('automatic establishment cannot bypass bootstrap or maintenance', async () => {
  let calls = 0;
  const controller = new AgentLifecycleController(
    mockBridge(FIXTURE),
    { state: 'bootstrap', step: 'create-state' },
    async () => {
      calls++;
      return { state: 'ready' };
    },
  );
  await assert.rejects(controller.establishAutomatic());
  assert.equal(controller.snapshot().state, 'bootstrap');
  controller.applyMaintenance({
    state: 'active',
    generation: 1,
    revision: 1,
    kind: 'verify',
    phase: 'running',
  });
  await assert.rejects(controller.establishAutomatic());
  assert.equal(controller.snapshot().state, 'maintenance');
  assert.equal(calls, 0);
});

test('automatic request validity suppresses late failure publication', async () => {
  const automatic = deferred<AgentStatus>();
  let current = true;
  const controller = new AgentLifecycleController(
    mockBridge(FIXTURE),
    { state: 'ready' },
    () => automatic.promise,
  );
  controller.disconnect('socket closed');
  const pending = controller.establishAutomatic(() => current);
  current = false;
  automatic.reject(new Error('late transport error'));
  await assert.rejects(pending);
  assert.deepEqual(controller.snapshot(), {
    state: 'disconnected',
    error: 'socket closed',
  });
});

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

test('duplicate disconnect does not stale a pending reconnect', async () => {
  const retry = deferred<AgentStatus>();
  const controller = new AgentLifecycleController({
    ...mockBridge(FIXTURE),
    retryAgentConnection: () => retry.promise,
  });
  assert.equal(controller.disconnect('socket closed'), true);
  const pending = controller.establish(true);
  assert.equal(controller.disconnect('duplicate socket event'), false);
  retry.resolve({ state: 'ready' });
  assert.deepEqual(await pending, { state: 'ready' });
  assert.equal(controller.snapshot().state, 'ready');
});

test('reconnect remains disconnected through retry and initialization', async () => {
  const retry = deferred<AgentStatus>();
  const initialization = deferred<AgentStatus>();
  const initializationEntered = deferred<void>();
  const controller = new AgentLifecycleController({
    ...mockBridge(FIXTURE),
    retryAgentConnection: () => retry.promise,
    initializeClientState: () => {
      initializationEntered.resolve(undefined);
      return initialization.promise;
    },
  });
  controller.disconnect('socket closed');
  const pending = controller.establish(true);
  assert.equal(controller.snapshot().state, 'disconnected');
  retry.resolve({ state: 'bootstrap', step: 'initialize-state' });
  await initializationEntered.promise;
  assert.equal(controller.snapshot().state, 'disconnected');
  initialization.resolve({ state: 'ready' });
  assert.deepEqual(await pending, { state: 'ready' });
  assert.equal(controller.snapshot().state, 'ready');
});

test('failed reconnect restores disconnected and rejects', async () => {
  const failure = new Error('service did not restart');
  const controller = new AgentLifecycleController({
    ...mockBridge(FIXTURE),
    retryAgentConnection: async () => {
      throw failure;
    },
  });
  controller.disconnect('socket closed');
  await assert.rejects(controller.establish(true), failure);
  assert.deepEqual(controller.snapshot(), {
    state: 'disconnected',
    error: 'socket closed',
  });
});

test('maintenance arriving during reconnect remains authoritative', async () => {
  const retry = deferred<AgentStatus>();
  const controller = new AgentLifecycleController({
    ...mockBridge(FIXTURE),
    retryAgentConnection: () => retry.promise,
  });
  controller.disconnect('socket closed');
  const pending = controller.establish(true);
  controller.applyMaintenance({
    state: 'active',
    generation: 1,
    revision: 1,
    kind: 'verify',
    phase: 'running',
  });
  retry.resolve({ state: 'ready' });
  await assert.rejects(pending);
  assert.deepEqual(controller.snapshot(), {
    state: 'maintenance',
    generation: 1,
    kind: 'verify',
    phase: 'running',
  });
});

test('disconnect arriving during maintenance is ignored', () => {
  const controller = new AgentLifecycleController(mockBridge(FIXTURE), {
    state: 'ready',
  });
  controller.applyMaintenance({
    state: 'active',
    generation: 1,
    revision: 1,
    kind: 'export',
    phase: 'quiescing',
  });
  assert.equal(controller.disconnect('late socket event'), false);
  assert.deepEqual(controller.snapshot(), {
    state: 'maintenance',
    generation: 1,
    kind: 'export',
    phase: 'quiescing',
  });
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
