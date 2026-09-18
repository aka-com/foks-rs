import { AccessLifetime } from '../../app/access-lifetime';
import { enqueueProfileWork } from '../../bridge';
import type { Bridge, ResetPreview } from '../../bridge';
import { attemptMutation, attemptRead } from '../../commands/command-policy';

export interface ResetWorkflowState {
  preview: ResetPreview | null;
  loading: boolean;
  error: string | null;
  available: boolean;
  busy: boolean;
}

const emptyState = (): ResetWorkflowState => ({
  preview: null,
  loading: false,
  error: null,
  available: false,
  busy: false,
});

export class ResetWorkflow {
  private readonly lifetime = new AccessLifetime();
  private live = false;
  private token: string | null = null;
  private state = emptyState();
  private listeners = new Set<() => void>();

  constructor(
    private readonly bridge: Bridge,
    private readonly profile: string,
    private readonly ownsBinding: () => boolean,
    private readonly onPreviewError: (error: unknown) => void,
    private readonly onMutationError: (error: unknown) => void,
  ) {}

  readonly getSnapshot = (): ResetWorkflowState => this.state;
  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  private publish(state: ResetWorkflowState): void {
    this.state = state;
    for (const listener of this.listeners) listener();
  }

  activate(): void {
    this.live = true;
  }

  retire(): void {
    this.live = false;
    this.lifetime.retire('access-change');
    this.token = null;
    this.publish(emptyState());
  }

  readonly load = async (): Promise<void> => {
    if (!this.live || !this.ownsBinding() || this.state.busy) return;
    this.lifetime.retire('access-change');
    const ticket = this.lifetime.capture(this.profile);
    const isCurrent = () =>
      this.live && this.ownsBinding() && ticket.isCurrent();
    this.token = null;
    this.publish({ ...emptyState(), loading: true });
    const result = await attemptRead({ kind: 'read-recovery' }, () =>
      enqueueProfileWork(this.bridge, this.profile, async () => {
        if (!isCurrent()) return undefined;
        return this.bridge.describeReset(this.profile);
      }),
    );
    if (!isCurrent()) return;
    if (result.outcome === 'read-failed') {
      this.publish({ ...emptyState(), error: result.error.message });
      this.onPreviewError(result.error);
      return;
    }
    const preview = result.value;
    if (!preview) return;
    if (preview.profile !== this.profile) {
      const error = new Error('describe_reset returned a different profile.');
      this.publish({ ...emptyState(), error: error.message });
      this.onPreviewError(error);
      return;
    }
    this.token = preview.token;
    this.publish({ ...emptyState(), preview, available: Boolean(this.token) });
  };

  async reset(
    confirmation: string,
    onReset: () => Promise<void>,
  ): Promise<void> {
    if (
      !this.live ||
      !this.ownsBinding() ||
      this.state.busy ||
      confirmation !== this.profile ||
      !this.token
    )
      return;
    const once = this.token;
    this.token = null;
    const ticket = this.lifetime.capture(this.profile);
    const isCurrent = () =>
      this.live && this.ownsBinding() && ticket.isCurrent();
    this.publish({ ...this.state, available: false, busy: true });
    let observed = false;
    try {
      const outcome = await attemptMutation(
        { kind: 'mutation' },
        () => this.bridge.resetServer(this.profile, confirmation, once),
        async () => {
          observed = isCurrent();
          if (observed) await onReset();
        },
      );
      if (!isCurrent() && !(observed && outcome.outcome === 'applied')) return;
      if (
        outcome.outcome !== 'applied' ||
        outcome.synchronization === 'pending'
      )
        this.onMutationError(outcome.error);
    } finally {
      if (isCurrent()) this.publish({ ...this.state, busy: false });
    }
  }
}
