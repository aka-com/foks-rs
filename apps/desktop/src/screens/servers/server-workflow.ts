import { AccessLifetime } from '../../app/access-lifetime';
import { enqueueProfileWork, sharedServerStatus } from '../../bridge';
import type { Bridge, CheckedServer, ServerStatusSnapshot } from '../../bridge';
import { attemptMutation } from '../../commands/command-policy';
import { serverAvailability } from '../../model';
import type { AgentSnapshot, Server } from '../../model';

export const serverBinding = (server: Server): string =>
  JSON.stringify([server.id, server.configuredProbe, server.host_id]);

export function canReadServer(server: Server): boolean {
  return (
    server.trust.status !== 'blocked' &&
    !server.restrictions.some((row) => row.kind === 'schema-incompatible')
  );
}

function canCheckServer(snapshot: AgentSnapshot, server: Server): boolean {
  const availability = serverAvailability(snapshot, server);
  return (
    canReadServer(server) &&
    (availability.available || availability.reason !== 'security-state-missing')
  );
}

const acceptanceText = (value: CheckedServer['acceptance']): string =>
  value === 'inserted'
    ? 'Server identity pinned'
    : value === 'advanced'
      ? 'Server verification updated'
      : 'Server verification unchanged';

export async function readCurrentServerStatus(
  bridge: Bridge,
  server: Server,
  isCurrent: () => boolean,
): Promise<ServerStatusSnapshot | undefined> {
  const admitted = await enqueueProfileWork(bridge, server.id, async () =>
    isCurrent(),
  );
  if (!admitted || !isCurrent()) return;
  const status = await sharedServerStatus(bridge, server.id);
  if (!isCurrent()) return;
  if (status.profile !== server.id)
    throw new Error('describe_server_status returned a different profile.');
  return status;
}

interface CheckContext {
  snapshot: AgentSnapshot;
  profile?: string;
}

interface CheckObserver {
  busy: (busy: boolean) => void;
  checked: (binding: string, report: CheckedServer) => void;
  status: (binding: string, status: ServerStatusSnapshot) => void;
  toast: (message: string) => void;
  refresh: (message: string) => Promise<void>;
  error: (error: unknown) => void | Promise<void>;
}

export class ServerCheckController {
  private readonly lifetime = new AccessLifetime();
  private active: object | null = null;
  private live = true;

  constructor(
    private readonly bridge: Bridge,
    private readonly context: () => CheckContext,
    private readonly observer: CheckObserver,
  ) {}

  activate(): void {
    this.live = true;
    this.observer.busy(false);
  }

  retire(): void {
    this.live = false;
    this.lifetime.retire('access-change');
    this.active = null;
  }

  async check(server: Server, seeded?: () => void): Promise<void> {
    const context = this.context();
    // Prevent duplicate checks from rapid key events and ignore blocked servers.
    if (
      !this.live ||
      this.active ||
      (seeded && !this.bridge.fixtureSnapshot) ||
      !canCheckServer(context.snapshot, server)
    )
      return;
    const ticket = this.lifetime.capture(server.id);
    const binding = serverBinding(server);
    const attempt = {};
    const isCurrent = (): boolean => {
      const next = this.context();
      const selected = next.snapshot.servers.find(
        (row) => row.id === server.id,
      );
      return (
        this.live &&
        ticket.isCurrent() &&
        this.active === attempt &&
        context.profile === next.profile &&
        selected !== undefined &&
        serverBinding(selected) === binding
      );
    };
    this.active = attempt;
    this.observer.busy(true);
    try {
      const outcome = await attemptMutation(
        { kind: 'mutation' },
        () =>
          enqueueProfileWork(this.bridge, server.id, async () => {
            if (!isCurrent()) return undefined;
            const latest = this.context().snapshot;
            const currentServer = latest.servers.find(
              (row) => row.id === server.id,
            );
            if (!currentServer || !canCheckServer(latest, currentServer))
              return;
            seeded?.();
            return this.bridge.checkServer(server.id);
          }),
        async (report) => {
          if (!report || !isCurrent()) return;
          if (report.profile !== server.id)
            throw new Error('check_server returned a different profile.');
          this.observer.checked(binding, report);
          if (!seeded)
            this.observer.toast(
              `Checked ${report.canonicalName}, ${acceptanceText(report.acceptance)}`,
            );
          const passive = await readCurrentServerStatus(
            this.bridge,
            server,
            isCurrent,
          );
          if (!passive || !isCurrent()) return;
          this.observer.status(binding, passive);
          if (!seeded)
            await this.observer.refresh(
              `Checked ${report.canonicalName}; refreshed signed server status`,
            );
        },
      );
      if (!isCurrent()) return;
      if (
        outcome.outcome !== 'applied' ||
        outcome.synchronization === 'pending'
      )
        await this.observer.error(outcome.error);
    } finally {
      if (isCurrent()) this.observer.busy(false);
      if (this.active === attempt) this.active = null;
    }
  }
}
