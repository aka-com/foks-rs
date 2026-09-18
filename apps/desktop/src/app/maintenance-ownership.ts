export class MaintenanceOwner {
  private state: 'active' | 'handed-off' | 'retired' = 'active';
  private cleanup: (() => void) | undefined;

  constructor(private readonly release: () => void) {}

  isCurrent(): boolean {
    return this.state === 'active';
  }

  get handedOff(): boolean {
    return this.state === 'handed-off';
  }

  install(unlisten: () => void): void {
    if (!this.isCurrent()) {
      unlisten();
      return;
    }
    this.stopListening();
    this.cleanup = unlisten;
  }

  handoff(current: () => boolean): boolean {
    if (this.state === 'retired' || !current()) return false;
    if (this.state === 'active') {
      this.state = 'handed-off';
      this.release();
      this.stopListening();
    }
    return this.state === 'handed-off' && current();
  }

  retire(): void {
    if (this.state === 'retired') return;
    this.state = 'retired';
    this.release();
    this.stopListening();
  }

  private stopListening(): void {
    const cleanup = this.cleanup;
    this.cleanup = undefined;
    cleanup?.();
  }
}

export class MaintenanceOwnership {
  private owner: MaintenanceOwner | null = null;

  acquire(): MaintenanceOwner {
    this.owner?.retire();
    const owner = new MaintenanceOwner(() => {
      if (this.owner === owner) this.owner = null;
    });
    this.owner = owner;
    return owner;
  }
}
