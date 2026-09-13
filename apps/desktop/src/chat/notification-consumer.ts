import type { Bridge } from '../bridge';
import { normalizeCommandError } from '../bridge';
import type { ChatMessage, ChatScope } from '../chat-contract';
import { chatClient, sameScope } from './client';
import type { ChatInboxService } from './inbox-service';
import type { LocalSession } from './local-contract';
import { notificationKey } from './local-contract';
import { incoming, notificationText } from './notification-policy';
import { messageVisible } from './visibility';

type Progress = {
  scope: ChatScope;
  revision: number;
  baseline: bigint | null;
  due: number;
  failures: number;
};
/** A bounded consumer, never a poll owner. Each history RPC uses the profile FIFO. */
export class NotificationConsumer {
  private progress = new Map<string, Progress>();
  private jobs = new Map<string, ReturnType<typeof chatClient>>();
  private profiles = new Set<string>();
  private stopped = false;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private off: () => void;
  private rotation = 0;
  private generation = 0;
  private lifetime = new AbortController();
  constructor(
    private bridge: Bridge,
    private service: ChatInboxService,
    private session: LocalSession,
    private report: (message: string) => void,
    private now = () => performance.now(),
  ) {
    this.off = service.subscribe(() => this.kick());
    this.kick();
  }
  stop() {
    this.stopped = true;
    this.generation++;
    this.lifetime.abort();
    this.off();
    clearTimeout(this.timer);
    for (const job of this.jobs.values()) job.dispose();
    this.jobs.clear();
    this.progress.clear();
  }
  private kick() {
    if (this.stopped || this.timer) return;
    this.timer = setTimeout(() => {
      this.timer = undefined;
      void this.scan();
    }, 100);
  }
  private async scan() {
    if (
      this.stopped ||
      !this.session.available ||
      !this.session.settings.enabled
    )
      return;
    const eligible: {
      id: string;
      channel: string;
      scope: ChatScope;
      revision: number;
    }[] = [];
    const live = new Set<string>();
    let invalidated = false;
    for (const [id, entry] of this.service.getSnapshot()) {
      if (entry.state !== 'ready' || entry.stale || !entry.scope || !entry.data)
        continue;
      for (const ch of entry.data.channels) {
        const key = JSON.stringify([id, ch.id]);
        live.add(key);
        const previous = this.progress.get(key);
        if (previous && !sameScope(previous.scope, entry.scope)) {
          this.progress.delete(key);
          invalidated = true;
        }
        const conversation = entry.data.conversations.find(
          (c) => c.channel.id === ch.id,
        );
        if (
          !ch.readable ||
          entry.blockedChannels.has(ch.id) ||
          conversation?.muted ||
          conversation?.hidden
        ) {
          invalidated ||= this.progress.has(key);
          this.progress.delete(key);
          continue;
        }
        eligible.push({
          id,
          channel: ch.id,
          scope: entry.scope,
          revision: entry.revision,
        });
      }
    }
    for (const key of this.progress.keys())
      if (!live.has(key)) {
        this.progress.delete(key);
        invalidated = true;
      }
    if (invalidated)
      void this.bridge
        .chatLocal({ action: 'clear', epoch: this.session.epoch })
        .catch(() => this.report('Could not clear desktop alerts.'));
    const size = eligible.length;
    for (let n = 0; n < size && this.jobs.size < 2; n++) {
      const item = eligible[this.rotation++ % size],
        key = JSON.stringify([item.id, item.channel]);
      if (this.jobs.has(key) || this.profiles.has(item.scope.store.profile))
        continue;
      let p = this.progress.get(key);
      if (p && (p.due > this.now() || p.revision === item.revision)) continue;
      if (!p) {
        if (this.progress.size >= 4096) {
          const old = Array.from(this.progress.keys()).find(
            (k) => !this.jobs.has(k),
          );
          if (old) this.progress.delete(old);
        }
        p = {
          scope: item.scope,
          revision: -1,
          baseline: null,
          due: 0,
          failures: 0,
        };
        this.progress.set(key, p);
      }
      const client = chatClient(this.bridge, item.scope.store.profile, item.id);
      this.jobs.set(key, client);
      this.profiles.add(item.scope.store.profile);
      void this.run(item, key, p, client).finally(() => {
        client.dispose();
        this.jobs.delete(key);
        this.profiles.delete(item.scope.store.profile);
        clearTimeout(this.timer);
        this.timer = undefined;
        this.kick();
      });
    }
    if (!this.stopped && !this.timer)
      this.timer = setTimeout(() => {
        this.timer = undefined;
        void this.scan();
      }, 1000);
  }
  private async run(
    item: { id: string; channel: string; scope: ChatScope; revision: number },
    key: string,
    p: Progress,
    client: ReturnType<typeof chatClient>,
  ) {
    const current = () => {
      const entry = this.service.getSnapshot().get(item.id);
      return (
        !this.stopped &&
        this.progress.get(key) === p &&
        entry?.state === 'ready' &&
        !entry.stale &&
        !!entry.scope &&
        sameScope(entry.scope, item.scope) &&
        !entry.blockedChannels.has(item.channel) &&
        entry.data?.channels.some((c) => c.id === item.channel && c.readable) &&
        !entry.data?.conversations.some(
          (c) => c.channel.id === item.channel && (c.muted || c.hidden),
        )
      );
    };
    // Release full history bodies at this boundary; retain only bounded snippets.
    const read = async (before: string | null) => {
      const reply = await client.request(
        {
          action: 'notification-history',
          channel: item.channel,
          before,
        },
        {
          key: JSON.stringify([item.scope, item.channel]),
          owner: client,
          generation: this.generation,
          signal: this.lifetime.signal,
          current: () => !!current(),
          cancel: () => client.dispose(),
          preemptible: false,
        },
      );
      return {
        ...reply,
        result: {
          ...reply.result,
          messages: reply.result.messages.map((m) => ({
            ...m,
            content:
              m.content.kind === 'text'
                ? {
                    kind: 'text' as const,
                    text: Array.from(m.content.text).slice(0, 256).join(''),
                  }
                : m.content,
          })),
        },
      };
    };
    try {
      const setting = await notificationKey(item.scope, item.channel);
      if (!current()) return;
      if (this.session.settings.overrides[setting] === false) {
        p.baseline = null;
        p.revision = item.revision;
        return;
      }
      const first = await read(null);
      if (!current() || !sameScope(first.scope, item.scope)) return;
      const upper = first.result.messages.reduce(
        (max, m) => (BigInt(m.sequence) > max ? BigInt(m.sequence) : max),
        0n,
      );
      if (p.baseline === null) {
        p.baseline = upper;
        p.revision = item.revision;
        p.failures = 0;
        return;
      }
      if (upper < p.baseline) {
        p.baseline = null;
        return;
      }
      let rows = first.result.messages,
        missing = first.result.missing_predecessors.length > 0;
      let before = first.result.before;
      if (
        before &&
        rows.length < 100 &&
        rows.every((m) => BigInt(m.sequence) > p.baseline!)
      ) {
        const next = await read(before);
        if (!current() || !sameScope(next.scope, item.scope)) return;
        rows = [...rows, ...next.result.messages];
        missing ||= next.result.missing_predecessors.length > 0;
        before = next.result.before;
      }
      const incomplete =
        missing ||
        rows.length > 100 ||
        (!!before && rows.every((m) => BigInt(m.sequence) > p.baseline!));
      let bytes = 0;
      const bounded: ChatMessage[] = [];
      for (const m of rows.slice(0, 100)) {
        bytes +=
          m.content.kind === 'text'
            ? new TextEncoder().encode(m.content.text).length
            : 0;
        if (bytes > 1024 * 1024) break;
        bounded.push(m);
      }
      const candidates = incoming(
        bounded,
        p.baseline,
        upper,
        item.scope.actor,
        (id) => messageVisible(item.id, item.channel, id),
      );
      // Commit local progress before OS effects: ambiguous delivery is not retried.
      p.baseline = upper;
      p.revision = item.revision;
      p.failures = 0;
      p.due = 0;
      for (const alert of notificationText(
        candidates,
        incomplete || bounded.length < Math.min(100, rows.length),
      )) {
        if (!current()) return;
        try {
          await this.bridge.chatLocal({
            action: 'display',
            epoch: this.session.epoch,
            storeId: item.id,
            scope: item.scope,
            channel: item.channel,
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
      if (error.code === 'cancelled') return;
      p.failures = Math.min(p.failures + 1, 6);
      p.due = this.now() + Math.min(1000 * 2 ** (p.failures - 1), 30000);
      if (error.code === 'chat-channel-integrity')
        this.service.blockChannel(item.id, item.channel);
      else if (error.code === 'chat-access-denied') {
        p.baseline = null;
        p.revision = item.revision;
      } else if (error.fatal) this.service.block(item.id, error.message);
    }
  }
}
