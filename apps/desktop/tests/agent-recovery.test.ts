import assert from 'node:assert/strict';
import test from 'node:test';

import { AgentLifecycleController } from '../src/agent-lifecycle';
import {
  AgentRecoveryController,
  type AgentRecoveryClock,
} from '../src/agent-recovery';
import type { CommandError } from '../src/bridge';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';
import type { AgentStatus } from '../src/model';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((accept, decline) => {
    resolve = accept;
    reject = decline;
  });
  return { promise, resolve, reject };
}

function failure(code = 'agent-lost', ambiguous = false): CommandError {
  return {
    code,
    message: code,
    retryable: true,
    ambiguous,
    fatal: code === 'agent-lost',
  };
}

async function flush(): Promise<void> {
  for (let index = 0; index < 20; index++) await Promise.resolve();
}

class Clock implements AgentRecoveryClock {
  time = 0;
  next = 0;
  tasks = new Map<number, { at: number; callback: () => void }>();

  later(callback: () => void, milliseconds: number): unknown {
    const handle = ++this.next;
    this.tasks.set(handle, { at: this.time + milliseconds, callback });
    return handle;
  }

  cancel(handle: unknown): void {
    this.tasks.delete(handle as number);
  }

  now(): number {
    return this.time;
  }

  random(): number {
    return 0.5;
  }

  async advance(milliseconds: number): Promise<void> {
    this.time += milliseconds;
    for (const [handle, task] of [...this.tasks]) {
      if (task.at > this.time) continue;
      this.tasks.delete(handle);
      task.callback();
    }
    await flush();
  }
}

function setup(
  options: {
    automatic?: () => Promise<AgentStatus>;
    manual?: () => Promise<AgentStatus>;
    reconcile?: (
      status: AgentStatus,
      isCurrent: () => boolean,
    ) => Promise<void>;
  } = {},
) {
  const clock = new Clock();
  const calls = { automatic: 0, manual: 0, initialize: 0, reconcile: 0 };
  const permission = { allowed: true };
  const lifecycle = new AgentLifecycleController(
    {
      ...mockBridge(FIXTURE),
      retryAgentConnection: async () => {
        calls.manual++;
        return options.manual ? options.manual() : { state: 'ready' };
      },
      initializeClientState: async () => {
        calls.initialize++;
        return { state: 'ready' };
      },
    },
    { state: 'ready' },
    async () => {
      calls.automatic++;
      return options.automatic ? options.automatic() : { state: 'ready' };
    },
  );
  const recovery = new AgentRecoveryController({
    lifecycle,
    clock,
    isRecoveryPermitted: () => permission.allowed,
    reconcile: async (status, isCurrent) => {
      calls.reconcile++;
      await options.reconcile?.(status, isCurrent);
    },
  });
  return { clock, calls, permission, lifecycle, recovery };
}

function maintenance(lifecycle: AgentLifecycleController): void {
  lifecycle.applyMaintenance({
    state: 'active',
    generation: 1,
    revision: 1,
    kind: 'verify',
    phase: 'running',
  });
}

test('automatic health recovery shares one bounded sequence and never replays a mutation', async () => {
  const error = failure('agent-lost', true);
  const { recovery, lifecycle, calls, clock } = setup({
    automatic: async () => {
      throw error;
    },
  });
  const generation = recovery.captureGeneration();
  const pending = recovery.recover(error, { generation });
  const rejected = assert.rejects(pending, (value: unknown) => value === error);
  assert.equal(recovery.recover(error), pending);
  await flush();
  assert.equal(calls.automatic, 1);
  assert.equal(recovery.snapshot().nextRetryAt, 1_000);
  await clock.advance(999);
  assert.equal(calls.automatic, 1);
  await clock.advance(1);
  assert.equal(calls.automatic, 2);
  await clock.advance(2_000);
  assert.equal(calls.automatic, 3);
  await clock.advance(4_000);
  await rejected;
  assert.equal(calls.automatic, 4);
  assert.equal(calls.manual, 0);
  assert.equal(calls.initialize, 0);
  assert.equal(calls.reconcile, 0);
  assert.equal(lifecycle.snapshot().state, 'disconnected');
  assert.equal(recovery.snapshot().halted, true);
  assert.equal(clock.tasks.size, 0);
  await recovery.recover(error);
  await clock.advance(60_000);
  assert.equal(calls.automatic, 4);
  assert.deepEqual(await recovery.retry(), { state: 'ready' });
  assert.equal(calls.manual, 1);
  recovery.dispose();
});

