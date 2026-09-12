import type { Bridge } from '../bridge';
import { normalizeCommandError } from '../bridge';
import type { ChatReply, ChatResult, ChatScope } from '../chat-contract';
import type { TeamStore, World } from '../model';
import { chatClient, integrity } from './client';
import {
  accountKey,
  teamIdentity,
  contentRevisions,
  readonlyMap,
  readonlySet,
  freezeDto,
} from './snapshots';

type Inbox = Extract<ChatResult, { kind: 'inbox' }>;
export interface TeamInbox {
  state: 'loading' | 'ready' | 'unavailable' | 'blocked';
  data?: Inbox;
  scope?: ChatScope;
  error: string;
  stale: boolean;
  revision: number;
  channelRevisions: ReadonlyMap<string, number>;
  blockedChannels: ReadonlySet<string>;
}
export interface ChatClock {
  now(): number;
  later(fn: () => void, delay: number): unknown;
  cancel(timer: unknown): void;
  random(): number;
}
export const systemChatClock: ChatClock = {
  now: () => Date.now(),
  later: (fn, delay) => setTimeout(fn, delay),
  cancel: (timer) => clearTimeout(timer as ReturnType<typeof setTimeout>),
  random: Math.random,
};
interface Team {
  blocked: Set<string>;
  store: TeamStore;
  dirty: boolean;
  due: number;
  retry: number;
  busy: boolean;
}
interface Account {
  key: string;
  teams: Map<string, Team>;
  head: string;
  actor?: string;
  host?: string;
  due: number;
  retry: number;
  busy: boolean;
  blocked: boolean;
  pollClient?: ReturnType<typeof chatClient>;
  pollTeam?: string;
}
const keyOf = accountKey;
const identity = teamIdentity;
const initial = (): TeamInbox => ({
  state: 'loading',
  error: '',
  stale: false,
  revision: 0,
  channelRevisions: readonlyMap([]),
  blockedChannels: new Set(),
});

