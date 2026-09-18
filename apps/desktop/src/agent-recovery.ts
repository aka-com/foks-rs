import type {
  AgentLifecycle,
  AgentLifecycleController,
} from './agent-lifecycle';
import type { CommandError } from './bridge';
import { normalizeCommandError } from './bridge';
import type { AgentStatus } from './model';

export interface AgentRecoveryClock {
  later(callback: () => void, milliseconds: number): unknown;
  cancel(handle: unknown): void;
  now(): number;
  random(): number;
}

export interface AgentRecoveryContext {
  generation?: number;
  scope?: 'request' | 'health-probe';
}

export interface AgentRecoveryOptions {
  lifecycle: AgentLifecycleController;
  isRecoveryPermitted: () => boolean;
  isReady?: () => boolean;
  reconcile: (status: AgentStatus, isCurrent: () => boolean) => Promise<void>;
  clock?: AgentRecoveryClock;
}

export interface AgentRecoverySnapshot {
  generation: number;
  pending: boolean;
  halted: boolean;
  attempts: number;
  nextRetryAt: number | null;
}

type RecoveryResult = AgentStatus | undefined;

type Sequence = {
  generation: number;
  promise: Promise<RecoveryResult>;
  resolve: (status: RecoveryResult) => void;
  reject: (error: unknown) => void;
  attempts: number;
  inFlight: boolean;
  timer: { handle: unknown } | null;
  nextRetryAt: number | null;
};

const RETRY_DELAYS = [1_000, 2_000, 4_000] as const;

const defaultClock: AgentRecoveryClock = {
  later: (callback, milliseconds) => setTimeout(callback, milliseconds),
  cancel: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
  now: () => Date.now(),
  random: () => Math.random(),
};

function maintenanceBlocks(state: AgentLifecycle): boolean {
  return [
    'maintenance',
    'restart-required',
    'recovery-required',
    'restoration-failed',
  ].includes(state.state);
}

function requiresExplicitAction(error: CommandError): boolean {
  return (
    (error.fatal &&
      !['agent-lost', 'agent-start-failed'].includes(error.code)) ||
    /integrity|takeover/.test(error.code) ||
    [
      'bootstrap-required',
      'unsafe-socket',
      'version-mismatch',
      'state-recovery-required',
      'state-restart-required',
      'restoration-failed',
      'unsupported-schema',
      'import-verification-required',
    ].includes(error.code)
  );
}

export function isAutomaticRecoveryError(
  error: CommandError,
  scope: AgentRecoveryContext['scope'] = 'request',
): boolean {
  return (
    !requiresExplicitAction(error) &&
    error.retryable &&
    (error.code === 'agent-lost' ||
      error.code === 'agent-start-failed' ||
      scope === 'health-probe')
  );
}

export class AgentRecoveryController {
  readonly #options: AgentRecoveryOptions;
  readonly #clock: AgentRecoveryClock;
  readonly #unsubscribe: () => void;
  #generation = 0;
  #active: Sequence | null = null;
  #halted = false;
  #disposed = false;
  #attempts = 0;