test('bootstrap result stops automatic recovery without initialization', async () => {
  const status: AgentStatus = { state: 'bootstrap', step: 'create-state' };
  const { recovery, lifecycle, calls, clock } = setup({
    automatic: async () => status,
  });
  assert.deepEqual(await recovery.recover(failure()), status);
  assert.deepEqual(lifecycle.snapshot(), status);
  assert.equal(calls.initialize, 0);
  assert.equal(calls.reconcile, 0);
  assert.equal(recovery.snapshot().halted, true);
  await recovery.recover(failure());
  await clock.advance(60_000);
  assert.equal(calls.automatic, 1);
  recovery.dispose();
});

test('manual Retry shares an in-flight automatic attempt and its reconciliation', async () => {
  const automatic = deferred<AgentStatus>();
  const reconciliation = deferred<void>();
  const { recovery, calls } = setup({
    automatic: () => automatic.promise,
    reconcile: () => reconciliation.promise,
  });
  const pending = recovery.recover(failure());
  await flush();
  assert.equal(recovery.retry(), pending);
  automatic.resolve({ state: 'ready' });
  await flush();
  assert.equal(recovery.retry(), pending);
  assert.equal(calls.reconcile, 1);
  reconciliation.resolve(undefined);
  assert.deepEqual(await pending, { state: 'ready' });
  assert.equal(calls.automatic, 1);
  assert.equal(calls.manual, 0);
  recovery.dispose();
});

test('manual Retry cancels scheduled backoff and retains one shared result', async () => {
  const { recovery, clock, calls } = setup({
    automatic: async () => {
      throw failure();
    },
  });
  const pending = recovery.recover(failure());
  await flush();
  assert.equal(clock.tasks.size, 1);
  assert.equal(recovery.retry(), pending);
  assert.equal(clock.tasks.size, 0);
  assert.deepEqual(await pending, { state: 'ready' });
  await clock.advance(60_000);
  assert.equal(calls.automatic, 1);
  assert.equal(calls.manual, 1);
  recovery.dispose();
});

test('lock cancellation retires pending success and queued retries', async () => {
  const automatic = deferred<AgentStatus>();
  const { recovery, permission, lifecycle, calls, clock } = setup({
    automatic: () => automatic.promise,
  });
  const pending = recovery.recover(failure());
  await flush();
  permission.allowed = false;
  recovery.cancel();
  assert.equal(await pending, undefined);
  automatic.resolve({ state: 'ready' });
  await flush();
  assert.equal(lifecycle.snapshot().state, 'disconnected');
  assert.equal(calls.reconcile, 0);
  await recovery.retry();
  await recovery.recover(failure());
  await clock.advance(60_000);
  assert.equal(calls.automatic, 1);
  assert.equal(calls.manual, 0);
  recovery.dispose();
});

test('permission checks cancel backoff even without a lock notification', async () => {
  const { recovery, permission, clock, calls } = setup({
    automatic: async () => {
      throw failure();
    },
  });
  const pending = recovery.recover(failure());
  await flush();
  permission.allowed = false;
  await clock.advance(1_000);
  assert.equal(await pending, undefined);
  assert.equal(clock.tasks.size, 0);
  assert.equal(calls.automatic, 1);
  recovery.dispose();
});

test('late completion checks permission before publishing lifecycle readiness', async () => {
  const automatic = deferred<AgentStatus>();
  const { recovery, permission, lifecycle, calls } = setup({
    automatic: () => automatic.promise,
  });
  const pending = recovery.recover(failure());
  await flush();
  permission.allowed = false;
  automatic.resolve({ state: 'ready' });
  assert.equal(await pending, undefined);
  assert.equal(lifecycle.snapshot().state, 'disconnected');
  assert.equal(calls.reconcile, 0);
  recovery.dispose();
});

test('manual reconnect cannot initialize bootstrap after permission is revoked', async () => {
  const manual = deferred<AgentStatus>();
  const { recovery, permission, lifecycle, calls } = setup({
    manual: () => manual.promise,
  });
  lifecycle.disconnect('socket closed');
  const pending = recovery.retry();
  await flush();
  permission.allowed = false;
  manual.resolve({ state: 'bootstrap', step: 'create-state' });
  assert.equal(await pending, undefined);
  assert.equal(lifecycle.snapshot().state, 'disconnected');
  assert.equal(calls.initialize, 0);
  assert.equal(calls.reconcile, 0);
  recovery.dispose();
});

