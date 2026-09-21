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

const QUEUED = 64,
  BASELINES = 4096,
  FALLBACK_MS = 5000;
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
  fallbackDue: number;
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
  constructor(
    private bridge: Bridge,
    private service: ChatInboxService,
    private session: LocalSession,
    private report: (message: string) => void,
    private clock: ChatClock = systemChatClock,
    private observe?: (event: NotificationMetric) => void,
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
    const due = this.clock.now() + delay;
    if (this.timer !== undefined && this.timerDue <= due) return;
    this.clock.cancel(this.timer);
    this.timerDue = due;
    this.timer = this.clock.later(() => {
      this.timer = undefined;
      this.timerDue = Infinity;
      this.scan();
    }, delay);
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
      if ((p.view.degraded || p.baselineOnly) && p.fallbackDue <= now)
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
    // Walk immutable metadata in two segments without allocating a channel array.
    // Overflow never allocates a promise and never restarts at map position zero.
    for (let segment = 0; segment < 2 && this.queued.size < QUEUED; segment++) {
      let index = 0;
      for (const [id, entry] of snapshot) {
        for (const channel of entry.data?.channels ?? []) {
          const position = index++;
          if (segment === 0 ? position < start : position >= start) continue;
          if (this.queued.size >= QUEUED) break;
          const view = notificationView(entry, channel.id);
          if (!view?.eligible || !view.fresh) continue;
          const key = channelWorkKey(id, view.scope, channel.id);
          if (this.jobs.has(key) || this.queued.has(key)) continue;
          let p = this.progress.get(key);
          if (p && (!p.dirty || p.due > now)) continue;
          if (!p) {
            if (this.progress.size >= BASELINES) {
              // Best-effort rotation must not become an endless baseline RPC loop.
              if (now >= this.evictionWindow) {
                this.evictionWindow = now + FALLBACK_MS;
                this.evictions = 0;
              }
              if (this.evictions >= QUEUED) continue;
              const old = this.evictionCandidate();

              if (!old) continue;
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
              fallbackDue: now + FALLBACK_MS,
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
    for (const [key, p] of this.queued) {
      if (!this.current(p)) {
        this.discard(p);
        continue;
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
    this.metric({
      kind: 'state',
      queued: this.queued.size,
      baselines: this.progress.size,
      active: this.admission.active,
    });
    let delay = 1000;
    for (const p of this.progress.values()) {
      if (p.locallyDisabled || p.deniedAt !== undefined || this.jobs.has(p.key))
        continue;
      const deadline = p.dirty
        ? p.due
        : p.view.degraded || p.baselineOnly
          ? p.fallbackDue
          : Infinity;
      if (deadline > now) delay = Math.min(delay, deadline - now);
    }
    this.kick(delay);
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
      return reply.result;
    };
    const commit = (upper: bigint) => {
      p.baseline = upper;
      p.baselineOnly = false;
      p.processed = revision;
      p.failures = 0;
      p.due = 0;
      p.fallbackDue = this.clock.now() + FALLBACK_MS;
      p.dirty = p.dirtyDuringFlight || p.view.revision !== revision;
    };
    try {
      p.preference = await notificationKey(p.view.scope, p.channel);
      if (!current()) return;
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
        rows.length < 100 &&
        rows.every((m) => BigInt(m.sequence) > baseline)
      ) {
        const next = await read(before);
        if (!current()) return;
        rows = [...rows, ...next.messages];
        before = next.before;
        incomplete ||= next.missing_predecessors.length > 0;
      }
      incomplete ||=
        rows.length > 100 ||
        (!!before && rows.every((m) => BigInt(m.sequence) > baseline));
      let bytes = 0;
      const bounded: ChatMessage[] = [];
      for (const m of rows.slice(0, 100)) {
        const size =
          m.content.kind === 'text'
            ? new TextEncoder().encode(m.content.text).length
            : 0;
        if (bytes + size > 1024 * 1024) {
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
