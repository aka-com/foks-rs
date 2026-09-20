import type { Bridge } from '../bridge';
import { normalizeCommandError } from '../bridge';
import type { ChatMessage } from '../chat-contract';
import { chatClient, channelIntegrity, integrity } from './client';
import { sameScope, channelWorkKey } from './scope';
import {
  systemChatClock,
  type ChatClock,
  type ChatInboxService,
} from './inbox-service';
import type { LocalSession } from './local-contract';
import { notificationKey } from './local-contract';
import { incoming, notificationText } from './notification-policy';
import { messageVisible } from './visibility';
import {
  classifyNotificationChange,
  notificationView,
  type NotificationView,
} from './notification-changes';
import { notificationAdmission } from './notification-admission';

export const NOTIFICATION_LIMITS = { queued: 64, baselines: 4096 } as const;
/**
 * The base wait between fallback passes, and the ceiling that wait doubles
 * toward for a channel whose passes keep finding nothing.
 *
 * The fallback exists for a projection that cannot say which thread moved: a
 * degraded host, or a channel whose baseline is not established yet. It is
 * the whole cost of a degraded host — twenty channels at the base interval is
 * four full history reads a second, sustained — so a channel that keeps
 * finding nothing doubles its wait, and any pass that finds a newer sequence
 * puts it straight back to the base.
 *
 * **The channel whose thread is open never backs off.** It keeps the base
 * interval however long it has been quiet, because it is the one channel
 * whose staleness the user is looking at.
 *
 * **What this trades is alert latency on a degraded host, deliberately.** A
 * message in a quiet channel that is not open is alerted up to
 * `FALLBACK_MAX_MS` after it arrives rather than up to `FALLBACK_MS`. No
 * message is skipped: the wait decides when the read happens, not whether,
 * and the read that finds it resets the channel to the base interval.
 */
const FALLBACK_MS = 5000;
const FALLBACK_MAX_MS = 60_000;
/**
 * How long to wait before retrying a channel that is due but could not be
 * started, because a profile admission was refused or this pass's row
 * allowance was spent. Every other wake-up is armed for a deadline the scan
 * computed, so this is the only fixed interval left.
 */
const PENDING_REARM_MS = 1000;
const CHANNEL_ROWS = 100,
  CHANNEL_BYTES = 1024 * 1024,
  PASS_ROWS = 400,
  PASS_BYTES = 4 * 1024 * 1024;
type Progress = {
  id: string;
  channel: string;
  key: string;
  generation: number;
  view: NotificationView;
  processed: number;
  baseline: bigint | null;
  baselineOnly: boolean;
  upper?: bigint;
  dirty: boolean;
  dirtyDuringFlight: boolean;
  overflow: boolean;
  due: number;
  /** When the last pass that could have observed an arrival finished. */
  fallbackAt: number;
  /** This channel's current wait, doubled per pass that found nothing. */
  fallbackInterval: number;
  failures: number;
  preference?: string;
  locallyDisabled?: boolean;
  deniedAt?: number;
};
type Job = {
  client: ReturnType<typeof chatClient>;
  controller: AbortController;
  release(): void;
};
export type NotificationMetric =
  | { kind: 'state'; queued: number; baselines: number; active: number }
  | {
      kind: 'pass';
      candidates: readonly string[];
      baselineOnly: boolean;
      incomplete: boolean;
      rows: number;
      bytes: number;
      /** From the pass's start to its history reads' end. */
      milliseconds?: number;
      /** This summary ends a pass whose remaining allowance cannot admit a channel. */
      budgetHit?: boolean;
    }
  | { kind: 'failure'; cancelled: boolean; busy: boolean; retry: number }
  | { kind: 'eviction' };