test('explicit-action events do not start automatic recovery or clear on cancellation', async () => {
  const { recovery, lifecycle, calls } = setup();
  await recovery.recover(failure('unsafe-socket'));
  assert.equal(lifecycle.snapshot().state, 'failure');
  assert.equal(recovery.snapshot().halted, true);
  recovery.cancel();
  await recovery.recover(failure());
  assert.equal(lifecycle.snapshot().state, 'failure');
  assert.equal(calls.automatic, 0);
  recovery.dispose();
});

test('a current explicit-action event preempts an in-flight recovery', async () => {
  const automatic = deferred<AgentStatus>();
  const { recovery, lifecycle, calls } = setup({
    automatic: () => automatic.promise,
  });
  const pending = recovery.recover(failure());
  await flush();
  await recovery.recover(failure('version-mismatch'), {
    generation: recovery.captureGeneration(),
  });
  assert.equal(await pending, undefined);
  assert.equal(recovery.snapshot().halted, true);
  automatic.resolve({ state: 'ready' });
  await flush();
  assert.equal(lifecycle.snapshot().state, 'failure');
  assert.equal(calls.reconcile, 0);
  recovery.dispose();
});

test('terminal maintenance states exclude automatic and generic manual recovery', async () => {
  for (const disposition of [
    { status: 'restart-selected-root', root: '/tmp/selected' } as const,
    { status: 'recovery-required', root: '/tmp/selected' } as const,
    {
      status: 'restoration-failed',
      root: '/tmp/selected',
      error: failure(),
    } as const,
  ]) {
    const { recovery, lifecycle, calls } = setup();
    lifecycle.applyMaintenance({
      state: 'complete',
      generation: 1,
      revision: 1,
      kind: 'verify',
      operation: { status: 'completed' },
      disposition,
    });
    const before = lifecycle.snapshot();
    await recovery.recover(failure());
    await recovery.retry();
    assert.deepEqual(lifecycle.snapshot(), before);
    assert.equal(calls.automatic, 0);
    assert.equal(calls.manual, 0);
    recovery.dispose();
  }
});

test('disposal clears scheduled backoff and prevents subsequent recovery', async () => {
  const { recovery, calls, clock } = setup({
    automatic: async () => {
      throw failure();
    },
  });
  const pending = recovery.recover(failure());
  await flush();
  recovery.dispose();
  assert.equal(await pending, undefined);
  assert.equal(clock.tasks.size, 0);
  await clock.advance(60_000);
  await recovery.retry();
  assert.equal(calls.automatic, 1);
  assert.equal(calls.manual, 0);
});

test('maintenance preempts pending recovery and preserves authoritative state', async () => {
  const automatic = deferred<AgentStatus>();
  const { recovery, lifecycle, calls, clock } = setup({
    automatic: () => automatic.promise,
  });
  const pending = recovery.recover(failure());
  await flush();
  maintenance(lifecycle);
  assert.equal(await pending, undefined);
  automatic.reject(failure());
  await flush();
  await recovery.retry();
  await clock.advance(60_000);
  assert.equal(lifecycle.snapshot().state, 'maintenance');
  assert.equal(calls.automatic, 1);
  assert.equal(calls.manual, 0);
  assert.equal(calls.reconcile, 0);
  recovery.dispose();
});

test('maintenance preempts scheduled backoff', async () => {
  const { recovery, lifecycle, calls, clock } = setup({
    automatic: async () => {
      throw failure();
    },
  });
  const pending = recovery.recover(failure());
  await flush();
  maintenance(lifecycle);
  assert.equal(await pending, undefined);
  assert.equal(clock.tasks.size, 0);
  await clock.advance(60_000);
  assert.equal(calls.automatic, 1);
  recovery.dispose();
});

test('catalog reconciliation failure leaves the recovered agent ready and Retry only refreshes', async () => {
  const catalogError = failure('catalog-required');
  let refreshes = 0;
  const { recovery, lifecycle, calls, clock } = setup({
    reconcile: async () => {
      if (++refreshes === 1) throw catalogError;
    },
  });
  await assert.rejects(
    recovery.recover(failure()),
    (error: unknown) => error === catalogError,
  );
  assert.equal(lifecycle.snapshot().state, 'ready');
  assert.equal(clock.tasks.size, 0);
  assert.equal(recovery.snapshot().halted, false);
  await recovery.recover(catalogError);
  await recovery.retry();
  assert.equal(calls.automatic, 1);
  assert.equal(calls.manual, 0);
  assert.equal(calls.reconcile, 2);
  recovery.dispose();
});