  constructor(options: AgentRecoveryOptions) {
    this.#options = options;
    this.#clock = options.clock ?? defaultClock;
    this.#unsubscribe = options.lifecycle.subscribe((state) => {
      if (maintenanceBlocks(state)) this.cancel();
    });
  }

  captureGeneration(): number {
    return this.#generation;
  }

  snapshot(): AgentRecoverySnapshot {
    return {
      generation: this.#generation,
      pending: this.#active !== null,
      halted: this.#halted,
      attempts: this.#active?.attempts ?? this.#attempts,
      nextRetryAt: this.#active?.nextRetryAt ?? null,
    };
  }

  recover(
    error: unknown,
    context: AgentRecoveryContext = {},
  ): Promise<RecoveryResult> {
    if (
      context.generation !== undefined &&
      context.generation !== this.#generation
    )
      return Promise.resolve(undefined);
    if (!this.#permitted()) {
      this.cancel();
      return Promise.resolve(undefined);
    }
    const typed = normalizeCommandError(error);
    if (requiresExplicitAction(typed)) {
      this.cancel();
      this.#halted = true;
      if (typed.code === 'bootstrap-required')
        this.#options.lifecycle.requireBootstrap(
          typed.details?.reason ?? 'initialize-state',
        );
      else this.#options.lifecycle.fail(typed);
      return Promise.resolve(undefined);
    }
    if (this.#active) return this.#active.promise;
    if (this.#halted) return Promise.resolve(undefined);
    if (!isAutomaticRecoveryError(typed, context.scope))
      return Promise.resolve(undefined);
    if (
      ['bootstrap', 'initializing'].includes(
        this.#options.lifecycle.snapshot().state,
      )
    )
      return Promise.resolve(undefined);
    this.#options.lifecycle.disconnect(typed.message);
    return this.#start(false);
  }

  retry(): Promise<RecoveryResult> {
    if (!this.#permitted()) {
      this.cancel();
      return Promise.resolve(undefined);
    }
    const active = this.#active;
    if (active) {
      if (active.timer) {
        this.#clock.cancel(active.timer.handle);
        active.timer = null;
        active.nextRetryAt = null;
        void this.#attempt(active, true);
      }
      return active.promise;
    }
    this.#halted = false;
    return this.#start(true);
  }

  cancel(): void {
    this.#generation++;
    const active = this.#active;
    this.#active = null;
    if (!active) return;
    if (active.timer) this.#clock.cancel(active.timer.handle);
    if (active.inFlight) this.#options.lifecycle.invalidatePending();
    this.#attempts = active.attempts;
    active.resolve(undefined);
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.cancel();
    this.#unsubscribe();
  }

  #permitted(): boolean {
    return (
      !this.#disposed &&
      this.#options.isRecoveryPermitted() &&
      !maintenanceBlocks(this.#options.lifecycle.snapshot())
    );
  }

  #current(sequence: Sequence): boolean {
    return (
      this.#active === sequence &&
      sequence.generation === this.#generation &&
      this.#permitted()
    );
  }

  #start(manual: boolean): Promise<RecoveryResult> {
    let resolve!: Sequence['resolve'];
    let reject!: Sequence['reject'];
    const promise = new Promise<RecoveryResult>((accept, decline) => {
      resolve = accept;
      reject = decline;
    });
    const sequence: Sequence = {
      generation: ++this.#generation,
      promise,
      resolve,
      reject,
      attempts: 0,
      inFlight: false,
      timer: null,
      nextRetryAt: null,
    };
    this.#active = sequence;
    void Promise.resolve().then(() => this.#attempt(sequence, manual));
    return promise;
  }

  #finish(
    sequence: Sequence,
    result: RecoveryResult,
    error?: { value: unknown },
  ): void {
    if (this.#active !== sequence) return;
    this.#active = null;
    this.#generation++;
    this.#attempts = sequence.attempts;
    if (error) sequence.reject(error.value);
    else sequence.resolve(result);
  }

  async #attempt(sequence: Sequence, manual: boolean): Promise<void> {
    if (!this.#current(sequence)) {
      if (this.#active === sequence) this.cancel();
      return;
    }
    sequence.attempts++;
    sequence.inFlight = true;
    let status: AgentStatus;
    const isCurrent = () => this.#current(sequence);
    try {
      const ready =
        this.#options.isReady?.() ??
        this.#options.lifecycle.snapshot().state === 'ready';
      status = manual
        ? ready
          ? { state: 'ready' }
          : await this.#options.lifecycle.establish(true, isCurrent)
        : await this.#options.lifecycle.establishAutomatic(isCurrent);
    } catch (error) {
      sequence.inFlight = false;
      if (!isCurrent()) {
        if (this.#active === sequence) this.cancel();
        return;
      }
      const typed = normalizeCommandError(error);
      if (
        !manual &&
        isAutomaticRecoveryError(typed) &&
        sequence.attempts <= RETRY_DELAYS.length
      ) {
        const delay =
          RETRY_DELAYS[sequence.attempts - 1] *
          (0.8 + 0.4 * Math.max(0, Math.min(1, this.#clock.random())));
        sequence.nextRetryAt = this.#clock.now() + delay;
        sequence.timer = {
          handle: this.#clock.later(() => {
            sequence.timer = null;
            sequence.nextRetryAt = null;
            void this.#attempt(sequence, false);
          }, delay),
        };
        return;
      }
      this.#halted = true;
      if (!manual && requiresExplicitAction(typed)) {
        if (typed.code === 'bootstrap-required')
          this.#options.lifecycle.requireBootstrap(
            typed.details?.reason ?? 'initialize-state',
          );
        else this.#options.lifecycle.fail(typed);
      }
      this.#finish(sequence, undefined, { value: error });
      return;
    }
    sequence.inFlight = false;
    if (!isCurrent()) {
      if (this.#active === sequence) this.cancel();
      return;
    }
    if (status.state === 'bootstrap') {
      this.#halted = true;
      this.#finish(sequence, status);
      return;
    }
    try {
      await this.#options.reconcile(status, isCurrent);
      if (!isCurrent()) {
        if (this.#active === sequence) this.cancel();
        return;
      }
      this.#halted = false;
      this.#finish(sequence, status);
    } catch (error) {
      if (!isCurrent()) {
        if (this.#active === sequence) this.cancel();
        return;
      }
      this.#finish(sequence, undefined, { value: error });
    }
  }
}
