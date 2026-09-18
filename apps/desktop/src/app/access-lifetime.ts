export type AccessRetirementReason =
  | 'lock'
  | 'disconnect'
  | 'maintenance'
  | 'access-change'
  | 'scope-removed'
  | 'dispose';

export interface AccessRetirement {
  readonly reason: AccessRetirementReason;
  readonly generation: number;
  readonly profile?: string;
}

export interface AccessTicket {
  readonly session: object;
  readonly generation: number;
  readonly profileGeneration?: number;
  readonly signal: AbortSignal;
  isCurrent(this: void): boolean;
}

interface ProfileLifetime {
  generation: number;
  controller: AbortController;
}

export class AccessLifetime {
  constructor(readonly owner: object = Object.freeze({})) {}

  private sessionIdentity: object = Object.freeze({});
  private revision = 0;
  private version = 0;
  readonly getSnapshot = (): number => this.version;
  private controller = new AbortController();
  private profiles = new Map<string, ProfileLifetime>();
  private listeners = new Set<(event: AccessRetirement) => void>();
  private disposed = false;

  get generation(): number {
    return this.revision;
  }
  get session(): object {
    return this.sessionIdentity;
  }

  capture(profile?: string): AccessTicket {
    const generation = this.revision;
    const scope = profile === undefined ? undefined : this.profile(profile);
    const profileGeneration = scope?.generation;
    const signal = (scope?.controller ?? this.controller).signal;
    return Object.freeze({
      session: this.session,
      generation,
      ...(profileGeneration === undefined ? {} : { profileGeneration }),
      signal,
      isCurrent: () =>
        !this.disposed &&
        !signal.aborted &&
        generation === this.revision &&
        (profile === undefined ||
          (this.profiles.get(profile) === scope &&
            scope?.generation === profileGeneration)),
    });
  }

  readonly subscribe = (
    listener: (event: AccessRetirement) => void,
  ): (() => void) => {
    if (this.disposed) return () => undefined;
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  retire(
    reason: Exclude<AccessRetirementReason, 'dispose'>,
    profile?: string,
  ): void {
    if (this.disposed) return;
    if (profile === undefined) {
      this.revision++;
      this.sessionIdentity = Object.freeze({});
      const previous = this.controller;
      this.controller = new AbortController();
      const scopes = [...this.profiles.values()];
      this.profiles.clear();
      previous.abort();
      for (const scope of scopes) scope.controller.abort();
    } else {
      const scope = this.profile(profile);
      const previous = scope.controller;
      scope.generation++;
      scope.controller = new AbortController();
      previous.abort();
    }
    this.publish(
      Object.freeze({
        reason,
        generation: this.revision,
        ...(profile === undefined ? {} : { profile }),
      }),
    );
  }

  retainProfiles(profiles: readonly string[]): void {
    const retained = new Set(profiles);
    for (const profile of [...this.profiles.keys()]) {
      if (retained.has(profile)) continue;
      this.retire('scope-removed', profile);
      this.profiles.delete(profile);
    }
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.controller.abort();
    for (const scope of this.profiles.values()) scope.controller.abort();
    this.profiles.clear();
    this.publish(
      Object.freeze({ reason: 'dispose', generation: this.revision }),
    );
    this.listeners.clear();
  }

  private profile(name: string): ProfileLifetime {
    let scope = this.profiles.get(name);
    if (!scope) {
      scope = { generation: 0, controller: new AbortController() };
      if (this.disposed) scope.controller.abort();
      else this.profiles.set(name, scope);
    }
    return scope;
  }

  private publish(event: AccessRetirement): void {
    this.version++;
    for (const listener of [...this.listeners]) {
      try {
        listener(event);
      } catch {
        continue;
      }
    }
  }
}