test('health-probe scope admits retryable probe failures without treating ordinary catalog errors as agent loss', async () => {
  const { recovery, calls } = setup();
  const error = failure('probe-timeout');
  await recovery.recover(error);
  assert.equal(calls.automatic, 0);
  assert.deepEqual(await recovery.recover(error, { scope: 'health-probe' }), {
    state: 'ready',
  });
  assert.equal(calls.automatic, 1);
  recovery.dispose();
});

test('old request failures cannot disconnect a successfully recovered agent', async () => {
  const { recovery, lifecycle, calls } = setup();
  const generation = recovery.captureGeneration();
  await recovery.recover(failure(), { generation });
  await recovery.recover(failure(), { generation });
  await recovery.recover(failure('version-mismatch'), { generation });
  assert.equal(lifecycle.snapshot().state, 'ready');
  assert.equal(calls.automatic, 1);
  assert.equal(recovery.snapshot().halted, false);
  recovery.dispose();
});

test('cancelled native failure cannot roll back a queued successful manual recovery', async () => {
  const automatic = deferred<AgentStatus>();
  const { recovery, lifecycle, calls } = setup({
    automatic: () => automatic.promise,
  });
  const old = recovery.recover(failure());
  await flush();
  recovery.cancel();
  const manual = recovery.retry();
  await flush();
  assert.equal(calls.manual, 0);
  automatic.reject(failure('version-mismatch'));
  assert.equal(await old, undefined);
  assert.deepEqual(await manual, { state: 'ready' });
  assert.equal(lifecycle.snapshot().state, 'ready');
  assert.equal(calls.manual, 1);
  assert.equal(calls.reconcile, 1);
  recovery.dispose();
});

test('cancelled reconciliation receives a validity guard and cannot affect the next sequence', async () => {
  const oldRefresh = deferred<void>();
  let oldIsCurrent!: () => boolean;
  let refreshes = 0;
  const { recovery, lifecycle, calls } = setup({
    reconcile: async (_status, isCurrent) => {
      if (++refreshes === 1) {
        oldIsCurrent = isCurrent;
        await oldRefresh.promise;
      }
    },
  });
  const old = recovery.recover(failure());
  await flush();
  recovery.cancel();
  assert.equal(oldIsCurrent(), false);
  assert.equal(await old, undefined);
  await recovery.retry();
  oldRefresh.reject(failure());
  await flush();
  assert.equal(lifecycle.snapshot().state, 'ready');
  assert.equal(calls.automatic, 1);
  assert.equal(calls.manual, 0);
  assert.equal(calls.reconcile, 2);
  recovery.dispose();
});

for (const code of [
  'bootstrap-required',
  'unsafe-socket',
  'version-mismatch',
  'integrity',
  'agent-takeover-required',
  'state-recovery-required',
  'state-restart-required',
  'restoration-failed',
  'unsupported-schema',
]) {
  test(`${code} halts automatic recovery despite a retryable flag`, async () => {
    const error = failure(code);
    const { recovery, lifecycle, calls, clock } = setup({
      automatic: async () => {
        throw error;
      },
    });
    await assert.rejects(
      recovery.recover(failure()),
      (value: unknown) => value === error,
    );
    assert.equal(recovery.snapshot().halted, true);
    assert.equal(
      lifecycle.snapshot().state,
      code === 'bootstrap-required' ? 'bootstrap' : 'failure',
    );
    assert.equal(clock.tasks.size, 0);
    await recovery.recover(failure());
    await clock.advance(60_000);
    assert.equal(calls.automatic, 1);
    assert.equal(calls.initialize, 0);
    recovery.dispose();
  });
}

test('credential-blocked recovery waits for explicit restoration and reconciles afterward', async () => {
  const error = { ...failure('agent-credentials-required'), retryable: false };
  const { recovery, lifecycle, calls, clock } = setup({
    automatic: async () => {
      throw error;
    },
  });
  await assert.rejects(recovery.recover(failure()), (value) => value === error);
  assert.equal(lifecycle.snapshot().state, 'failure');
  assert.equal(recovery.snapshot().halted, true);
  await clock.advance(60_000);
  await recovery.recover(failure());
  assert.equal(calls.automatic, 1);
  assert.equal(calls.manual, 0);
  await recovery.retry();
  assert.equal(calls.manual, 1);
  assert.equal(calls.reconcile, 1);
  assert.equal(lifecycle.snapshot().state, 'ready');
  recovery.dispose();
});
