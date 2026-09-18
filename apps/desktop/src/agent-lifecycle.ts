import type { AgentStatus } from './model';
import type {
  Bridge,
  CommandError,
  MaintenanceKind,
  MaintenanceOperationOutcome,
  MaintenancePhase,
  MaintenanceSnapshot,
} from './bridge';

export type AgentLifecycle =
  | { readonly state: 'checking' }
  | { readonly state: 'bootstrap'; readonly step: string }
  | { readonly state: 'initializing'; readonly step: string }
  | { readonly state: 'ready' }
  | {
      readonly state: 'maintenance';
      readonly generation: number;
      readonly kind: MaintenanceKind;
      readonly phase: MaintenancePhase;
    }
  | {
      readonly state: 'restart-required';
      readonly generation: number;
      readonly operation: MaintenanceOperationOutcome;
      readonly root: string;
    }
  | {
      readonly state: 'recovery-required';
      readonly generation: number;
      readonly operation: MaintenanceOperationOutcome;
      readonly root: string;
    }
  | {
      readonly state: 'restoration-failed';
      readonly generation: number;
      readonly operation: MaintenanceOperationOutcome;
      readonly error: CommandError;
    }
  | { readonly state: 'disconnected'; readonly error?: string }
  | { readonly state: 'failure'; readonly error: unknown };

export function lifecycleFromStatus(status: AgentStatus): AgentLifecycle {
  return status.state === 'ready'
    ? { state: 'ready' }
    : { state: 'bootstrap', step: status.step };
}

export function agentStatusLabel(status: AgentStatus): string {
  return status.state === 'ready' ? 'Ready' : `Bootstrap · ${status.step}`;
}

export function agentLifecycleLabel(lifecycle: AgentLifecycle): string {
  switch (lifecycle.state) {
    case 'ready':
      return 'Ready';
    case 'checking':
      return 'Checking';
    // Expose internal bootstrap and initialization steps as one startup state.
    case 'bootstrap':
    case 'initializing':
      return 'Starting the FOKS agent';
    case 'maintenance':
      return `${lifecycle.kind} · ${lifecycle.phase}`;
    case 'restart-required':
      return 'Restart required';
    case 'recovery-required':
      return 'Recovery required';
    case 'restoration-failed':
      return 'Service restart failed';
    case 'disconnected':
      return 'Disconnected';
    case 'failure':
      return 'Unavailable';
  }
}

export function isAgentReady(lifecycle: AgentLifecycle): boolean {
  return lifecycle.state === 'ready';
}

/**
 * Report the operation separately from whether its agent could be restored.
 * A restart is the one kind with no operation of its own, so it is named
 * rather than called maintenance.
 */
export function maintenanceOutcomeMessage(
  outcome: MaintenanceOperationOutcome,
  kind?: MaintenanceKind,
): string {
  const what = kind === 'restart' ? 'The agent restart' : 'State maintenance';
  switch (outcome.status) {
    case 'completed':
      return kind === 'restart'
        ? 'The agent was restarted.'
        : 'State maintenance completed.';
    case 'cancelled':
      return `${what} was cancelled.`;
    case 'failed':
      return `${what} failed: ${outcome.error.message}`;
  }
}

class StaleAgentLifecycle extends Error {}

export class AgentLifecycleController {
  readonly #bridge: Bridge;
  readonly #autoRecover: () => Promise<AgentStatus>;
  readonly #listeners = new Set<(state: AgentLifecycle) => void>();
  #state: AgentLifecycle;
  #generation = 0;
  #maintenanceGeneration = 0;
  #maintenanceRevision = 0;
  #inFlight: { generation: number; promise: Promise<AgentStatus> } | null =
    null;

  constructor(
    bridge: Bridge,
    status?: AgentStatus,
    autoRecover: () => Promise<AgentStatus> = () => bridge.agentStatus(),
  ) {
    this.#bridge = bridge;
    this.#autoRecover = autoRecover;
    this.#state = status ? lifecycleFromStatus(status) : { state: 'checking' };
  }

  snapshot(): AgentLifecycle {
    return this.#state;
  }