/** Main-window owner. Finite jobs, account poll scope, and one background job per profile. */
export class ChatInboxService {
  private accounts = new Map<string, Account>();
  private snapshot: ReadonlyMap<string, TeamInbox> = readonlyMap([]);
  private listeners = new Set<() => void>();
  private jobs = new Map<ReturnType<typeof chatClient>, Account>();
  private profiles = new Set<string>();
  private polls = 0;
  private running = false;
  private timer: unknown;
  private epoch = 0;
  constructor(
    private bridge: Bridge,
    private clock = systemChatClock,
    private capacity = 4,
  ) {}
  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private publish(id: string, entry: TeamInbox) {
    const accepted = Object.freeze({
      ...entry,
      scope: entry.scope ? freezeDto(structuredClone(entry.scope)) : undefined,
      data: entry.data ? freezeDto(structuredClone(entry.data)) : undefined,
      blockedChannels: readonlySet(entry.blockedChannels),
      channelRevisions: readonlyMap(entry.channelRevisions),
    });
    this.snapshot = readonlyMap(new Map(this.snapshot).set(id, accepted));
    for (const listener of this.listeners) listener();
  }
  start() {
    if (!this.running) {
      this.running = true;
      this.kick();
    }
  }
  stop() {
    this.running = false;
    this.epoch++;
    this.clock.cancel(this.timer);
    for (const client of this.jobs.keys()) client.dispose();
    this.jobs.clear();
    this.accounts.clear();
    this.profiles.clear();
    this.polls = 0;
    this.snapshot = readonlyMap([]);
    for (const listener of this.listeners) listener();
  }
  updateStores(world: World) {
    const eligible = world.stores.filter(
      (s): s is TeamStore =>
        s.kind === 'team' &&
        s.team_kind === 'named' &&
        s.active !== false &&
        world.servers.some((p) => p.id === s.server && p.chat_available),
    );
    const wanted = new Map(eligible.map((s) => [s.id, s]));
    for (const account of [...this.accounts.values()]) {
      for (const [id, team] of account.teams) {
        const store = wanted.get(id);
        if (!store || identity(store) !== identity(team.store)) {
          account.teams.delete(id);
          const next = new Map(this.snapshot);
          next.delete(id);
          this.snapshot = readonlyMap(next);
          for (const [client, owner] of this.jobs)
            if (owner === account) client.dispose();
        }
      }
      if (!account.teams.size) this.accounts.delete(account.key);
    }
    for (const store of eligible) {
      const key = keyOf(store);
      let account = this.accounts.get(key);
      if (!account) {
        account = {
          key,
          teams: new Map(),
          head: '0',
          due: 0,
          retry: 250,
          busy: false,
          blocked: false,
        };
        this.accounts.set(key, account);
      }
      if (!account.teams.has(store.id)) {
        account.teams.set(store.id, {
          store,
          blocked: new Set(),
          dirty: true,
          due: 0,
          retry: 250,
          busy: false,
        });
        this.publish(store.id, initial());
      }
    }
    for (const listener of this.listeners) listener();
    this.kick();
  }
  invalidate(id: string) {
    for (const account of this.accounts.values()) {
      const team = account.teams.get(id);
      if (team && !account.blocked) {
        team.dirty = true;
        team.due = 0;
      }
    }
    this.kick();
  }
  isChannelBlocked(id: string, channel: string): boolean {
    return this.snapshot.get(id)?.blockedChannels.has(channel) ?? false;
  }
  blockChannel(id: string, channel: string) {
    for (const account of this.accounts.values()) {
      const team = account.teams.get(id);
      if (!team || account.blocked || team.blocked.has(channel)) continue;
      team.blocked.add(channel);
      const old = this.snapshot.get(id) ?? initial();
      this.publish(id, {
        ...old,
        blockedChannels: new Set(team.blocked),
        data: old.data
          ? withoutBlockedPreviews(old.data, team.blocked)
          : undefined,
        revision: old.revision + 1,
      });
    }
  }
  block(id: string, message: string) {
    for (const account of this.accounts.values())
      if (account.teams.has(id)) {
        account.blocked = true;
        for (const team of account.teams.values())
          this.publish(team.store.id, {
            ...initial(),
            state: 'blocked',
            error: message,
          });
        for (const [client, owner] of this.jobs)
          if (owner === account) client.dispose();
      }
  }
  private accept(account: Account, team: Team, reply: ChatReply) {
    const { scope } = reply;
    const store = team.store;
    if (
      scope.store.profile !== store.server ||
      scope.store.account_alias !== store.account ||
      scope.store.team_alias !== store.alias ||
      scope.store.team_id !== store.team_id_hex ||
      (account.actor &&
        (scope.actor !== account.actor || scope.host !== account.host))
    )
      throw integrity();
    account.actor = scope.actor;
    account.host = scope.host;
  }
  private valid(account: Account, team: Team, epoch: number) {
    return (
      this.running &&
      epoch === this.epoch &&
      this.accounts.get(account.key) === account &&
      account.teams.get(team.store.id) === team &&
      !account.blocked
    );
  }
  private delay(retry: number) {
    return Math.min(
      30_000,
      retry + Math.floor((this.clock.random() * retry) / 4),
    );
  }
  private kick() {
    if (!this.running) return;
    this.clock.cancel(this.timer);
    this.timer = this.clock.later(() => this.drain(), 0);
  }
  private drain() {
    if (!this.running) return;
    const now = this.clock.now();
    for (const account of [...this.accounts.values()]) {
      if (account.blocked) continue;
      const teams = [...account.teams.values()];
      const team = teams.find((t) => !t.busy && t.due <= now);
      if (team && !this.profiles.has(team.store.server))
        void this.sync(account, team);
      const canonical = teams
        .filter((t) => this.snapshot.get(t.store.id)?.state !== 'unavailable')
        .sort((a, b) => a.store.id.localeCompare(b.store.id))[0];
      if (
        canonical &&
        !account.busy &&
        account.due <= now &&
        this.polls < this.capacity
      ) {
        // Move admitted accounts to the back for fair finite-job re-admission.
        this.accounts.delete(account.key);
        this.accounts.set(account.key, account);
        void this.poll(account, canonical);
      }
    }
    this.timer = this.clock.later(() => this.drain(), 250);
  }
  private async sync(account: Account, team: Team) {
    const epoch = this.epoch;
    team.busy = true;
    team.dirty = false;
    this.profiles.add(team.store.server);
    account.teams.delete(team.store.id);
    account.teams.set(team.store.id, team);
    const client = chatClient(this.bridge, team.store.server, team.store.id);
    this.jobs.set(client, account);
    try {
      const reply = await client.request({
        action: 'sync-inbox',
        blocked_channels: [...team.blocked],
      });
      if (!this.valid(account, team, epoch)) return;
      this.accept(account, team, reply);
      if (reply.result.kind !== 'inbox') throw integrity();
      for (const id of reply.result.blocked_channels) team.blocked.add(id);
      const data = withoutBlockedPreviews(reply.result, team.blocked);
      const old = this.snapshot.get(team.store.id);
      const revisions = contentRevisions(
        old?.data,
        data,
        old?.channelRevisions ?? new Map(),
      );
      const changed = [...revisions].some(
        ([id, value]) => old?.channelRevisions.get(id) !== value,
      );
      this.publish(team.store.id, {
        state: 'ready',
        blockedChannels: new Set(team.blocked),
        scope: reply.scope,
        data,
        error: data.read_retry_pending
          ? 'Read status will retry.'
          : data.previews_incomplete
            ? 'Some previews are unavailable.'
            : '',
        stale: false,
        revision: (old?.revision ?? 0) + Number(changed),
        channelRevisions: revisions,
      });
      team.retry = 250;
      team.due = team.dirty ? 0 : this.clock.now() + 25_000;
    } catch (cause) {
      if (!this.valid(account, team, epoch)) return;
      const error = normalizeCommandError(cause);
      if (error.fatal) this.block(team.store.id, error.message);
      else if (error.code !== 'cancelled') {
        const denied = [
          'chat-access-denied',
          'capability-denied',
          'chat-unsupported',
        ].includes(error.code);
        if (denied && account.pollTeam === team.store.id) {
          account.pollClient?.dispose();
          account.due = 0;
        }
        const old = this.snapshot.get(team.store.id) ?? initial();
        this.publish(
          team.store.id,
          denied
            ? {
                ...initial(),
                blockedChannels: new Set(team.blocked),
                state: 'unavailable',
                error: error.message,
              }
            : { ...old, error: error.message, stale: true },
        );
        team.dirty = true;
        team.due =
          this.clock.now() + (denied ? 30_000 : this.delay(team.retry));
        team.retry = Math.min(30_000, team.retry * 2);
      }
    } finally {
      client.dispose();
      this.jobs.delete(client);
      if (epoch === this.epoch) {
        team.busy = false;
        this.profiles.delete(team.store.server);
      }
    }
  }
  private async poll(account: Account, team: Team) {
    const epoch = this.epoch;
    const since = account.head;
    account.busy = true;
    this.polls++;
    const client = chatClient(this.bridge, team.store.server, team.store.id);
    this.jobs.set(client, account);
    account.pollClient = client;
    account.pollTeam = team.store.id;
    try {
      const reply = await client.request({
        action: 'poll-inbox',
        since,
        timeout_milliseconds: 25_000,
      });
      if (!this.valid(account, team, epoch)) return;
      this.accept(account, team, reply);
      if (reply.result.kind !== 'poll') throw integrity();
      const result = reply.result;
      if (result.bumped !== BigInt(result.inbox_version) > BigInt(since))
        throw integrity();
      if (BigInt(result.inbox_version) < BigInt(since)) {
        account.head = result.inbox_version;
        // Cancel pre-reset projections, preserve account identity and hard Rust anchors.
        for (const [job, owner] of this.jobs)
          if (owner === account && job !== client) job.dispose();
        for (const t of account.teams.values()) {
          t.dirty = true;
          t.due = 0;
          this.publish(t.store.id, {
            ...initial(),
            blockedChannels: new Set(t.blocked),
          });
        }
      } else if (result.bumped) {
        account.head =
          BigInt(result.inbox_version) > BigInt(account.head)
            ? result.inbox_version
            : account.head;
        for (const t of account.teams.values()) {
          t.dirty = true;
          t.due = 0;
        }
      }
      account.retry = 250;
      account.due = this.clock.now() + 250;
    } catch (cause) {
      if (!this.valid(account, team, epoch)) return;
      const error = normalizeCommandError(cause);
      if (error.fatal) this.block(team.store.id, error.message);
      if (error.code === 'cancelled') {
        account.due = 0;
        return;
      }
      account.due = this.clock.now() + this.delay(account.retry);
      account.retry = Math.min(30_000, account.retry * 2);
    } finally {
      client.dispose();
      this.jobs.delete(client);
      if (epoch === this.epoch) {
        account.pollClient = undefined;
        account.pollTeam = undefined;
        account.busy = false;
        this.polls--;
      }
    }
  }
}

// A sync started before quarantine may finish afterward. Sanitize it at publication.
function withoutBlockedPreviews(
  data: Inbox,
  blocked: ReadonlySet<string>,
): Inbox {
  return {
    ...data,
    conversations: data.conversations.map((c) =>
      blocked.has(c.channel.id) ? { ...c, preview: null } : c,
    ),
  };
}
