import type { ChatClock } from '../clock';
import { DeadlineTimer } from '../deadline-timer';
import { awaitingAccess, intentHeld, running } from './outgoing';
import type { Team } from './team';

/**
 * How long the tick waits while a team has work in hand. The tick itself
 * issues nothing; it lets each team's own gate through.
 */
export const ACTIVE_TICK = 1000;
/**
 * How long it waits when no team has anything for it to do. The recovery it
 * would run issues `Chat/Pending` and `Chat/CleanupPending` for every team,
 * so an idle tick is pure cost; at this cadence the service still notices
 * operations another session left behind.
 */
export const IDLE_TICK = 30_000;
/** How long a team's gate stays closed after a background pass starts. */
const TEAM_INTERVAL = 2000;
/** How long it stays closed after a background pass fails. */
const TEAM_RETRY = 5000;
/** How many profiles may run a background pass at once. */
const CONCURRENT_PROFILES = 4;

/**
 * Whether the tick would do anything for this team: a recovery never run or
 * in flight, jobs the last recovery left, work this session started since
 * it, or, per `unopened`, a readable channel not opened yet. An operation
 * the recovery has nothing left to do about is not work.
 */
export function teamHasWork(team: Team, unopened: () => boolean): boolean {
  return (
    !team.recovered ||
    team.refresh !== undefined ||
    team.jobs ||
    team.messages.some(
      (m) =>
        running(m) ||
        m.delivery.stage === 'queued' ||
        intentHeld(m) ||
        (m.automatic && awaitingAccess(m)),
    ) ||
    team.operations.some(
      (op) => op.statusUnknown || op.state === 'uncertain',
    ) ||
    unopened()
  );
}

/**
 * When the next tick could act: the soonest gate of a team with work, but
 * no sooner than {@link ACTIVE_TICK}, or the idle wait.
 */
export function nextTickDelay(
  teams: Iterable<Team>,
  now: number,
  outstanding: (team: Team) => boolean,
): number {
  let soonest = Infinity;
  for (const team of teams)
    if (outstanding(team))
      soonest = Math.min(soonest, Math.max(0, team.due - now));
  return Math.min(IDLE_TICK, Math.max(ACTIVE_TICK, soonest));
}

export interface SchedulerHost {
  /** The service is started. */
  active(): boolean;
  teams(): Iterable<Team>;
  /** Brings the teams up to date with the agent and inbox snapshots. */
  synchronize(): void;
  /** The team is current and {@link teamHasWork}. */
  outstanding(team: Team): boolean;
  current(team: Team): boolean;
  /** One background pass over the team: open channels, then recover. */
  run(team: Team): Promise<void>;
}

/**
 * The send service's tick, on the service's clock. Each tick starts a
 * background pass for every current team whose gate has opened, at most one
 * per profile and {@link CONCURRENT_PROFILES} at once, visiting the teams
 * round-robin so that no team waits behind the same others every time.
 */
export class SendScheduler {
  private timer: DeadlineTimer;
  private cursor = 0;
  /** The profiles with a background pass in flight. */
  private profiles = new Set<string>();

  constructor(
    private clock: ChatClock,
    private host: SchedulerHost,
  ) {
    this.timer = new DeadlineTimer(clock, () => this.tick());
  }

  /**
   * Arms the tick. A re-arm can only bring it forward, so a burst of
   * publications cannot push it out indefinitely; `reset` replaces the
   * deadline, and the tick itself resets it.
   */
  arm(reset = false) {
    if (!this.host.active()) {
      this.timer.cancel();
      return;
    }
    const now = this.clock.now();
    const due =
      now +
      nextTickDelay(this.host.teams(), now, (team) =>
        this.host.outstanding(team),
      );
    if (reset) this.timer.reset(due);
    else this.timer.armAt(due);
  }
  cancel() {
    this.timer.cancel();
  }
  /**
   * Forgets the passes in flight, for a service that dropped its teams. A
   * pass that ends afterwards releases its profile from the set it started
   * under, not from this one.
   */
  forget() {
    this.profiles = new Set();
  }

  private tick() {
    if (!this.host.active()) return;
    this.host.synchronize();
    const teams = [...this.host.teams()];
    for (
      let count = 0;
      count < teams.length && this.profiles.size < CONCURRENT_PROFILES;
      count++
    ) {
      const team = teams[this.cursor++ % teams.length];
      if (
        !this.host.current(team) ||
        team.due > this.clock.now() ||
        this.profiles.has(team.profile)
      )
        continue;
      const profiles = this.profiles;
      profiles.add(team.profile);
      team.due = this.clock.now() + TEAM_INTERVAL;
      void this.host
        .run(team)
        .catch(() => {
          team.due = this.clock.now() + TEAM_RETRY;
        })
        .finally(() => {
          profiles.delete(team.profile);
          // The recovery is what settles whether the team still has work, so
          // its end is where the next tick's distance is decided.
          this.arm(true);
        })
        .catch(() => {});
    }
    this.arm(true);
  }
}