  subscribe(listener: (state: AgentLifecycle) => void): () => void {
    this.#listeners.add(listener);
    listener(this.#state);
    return () => this.#listeners.delete(listener);
  }

  disconnect(message?: string): boolean {
    if (
      this.#state.state === 'disconnected' ||
      this.#state.state === 'maintenance' ||
      this.#state.state === 'restart-required' ||
      this.#state.state === 'recovery-required' ||
      this.#state.state === 'restoration-failed'
    )
      return false;
    this.#generation++;
    this.#publish({ state: 'disconnected', error: message });
    return true;
  }

  requireBootstrap(step: string): void {
    this.#generation++;
    this.#publish({ state: 'bootstrap', step });
  }

  fail(error: unknown): void {
    this.#generation++;
    this.#publish({ state: 'failure', error });
  }

  applyMaintenance(snapshot: MaintenanceSnapshot): boolean {
    if (snapshot.generation < this.#maintenanceGeneration) return false;
    if (
      snapshot.generation === this.#maintenanceGeneration &&
      snapshot.revision <= this.#maintenanceRevision
    )
      return false;
    if (snapshot.generation > this.#maintenanceGeneration)
      this.#maintenanceRevision = 0;
    this.#maintenanceGeneration = snapshot.generation;
    this.#maintenanceRevision = snapshot.revision;
    if (snapshot.state === 'idle') return true;
    this.#generation++;
    if (snapshot.state === 'active') {
      this.#publish({
        state: 'maintenance',
        generation: snapshot.generation,
        kind: snapshot.kind,
        phase: snapshot.phase,
      });
      return true;
    }
    switch (snapshot.disposition.status) {
      case 'continue-current-root':
        // Restoration may return a bootstrap status. Query the normal lifecycle path
        // instead of treating maintenance completion as proof that the agent is ready.
        this.#publish({ state: 'checking' });
        return true;
      case 'restart-selected-root':
        this.#publish({
          state: 'restart-required',
          generation: snapshot.generation,
          operation: snapshot.operation,
          root: snapshot.disposition.root,
        });
        return true;
      case 'recovery-required':
        this.#publish({
          state: 'recovery-required',
          generation: snapshot.generation,
          operation: snapshot.operation,
          root: snapshot.disposition.root,
        });
        return true;
      case 'restoration-failed':
        this.#publish({
          state: 'restoration-failed',
          generation: snapshot.generation,
          operation: snapshot.operation,
          error: snapshot.disposition.error,
        });
        return true;
    }
  }

  invalidatePending(): void {
    this.#generation++;
  }

  establishAutomatic(
    isCurrent: () => boolean = () => true,
  ): Promise<AgentStatus> {
    return this.#establish('automatic', isCurrent);
  }

  establish(
    reconnect = false,
    isCurrent: () => boolean = () => true,
  ): Promise<AgentStatus> {
    return this.#establish(reconnect ? 'reconnect' : 'initial', isCurrent);
  }

  #establish(
    mode: 'automatic' | 'reconnect' | 'initial',
    isCurrent: () => boolean = () => true,
  ): Promise<AgentStatus> {
    const requestedGeneration = this.#generation;
    const live = this.#inFlight;
    if (live?.generation === requestedGeneration) return live.promise;
    const pending = live
      ? live.promise
          .catch(() => undefined)
          .then(() => this.#run(mode, requestedGeneration, isCurrent))
      : this.#run(mode, requestedGeneration, isCurrent);
    const entry = { generation: requestedGeneration, promise: pending };
    this.#inFlight = entry;
    void pending
      .finally(() => {
        if (this.#inFlight === entry) this.#inFlight = null;
      })
      .catch(() => undefined);
    return pending;
  }

  #run(
    mode: 'automatic' | 'reconnect' | 'initial',
    generation: number,
    isCurrent: () => boolean,
  ): Promise<AgentStatus> {
    const requireCurrent = () => {
      this.#requireCurrent(generation);
      if (!isCurrent()) throw new StaleAgentLifecycle();
    };
    const disconnected =
      mode !== 'initial' && this.#state.state === 'disconnected'
        ? this.#state
        : null;
    return (async () => {
      requireCurrent();
      if (
        mode === 'automatic' &&
        [
          'bootstrap',
          'initializing',
          'maintenance',
          'restart-required',
          'recovery-required',
          'restoration-failed',
        ].includes(this.#state.state)
      )
        throw new StaleAgentLifecycle();
      if (!disconnected) this.#publish({ state: 'checking' });
      requireCurrent();
      let status =
        mode === 'automatic'
          ? await this.#autoRecover()
          : mode === 'reconnect'
            ? await this.#bridge.retryAgentConnection()
            : await this.#bridge.agentStatus();
      requireCurrent();
      if (mode === 'automatic') {
        this.#publish(lifecycleFromStatus(status));
        return status;
      }
      if (status.state === 'bootstrap') {
        if (!disconnected)
          this.#publish({ state: 'initializing', step: status.step });
        requireCurrent();
        status = await this.#bridge.initializeClientState();
        requireCurrent();
      }
      if (status.state !== 'ready') {
        const error: CommandError = {
          code: 'bootstrap-required',
          message: 'The local agent did not become ready after initialization.',
          retryable: false,
          ambiguous: false,
          fatal: false,
          details: { reason: status.step },
        };
        throw error;
      }
      this.#publish({ state: 'ready' });
      return status;
    })().catch((error) => {
      if (
        !(error instanceof StaleAgentLifecycle) &&
        generation === this.#generation &&
        isCurrent()
      )
        this.#publish(disconnected ?? { state: 'failure', error });
      throw error;
    });
  }

  #requireCurrent(generation: number): void {
    if (generation !== this.#generation) throw new StaleAgentLifecycle();
  }

  #publish(state: AgentLifecycle): void {
    this.#state = state;
    for (const listener of this.#listeners) listener(state);
  }
}