/** Metadata-only dirty work derived from the shell's immutable verified projection. */
export class NotificationConsumer {
  private progress = new Map<string, Progress>();
  private queued = new Map<string, Progress>();
  private jobs = new Map<string, Job>();
  private admission;
  private stopped = false;
  private timer: unknown;
  private timerDue = Infinity;
  private off: () => void;
  private offAdmission: () => void;
  private cursor = 0;
  private generation = 0;
  private evictionWindow = 0;
  private evictions = 0;
  private pass = { rows: 0, bytes: 0 };
  private nextPass = 0;
  constructor(
    private bridge: Bridge,
    private service: ChatInboxService,
    private session: LocalSession,
    private report: (message: string) => void,
    private clock: ChatClock = systemChatClock,
    private observe?: (event: NotificationMetric) => void,
    private limits: { queued: number; baselines: number } = NOTIFICATION_LIMITS,
  ) {
    this.admission = notificationAdmission(bridge);
    this.off = service.subscribe(() => this.kick());
    this.offAdmission = this.admission.subscribe(() => this.kick());
    this.kick();
  }
  private metric(event: NotificationMetric) {
    try {
      this.observe?.(event);
    } catch {
      /* diagnostics are not authority */
    }
  }
  stop() {
    if (this.stopped) return;
    this.stopped = true;
    this.generation++;
    this.off();
    this.offAdmission();
    this.clock.cancel(this.timer);
    this.timer = undefined;
    for (const job of this.jobs.values()) {
      job.controller.abort();
      job.client.dispose();
    }
    this.queued.clear();
    this.progress.clear();
    // Active permits survive until the native promises actually settle.
  }
  private kick(delay = 0) {
    if (this.stopped) return;
    const due = Math.max(this.clock.now() + delay, this.nextPass);
    if (this.timer !== undefined && this.timerDue <= due) return;
    this.clock.cancel(this.timer);
    this.timerDue = due;
    this.timer = this.clock.later(() => {
      this.timer = undefined;
      this.timerDue = Infinity;
      this.scan();
    }, due - this.clock.now());
  }
  private discard(p: Progress) {
    this.progress.delete(p.key);
    this.queued.delete(p.key);
    const job = this.jobs.get(p.key);
    job?.controller.abort();
    job?.client.dispose();
  }
  private evictionCandidate(): Progress | undefined {
    for (const record of this.progress.values())
      if (!this.jobs.has(record.key) && !this.queued.has(record.key))
        return record;
  }
  private current(p: Progress): boolean {
    if (
      this.stopped ||
      !this.session.available ||
      !this.session.settings.enabled ||
      this.progress.get(p.key) !== p
    )
      return false;
    if (p.preference && this.session.settings.overrides[p.preference] === false)
      return false;
    const next = notificationView(
      this.service.getSnapshot().get(p.id),
      p.channel,
    );
    return (
      !!next &&
      next.fresh &&
      next.eligible &&
      sameScope(next.scope, p.view.scope) &&
      next.readRole === p.view.readRole &&
      next.admin === p.view.admin
    );
  }
  /** Whether this channel's thread is the one presented for its store. */
  private presented(p: Progress): boolean {
    return this.service.openChannel(p.id) === p.channel;
  }
  /**
   * When this channel's next fallback pass is due. Read rather than stored,
   * so opening a channel that had backed off makes it due at the base
   * interval from its last pass immediately, without waiting the interval it
   * had reached.
   */
  private fallbackDue(p: Progress): number {
    return (
      p.fallbackAt + (this.presented(p) ? FALLBACK_MS : p.fallbackInterval)
    );
  }
  private scan() {
    if (this.stopped) return;
    const snapshot = this.service.getSnapshot(),
      now = this.clock.now();
    let invalidated = false;
    for (const p of this.progress.values()) {
      const next = notificationView(
        snapshot.get(p.id),
        p.channel,
        !p.preference ||
          this.session.settings.overrides[p.preference] !== false,
      );
      const change = classifyNotificationChange(p.view, next);
      if (
        !this.session.available ||
        !this.session.settings.enabled ||
        change.remove ||
        change.baseline
      ) {
        this.discard(p);
        invalidated = true;
        continue;
      }
      const disabled =
        !!p.preference &&
        this.session.settings.overrides[p.preference] === false;
      if (disabled) {
        invalidated ||= !p.locallyDisabled;
        p.locallyDisabled = true;
        p.baseline = null;
        p.baselineOnly = true;
        p.dirty = false;
        this.queued.delete(p.key);
        this.jobs.get(p.key)?.controller.abort();
        p.view = next!;
        continue;
      }
      if (p.locallyDisabled) {
        p.locallyDisabled = false;
        p.dirty = true;
      }
      if (p.deniedAt !== undefined) {
        if (next!.authorization <= p.deniedAt) {
          p.view = next!;
          continue;
        }
        p.deniedAt = undefined;
        p.dirty = true;
      }
      if (change.content || change.fallback) {
        p.dirty = true;
        p.dirtyDuringFlight ||= this.jobs.has(p.key);
      }
      p.view = next!;
      if ((p.view.degraded || p.baselineOnly) && this.fallbackDue(p) <= now)
        p.dirty = true;
    }
    if (invalidated)
      void this.bridge
        .chatLocal({ action: 'clear', epoch: this.session.epoch })
        .catch(() => {
          if (!this.stopped) this.report('Could not clear desktop alerts.');
        });
    if (!this.session.available || !this.session.settings.enabled) return;

    const size = [...snapshot.values()].reduce(
      (n, e) => n + (e.data?.channels.length ?? 0),
      0,
    );
    const start = size ? this.cursor % size : 0;
    // Whether an eligible channel was passed over because the baseline bound
    // or this pass's queue was full. Rotation is the only work that makes
    // progress on its own, so it is the only reason left to arm a timer for
    // a set of channels that are all otherwise idle.
    let deferred = false;
    // Walk immutable metadata in two segments without allocating a channel array.
    // Overflow never allocates a promise and never restarts at map position zero.
    for (
      let segment = 0;
      segment < 2 && this.queued.size < this.limits.queued;
      segment++
    ) {
      let index = 0;
      for (const [id, entry] of snapshot) {
        for (const channel of entry.data?.channels ?? []) {
          const position = index++;
          if (segment === 0 ? position < start : position >= start) continue;
          if (this.queued.size >= this.limits.queued) {
            deferred = true;
            break;
          }
          const view = notificationView(entry, channel.id);
          if (!view?.eligible || !view.fresh) continue;
          const key = channelWorkKey(id, view.scope, channel.id);
          if (this.jobs.has(key) || this.queued.has(key)) continue;
          let p = this.progress.get(key);
          if (p && (!p.dirty || p.due > now)) continue;
          if (!p) {
            if (this.progress.size >= this.limits.baselines) {
              // Best-effort rotation must not become an endless baseline RPC loop.
              if (now >= this.evictionWindow) {
                this.evictionWindow = now + FALLBACK_MS;
                this.evictions = 0;
              }
              if (this.evictions >= this.limits.queued) {
                deferred = true;
                continue;
              }
              const old = this.evictionCandidate();

              if (!old) {
                deferred = true;
                continue;
              }
              this.discard(old);
              this.evictions++;
              this.metric({ kind: 'eviction' });
            }
            p = {
              id,
              channel: channel.id,
              key,
              view,
              generation: ++this.generation,
              processed: -1,
              baseline: null,
              baselineOnly: true,
              dirty: true,
              dirtyDuringFlight: false,
              overflow: false,
              due: 0,
              fallbackAt: now,
              fallbackInterval: FALLBACK_MS,
              failures: 0,
            };
            this.progress.set(key, p);
          }
          p.overflow = false;
          this.queued.set(key, p);
          this.cursor = position + 1;
        }
      }
    }
    for (const p of this.progress.values())
      p.overflow = p.dirty && !this.queued.has(p.key) && !this.jobs.has(p.key);
    let budgetHit = false;
    for (const [key, p] of this.queued) {
      if (!this.current(p)) {
        this.discard(p);
        continue;
      }
      // Reserve a complete channel read before admission. Concurrent profiles
      // cannot overshoot the pass, and a baseline never commits half a read.
      if (
        this.pass.rows + (this.jobs.size + 1) * CHANNEL_ROWS > PASS_ROWS ||
        this.pass.bytes + (this.jobs.size + 1) * CHANNEL_BYTES > PASS_BYTES
      ) {
        budgetHit = true;
        break;
      }
      const release = this.admission.acquire(p.view.scope.store.profile);
      if (!release) continue;
      this.queued.delete(key);
      const client = chatClient(this.bridge, p.view.scope.store.profile, p.id);
      const job = { client, controller: new AbortController(), release };
      this.jobs.set(key, job);
      void this.run(p, job).finally(() => {
        client.dispose();
        if (this.jobs.get(key) === job) this.jobs.delete(key);
        p.upper = undefined;
        release();
        this.kick();
      });
    }
    if (budgetHit && this.jobs.size === 0) {
      this.metric({
        kind: 'pass',
        candidates: [],
        baselineOnly: true,
        incomplete: false,
        ...this.pass,
        budgetHit: true,
      });
      // Retain Map insertion order: leftovers precede newly dirtied channels.
      // Yield a timer turn even if an admission or inbox event arrives now.
      for (const p of this.queued.values()) {
        p.dirty = true;
        p.due = 0;
      }
      this.pass = { rows: 0, bytes: 0 };
      this.nextPass = now + 1;
      this.kick();
    } else if (this.jobs.size === 0 && this.queued.size === 0) {
      this.pass = { rows: 0, bytes: 0 };
    }
    this.metric({
      kind: 'state',
      queued: this.queued.size,
      baselines: this.progress.size,
      active: this.admission.active,
    });
    // Arm for the earliest deadline this pass computed rather than for a
    // fixed floor: a channel with nothing due states `Infinity`, and a set of
    // channels that all do arms no timer at all. The consumer then wakes on
    // an inbox publication or an admission release, which is what actually
    // changes its work. A channel whose deadline has already passed is
    // waiting for an admission or for this pass's allowance, not for time, so
    // it re-arms at the retry floor instead of at zero.
    let delay = Infinity;
    for (const p of this.progress.values()) {
      if (p.locallyDisabled || p.deniedAt !== undefined || this.jobs.has(p.key))
        continue;
      const deadline = p.dirty
        ? p.due
        : p.view.degraded || p.baselineOnly
          ? this.fallbackDue(p)
          : Infinity;
      delay = Math.min(
        delay,
        deadline > now ? deadline - now : PENDING_REARM_MS,
      );
    }
    if (deferred)
      delay = Math.min(
        delay,
        Math.max(PENDING_REARM_MS, this.evictionWindow - now),
      );
    if (delay !== Infinity) this.kick(delay);
  }
  private async run(p: Progress, job: Job) {
    const revision = p.view.revision,
      generation = p.generation;
    const started = this.clock.now();
    p.dirtyDuringFlight = false;
    const current = () =>
      this.current(p) &&
      p.generation === generation &&
      !job.controller.signal.aborted;
    const read = async (before: string | null) => {
      const reply = await job.client.request(
        { action: 'notification-history', channel: p.channel, before },
        {
          key: p.key,
          owner: job,
          generation,
          signal: job.controller.signal,
          current,
          cancel: () => job.client.dispose(),
          preemptible: false,
        },
      );
      if (!sameScope(reply.scope, p.view.scope)) throw integrity();
      // Native enforcement precedes decryption; reject malformed fake/adapter replies too.
      if (
        reply.result.messages.length > 50 ||
        reply.result.messages.some(
          (m) =>
            m.content.kind === 'text' &&
            Array.from(m.content.text).length > 256,
        )
      )
        throw channelIntegrity('Invalid notification history bound.');
      // Count every returned row, including baseline-only and repeated pages.
      this.pass.rows += reply.result.messages.length;
      this.pass.bytes += reply.result.messages.reduce(
        (sum, m) =>
          sum +
          (m.content.kind === 'text'
            ? new TextEncoder().encode(m.content.text).length
            : 0),
        0,
      );
      return reply.result;
    };
    const commit = (upper: bigint) => {
      // A pass that reached a newer sequence is evidence that this channel
      // still receives, so its wait returns to the base; a pass that reached
      // nothing new doubles it, up to the ceiling. The channel whose thread
      // is open is held at the base by `fallbackDue`, whatever is stored here.
      const observed = p.baseline === null || upper > p.baseline;
      p.baseline = upper;
      p.baselineOnly = false;
      p.processed = revision;
      p.failures = 0;
      p.due = 0;
      p.fallbackAt = this.clock.now();
      p.fallbackInterval = observed
        ? FALLBACK_MS
        : Math.min(p.fallbackInterval * 2, FALLBACK_MAX_MS);
      p.dirty = p.dirtyDuringFlight || p.view.revision !== revision;
    };
    try {
      p.preference = await notificationKey(p.view.scope, p.channel);
      if (!current()) return;
      // A baseline is the sequence this channel stood at when watching began,
      // and a full history read establishes it as the newest sequence the
      // first page carries. The projection already states that number: the
      // conversation row's own head. Seeding from it costs no round trip, and
      // it cannot sit above what the read would have established, because the
      // row is derived from a message the channel holds and lags the server
      // rather than leading it. Seeding above the true position is the one
      // failure here that reports nothing — those messages would simply never
      // alert — so a projection that states no position, and a degraded one
      // whose rows stopped advancing while messages kept arriving, still read.
      if (p.baseline === null && !p.view.degraded && p.view.position !== null) {
        commit(p.view.position);
        this.metric({
          kind: 'pass',
          milliseconds: this.clock.now() - started,
          candidates: [],
          baselineOnly: true,
          incomplete: false,
          rows: 0,
          bytes: 0,
        });
        return;
      }
      const first = await read(null);
      if (!current()) return;
      const upper = first.messages.reduce(
        (max, m) => (BigInt(m.sequence) > max ? BigInt(m.sequence) : max),
        0n,
      );
      p.upper = upper;
      if (p.baseline !== null && upper < p.baseline) {
        // An omitted/stale page is not permission to replay already observed
        // history. Retain the known floor and rebaseline on a bounded retry.
        commit(p.baseline);
        p.baselineOnly = true;
        // Rebaselining after a page the server could not serve in full is a
        // recovery, not a quiet channel: the geometric wait is for a channel
        // that keeps finding nothing, and this one found something wrong.
        p.fallbackInterval = FALLBACK_MS;
        this.metric({
          kind: 'pass',
          milliseconds: this.clock.now() - started,
          candidates: [],
          baselineOnly: true,
          incomplete: true,
          rows: first.messages.length,
          bytes: 0,
        });
        return;
      }
      if (p.baseline === null || p.baselineOnly) {
        commit(upper);
        this.metric({
          kind: 'pass',
          milliseconds: this.clock.now() - started,
          candidates: [],
          baselineOnly: true,
          incomplete: p.view.degraded || first.missing_predecessors.length > 0,
          rows: first.messages.length,
          bytes: 0,
        });
        return;
      }
      const baseline = p.baseline;
      let rows = first.messages,
        before = first.before;
      let incomplete = p.view.degraded || first.missing_predecessors.length > 0;
      if (
        before &&
        rows.length < CHANNEL_ROWS &&
        rows.every((m) => BigInt(m.sequence) > baseline)
      ) {
        const next = await read(before);
        if (!current()) return;
        rows = [...rows, ...next.messages];
        before = next.before;
        incomplete ||= next.missing_predecessors.length > 0;
      }
      incomplete ||=
        rows.length > CHANNEL_ROWS ||
        (!!before && rows.every((m) => BigInt(m.sequence) > baseline));
      let bytes = 0;
      const bounded: ChatMessage[] = [];
      for (const m of rows.slice(0, CHANNEL_ROWS)) {
        const size =
          m.content.kind === 'text'
            ? new TextEncoder().encode(m.content.text).length
            : 0;
        if (bytes + size > CHANNEL_BYTES) {
          incomplete = true;
          break;
        }
        bytes += size;
        bounded.push(m);
      }
      const candidates = incoming(
        bounded,
        baseline,
        upper,
        p.view.scope.actor,
        (id) => messageVisible(p.id, p.channel, id),
      );
      if (!current()) return;
      commit(upper);
      this.metric({
        kind: 'pass',
        milliseconds: this.clock.now() - started,
        candidates: candidates.map((m) => m.id),
        baselineOnly: false,
        incomplete,
        rows: bounded.length,
        bytes,
      });
      for (const alert of notificationText(candidates, incomplete)) {
        if (!current()) return;
        try {
          await this.bridge.chatLocal({
            action: 'display',
            epoch: this.session.epoch,
            storeId: p.id,
            scope: p.view.scope,
            channel: p.channel,
            ...alert,
            body: this.session.settings.previews ? alert.body : undefined,
          });
        } catch {
          if (!this.stopped)
            this.report(
              'Desktop alert could not be delivered. Chat is still connected.',
            );
        }
      }
    } catch (cause) {
      if (!current()) return;
      const error = normalizeCommandError(cause);
      if (error.code === 'cancelled') {
        this.metric({
          kind: 'failure',
          cancelled: true,
          busy: false,
          retry: p.failures,
        });
        return;
      }
      p.dirty = true;
      if (error.code === 'chat-channel-integrity') {
        this.service.blockChannel(p.id, p.channel);
        this.discard(p);
      } else if (error.code === 'chat-access-denied') {
        p.baseline = null;
        p.baselineOnly = true;
        p.dirty = false;
        p.deniedAt = p.view.authorization;
        this.queued.delete(p.key);
        void this.bridge
          .chatLocal({ action: 'clear', epoch: this.session.epoch })
          .catch(() => {});
        this.service.invalidate(p.id);
      } else if (this.service.handleError(p.id, error, p.channel)) {
        this.discard(p);
      } else {
        p.failures = Math.min(p.failures + 1, 6);
        p.due =
          this.clock.now() + Math.min(1000 * 2 ** (p.failures - 1), 30000);
      }
      this.metric({
        kind: 'failure',
        cancelled: false,
        busy: error.code === 'profile-busy',
        retry: p.failures,
      });
    }
  }
}
