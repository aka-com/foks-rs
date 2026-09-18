import type { Bridge, CommandError } from '../bridge';
import { commandRecovery, normalizeCommandError } from '../bridge';
import { reportReadinessError } from '../bridge/errors';
import type { ChatReply, ChatResult, ChatScope } from '../chat-contract';
import { chatAvailable } from '../model';
import type { AvailabilityOptions, TeamStore, AgentSnapshot } from '../model';
import { cancelled as cancelledAccess, chatClient, integrity } from './client';
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
  failure?: CommandError;
  stale: boolean;
  revision: number;
  /** Advances only after an accepted authorized team synchronization. */
  authorizationRevision?: number;
  channelRevisions: ReadonlyMap<string, number>;
  /** Conservative invalidation for active threads on degraded Basic projections. */
  channelRefreshRevisions?: ReadonlyMap<string, number>;
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
  generation: number;
  quarantine?: CommandError;
  blocked: Set<string>;
  store: TeamStore;
  dirty: boolean;
  due: number;
  retry: number;
  busy: boolean;
  /** Authenticated compatibility deadline, in the clock's millisecond unit. */
  accessExpiresAt?: number;
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
  blocked?: CommandError;
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
      failure: entry.failure
        ? freezeDto(structuredClone(entry.failure))
        : undefined,
      data: entry.data ? freezeDto(structuredClone(entry.data)) : undefined,
      blockedChannels: readonlySet(entry.blockedChannels),
      channelRevisions: readonlyMap(entry.channelRevisions),
      channelRefreshRevisions: readonlyMap(
        entry.channelRefreshRevisions ?? entry.channelRevisions,
      ),
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
  /**
   * Use the shell's availability clock so the Chat tab and inbox service agree
   * when a check-in expires. Otherwise the tab could wait for an entry that the
   * service considers ineligible. With no supplied clock, use the service clock
   * so tests can control this decision.
   *
   * A team is eligible when `chatAvailable` returns true, matching the Chat
   * tab, team column, and New chat sheet.
   */
  updateStores(
    agentSnapshot: AgentSnapshot,
    options: AvailabilityOptions = {},
  ) {
    const timed: AvailabilityOptions = {
      ...options,
      nowSeconds: options.nowSeconds ?? this.clock.now() / 1000,
    };
    const eligible = agentSnapshot.stores.filter(
      (s): s is TeamStore =>
        s.kind === 'team' &&
        s.team_kind === 'named' &&
        chatAvailable(agentSnapshot, s, timed),
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
        };
        this.accounts.set(key, account);
      }
      if (!account.teams.has(store.id)) {
        const server = agentSnapshot.servers.find(
          (entry) => entry.id === store.server,
        );
        account.teams.set(store.id, {
          generation: 0,
          store,
          blocked: new Set(),
          dirty: true,
          due: 0,
          retry: 250,
          busy: false,
          accessExpiresAt:
            server?.compatibility.status === 'required'
              ? server.compatibility.expiresAt * 1000
              : undefined,
        });
        this.publish(
          store.id,
          account.blocked
            ? {
                ...initial(),
                state: 'blocked',
                error: account.blocked.message,
                failure: account.blocked,
              }
            : initial(),
        );
      } else {
        const team = account.teams.get(store.id);
        const server = agentSnapshot.servers.find(
          (entry) => entry.id === store.server,
        );
        if (team) {
          team.store = store;
          team.accessExpiresAt =
            server?.compatibility.status === 'required'
              ? server.compatibility.expiresAt * 1000
              : undefined;
        }
      }
    }
    for (const listener of this.listeners) listener();
    this.kick();
  }
  invalidate(id: string) {
    for (const account of this.accounts.values()) {
      const team = account.teams.get(id);
      if (team && !account.blocked && !team.quarantine) {
        team.dirty = true;
        team.due = 0;
      }
    }
    this.kick();
  }
  /**
   * Whether a team has been invalidated and the synchronization that answers it
   * has not published yet. A channel this Mac has just created is listed by
   * that synchronization, so a location naming it is waiting rather than naming
   * something that is not there. A synchronization already running counts:
   * `sync` clears `dirty` when it starts and publishes only when it lands.
   */
  isInvalidated(id: string): boolean {
    for (const account of this.accounts.values()) {
      const team = account.teams.get(id);
      if (team) return team.dirty || team.busy;
    }
    return false;
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
    this.handleError(id, integrity(message));
  }
  handleError(id: string, cause: unknown, channel?: string): boolean {
    const error = normalizeCommandError(cause);
    const recovery = commandRecovery(error);
    if (recovery.kind === 'ignore') return true;
    if (recovery.scope === 'channel' && channel) {
      this.blockChannel(id, channel);
      return true;
    }
    if (
      recovery.scope === 'request' &&
      !error.fatal &&
      error.code !== 'invalid-response'
    )
      return false;
    const profile = [...this.accounts.values()]
      .flatMap((account) => [...account.teams.values()])
      .find((team) => team.store.id === id)?.store.server;
    for (const account of this.accounts.values()) {
      const teams = [...account.teams.values()].filter(
        (team) =>
          recovery.scope === 'agent' ||
          (recovery.scope === 'profile' && team.store.server === profile) ||
          (recovery.scope === 'account' && account.teams.has(id)) ||
          team.store.id === id,
      );
      if (!teams.length || account.blocked) continue;
      const quarantined = recovery.kind === 'quarantine';
      if (quarantined && recovery.scope !== 'channel') account.blocked = error;
      for (const team of teams) {
        team.generation++;
        if (quarantined) team.quarantine = error;
        team.dirty = true;
        team.due =
          quarantined || recovery.scope === 'request'
            ? Infinity
            : this.clock.now() +
              (recovery.kind === 'revalidate'
                ? 30_000
                : this.delay(team.retry));
        team.retry = Math.min(30_000, team.retry * 2);
        this.publish(team.store.id, {
          ...initial(),
          state: quarantined ? 'blocked' : 'unavailable',
          error: error.message,
          failure: error,
          authorizationRevision: this.snapshot.get(team.store.id)
            ?.authorizationRevision,
          revision: (this.snapshot.get(team.store.id)?.revision ?? 0) + 1,
          blockedChannels: new Set(team.blocked),
        });
      }
      for (const [client, owner] of this.jobs)
        if (owner === account) client.dispose();
      account.due = this.clock.now() + this.delay(account.retry);
    }
    reportReadinessError(error);
    this.kick();
    return true;
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
  private valid(
    account: Account,
    team: Team,
    epoch: number,
    generation: number,
  ) {
    return (
      this.running &&
      epoch === this.epoch &&
      this.accounts.get(account.key) === account &&
      account.teams.get(team.store.id) === team &&
      !account.blocked &&
      !team.quarantine &&
      generation === team.generation &&
      this.accessValid(team)
    );
  }
  private accessValid(team: Team): boolean {
    return (
      team.accessExpiresAt === undefined ||
      this.clock.now() < team.accessExpiresAt
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
      const team = teams.find(
        (candidate) =>
          !candidate.quarantine &&
          this.accessValid(candidate) &&
          !candidate.busy &&
          candidate.due <= now,
      );
      if (team && !this.profiles.has(team.store.server))
        void this.sync(account, team);
      const canonical = teams
        .filter(
          (candidate) =>
            !candidate.quarantine &&
            this.accessValid(candidate) &&
            this.snapshot.get(candidate.store.id)?.state !== 'unavailable',
        )
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
    const generation = team.generation;
    team.busy = true;
    team.dirty = false;
    this.profiles.add(team.store.server);
    account.teams.delete(team.store.id);
    account.teams.set(team.store.id, team);
    const client = chatClient(this.bridge, team.store.server, team.store.id);
    this.jobs.set(client, account);
    try {
      if (!this.valid(account, team, epoch, generation)) return;
      const reply = await client.request(
        {
          action: 'sync-inbox',
          blocked_channels: [...team.blocked],
        },
        undefined,
        () => {
          if (!this.valid(account, team, epoch, generation))
            throw cancelledAccess();
        },
      );
      if (!this.valid(account, team, epoch, generation)) return;
      this.accept(account, team, reply);
      if (reply.result.kind !== 'inbox') throw integrity();
      for (const id of reply.result.blocked_channels) team.blocked.add(id);
      const data = withoutBlockedPreviews(reply.result, team.blocked);
      const old = this.snapshot.get(team.store.id);
      const revisions = contentRevisions(
        old?.data,
        data,
        old?.channelRevisions ?? new Map(),
        false,
      );
      const refreshRevisions = contentRevisions(
        old?.data,
        data,
        old?.channelRefreshRevisions ?? old?.channelRevisions ?? new Map(),
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
        channelRefreshRevisions: refreshRevisions,
        authorizationRevision: (old?.authorizationRevision ?? 0) + 1,
      });
      team.retry = 250;
      team.due = team.dirty ? 0 : this.clock.now() + 25_000;
    } catch (cause) {
      if (!this.valid(account, team, epoch, generation)) return;
      const error = normalizeCommandError(cause);
      if (this.handleError(team.store.id, error)) return;
      const old = this.snapshot.get(team.store.id) ?? initial();
      this.publish(team.store.id, {
        ...old,
        error: error.message,
        failure: error,
        stale: true,
      });
      team.dirty = true;
      team.due = this.clock.now() + this.delay(team.retry);
      team.retry = Math.min(30_000, team.retry * 2);
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
    const generation = team.generation;
    const since = account.head;
    account.busy = true;
    this.polls++;
    const client = chatClient(this.bridge, team.store.server, team.store.id);
    this.jobs.set(client, account);
    account.pollClient = client;
    account.pollTeam = team.store.id;
    try {
      if (!this.valid(account, team, epoch, generation)) return;
      const reply = await client.request(
        {
          action: 'poll-inbox',
          since,
          timeout_milliseconds: 25_000,
        },
        undefined,
        () => {
          if (!this.valid(account, team, epoch, generation))
            throw cancelledAccess();
        },
      );
      if (!this.valid(account, team, epoch, generation)) return;
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
      if (!this.valid(account, team, epoch, generation)) return;
      const error = normalizeCommandError(cause);
      if (error.code === 'cancelled') {
        account.due = 0;
        return;
      }
      if (this.handleError(team.store.id, error)) return;
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
