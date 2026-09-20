import type { Bridge, CommandError } from '../bridge';
import { commandRecovery, normalizeCommandError } from '../bridge';
import { reportReadinessError } from '../bridge/errors';
import type { ChatReply, ChatResult, ChatScope } from '../chat-contract';
import { chatAvailable } from '../model';
import type { AvailabilityOptions, TeamStore, AgentSnapshot } from '../model';
import { cancelled as cancelledAccess, chatClient, integrity } from './client';
import { ChatHistoryCache } from './history-cache';
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
  /**
   * Nonfatal limitations from a successful synchronization, such as a pending
   * read-status retry or an unavailable preview. Current data remains usable
   * and polling continues.
   */
  note: string;
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
/**
 * One poll, one synchronization, or the latency between a poll that said an
 * account moved and the synchronization that published the change. Carries
 * the account key and the store id, never a channel or a message.
 */
export type ChatInboxTiming =
  | {
      kind: 'poll';
      account: string;
      milliseconds: number;
      bumped: boolean;
      outcome: 'ok' | 'error' | 'cancelled';
      code?: string;
    }
  | {
      kind: 'sync';
      store: string;
      milliseconds: number;
      changed: boolean;
      conversations: number;
      outcome: 'ok' | 'error' | 'cancelled';
      code?: string;
    }
  | { kind: 'arrival'; store: string; milliseconds: number };
export const systemChatClock: ChatClock = {
  now: () => Date.now(),
  later: (fn, delay) => setTimeout(fn, delay),
  cancel: (timer) => clearTimeout(timer as ReturnType<typeof setTimeout>),
  random: Math.random,
};
interface Team {
  generation: number;
  binding: string;
  quarantine?: CommandError;
  blocked: Set<string>;
  store: TeamStore;
  dirty: boolean;
  due: number;
  retry: number;
  busy: boolean;
  /** Authenticated compatibility deadline, in the clock's millisecond unit. */
  accessExpiresAt?: number;
  /** When a poll last said this team's account moved, until a sync publishes. */
  bumpedAt?: number;
  /**
   * Per channel, when a degraded projection last invalidated it with no
   * evidence that anything moved, and how long the next such invalidation
   * waits. Only consulted while the projection is degraded.
   */
  degraded: Map<string, { at: number; interval: number; pending: boolean }>;
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
  /** The poll failure the account is backing off from, until a poll succeeds. */
  failure?: CommandError;
  pollClient?: ReturnType<typeof chatClient>;
  pollTeam?: string;
}
const keyOf = accountKey;
const identity = teamIdentity;
/**
 * Whether a catalog read answers the failure: the backend reported that the
 * profile's catalog was retired or not yet read, or that it no longer lists
 * the store. Until the next read lands, retries can only repeat the answer,
 * and once it has landed the backoff they built up has nothing left to wait
 * for.
 */
const answeredByCatalog = (failure: CommandError | undefined): boolean =>
  failure?.code === 'catalog-required' || failure?.code === 'store-not-found';
/**
 * How long a degraded projection waits before invalidating a channel again on
 * no evidence, and the ceiling that wait doubles toward.
 *
 * A degraded host cannot report what changed, so an invalidation it raises is
 * a guess, and acting on every guess is what makes a degraded host poll: with
 * twenty channels the fallback alone costs four full history reads a second.
 * The channel the user has open keeps the base interval and never backs off;
 * every other channel doubles its wait up to the ceiling and resets to the
 * base the moment the projection shows it actually moved.
 *
 * **This trades alert latency on a degraded host, and it is a product
 * decision.** On such a host nothing reports which thread moved, so a message
 * in a channel that is not open surfaces at that channel's current wait —
 * `DEGRADED_REFRESH_MAX_MS` in the worst case, against five seconds before.
 * Nothing is lost: the wait bounds when the read happens, not whether it
 * happens, and the first read that finds the message resets the channel to
 * the base interval.
 */
const DEGRADED_REFRESH_BASE_MS = 5_000;
const DEGRADED_REFRESH_MAX_MS = 60_000;
/**
 * How long an idle team waits before resynchronizing, and the short cadence
 * it keeps instead while its account's poll is failing or its projection is
 * degraded.
 *
 * The resynchronization is not what carries messages: a send stamps every
 * recipient's inbox version server-side, which ends the account's long poll,
 * which marks every team of that account due immediately. What it uniquely
 * catches is what moves no message — a new empty channel, a rename, a role
 * change that alters readability, a preference change — and recovery when the
 * poll itself is not running. None of those needs twenty-five seconds, so it
 * keeps the short cadence only while the poll is failing or the host cannot
 * report what changed.
 */
const RESYNC_IDLE_MS = 300_000;
const RESYNC_SHORT_MS = 25_000;
const initial = (): TeamInbox => ({
  state: 'loading',
  error: '',
  note: '',
  stale: false,
  revision: 0,
  channelRevisions: readonlyMap([]),
  blockedChannels: new Set(),
});

/** Main-window owner. Finite jobs, account poll scope, and one background job per profile. */
export class ChatInboxService {
  readonly histories = new ChatHistoryCache();
  private accessGenerations: ReadonlyMap<string, number> = new Map();
  private accounts = new Map<string, Account>();
  private snapshot: ReadonlyMap<string, TeamInbox> = readonlyMap([]);
  private listeners = new Set<() => void>();
  private jobs = new Map<ReturnType<typeof chatClient>, Account>();
  private profiles = new Set<string>();
  private polls = 0;
  private running = false;
  private visible = true;
  private timer: unknown;
  private epoch = 0;
  /**
   * Told when a team's read answers that its profile's vault has not been
   * refreshed, so the shell can request that profile's catalog job. Asked
   * once per profile per ten seconds, however many teams or retries say so.
   */
  onCatalogRequired?: (profile: string) => void;
  private catalogRequests = new Map<string, number>();
  private observers = new Set<(event: ChatInboxTiming) => void>();
  /**
   * Per store, the channel whose thread is mounted, or nothing. A degraded
   * projection never backs this channel off, and the notification consumer
   * reads it for the same reason; both are best-effort presentation, never
   * authority over what may be read.
   */
  private open = new Map<string, string>();
  constructor(
    private bridge: Bridge,
    private clock = systemChatClock,
    private capacity = 4,
  ) {}
  getSnapshot = () => this.snapshot;
  /** Opt-in timings; an observer cannot change what the service does. */
  observe(observer: (event: ChatInboxTiming) => void): () => void {
    this.observers.add(observer);
    return () => {
      this.observers.delete(observer);
    };
  }
  private report(event: ChatInboxTiming) {
    const frozen = Object.freeze(event);
    for (const observer of this.observers) {
      try {
        observer(frozen);
      } catch {
        /* diagnostics are not authority */
      }
    }
  }
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
    this.histories.update(
      id,
      accepted,
      this.accessGenerations.get(accepted.scope?.store.profile ?? '') ?? 0,
    );
    for (const listener of this.listeners) listener();
  }
  start() {
    if (!this.running) {
      this.running = true;
      this.kick();
    }
  }
  /**
   * While hidden, synchronize teams only when account polling reports a change.
   * Suspend periodic full-team synchronization until the window becomes visible.
   */
  setVisible(visible: boolean) {
    if (visible === this.visible) return;
    this.visible = visible;
    // Showing the window is one of the moments the periodic
    // resynchronization exists for: what it uniquely catches is what the
    // account poll cannot report, and the user is about to look at it. The
    // idle cadence is minutes, so waiting for it here would show a stale
    // channel list; every team's next pass is due immediately instead.
    if (visible)
      for (const account of this.accounts.values())
        for (const team of account.teams.values())
          if (!account.blocked && !team.quarantine && team.due !== Infinity)
            team.due = 0;
    this.kick();
  }
  /**
   * States which channel's thread is mounted for a store, or none. Presenting
   * a channel is what exempts it from the degraded backoff: it is the one
   * channel whose staleness the user can see.
   */
  setOpenChannel(id: string, channel: string | null) {
    if (this.open.get(id) === (channel ?? undefined)) return;
    if (channel === null) this.open.delete(id);
    else this.open.set(id, channel);
    // Tell the listeners: a consumer holding a backed-off channel is asleep
    // on a timer it armed for the interval that channel had reached, and
    // presenting it is exactly the moment that interval no longer applies.
    // The published snapshot is unchanged, so a renderer that reads it
    // re-renders nothing.
    for (const listener of this.listeners) listener();
  }
  openChannel(id: string): string | undefined {
    return this.open.get(id);
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
    this.open.clear();
    this.snapshot = readonlyMap([]);
    this.histories.clear();
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
    accessGenerations: ReadonlyMap<string, number> = new Map(),
  ) {
    this.accessGenerations = accessGenerations;
    const binding = (store: TeamStore): string => {
      const server = agentSnapshot.servers.find(
        (entry) => entry.id === store.server,
      );
      return JSON.stringify([
        identity(store),
        server?.host_id,
        server?.configuredProbe,
        accessGenerations.get(store.server) ?? 0,
      ]);
    };
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
        if (!store || binding(store) !== team.binding) {
          account.teams.delete(id);
          this.open.delete(id);
          this.histories.clear(id);
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
          binding: binding(store),
          store,
          blocked: new Set(),
          degraded: new Map(),
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
          // The catalog this snapshot carries was read after the failure the
          // team is backing off from, so the next attempt is due now.
          if (answeredByCatalog(this.snapshot.get(store.id)?.failure)) {
            team.dirty = true;
            team.due = 0;
            team.retry = 250;
          }
        }
      }
    }
    for (const account of this.accounts.values()) {
      if (!answeredByCatalog(account.failure)) continue;
      account.failure = undefined;
      account.due = 0;
      account.retry = 250;
    }
    for (const listener of this.listeners) listener();
    this.kick();
  }
  /**
   * Publish a read the agent confirmed, with no synchronization behind it.
   *
   * The agent answers a mark-read only after the server accepted the pointer
   * for that exact sequence and its own store recorded it, so what is applied
   * here is never ahead of what the server holds. It is applied as a floor:
   * a read another device made first is not rolled back, and a reply that
   * lands after one leaves the larger pointer alone. The conversation's
   * newest position is held fixed — the unread count falls by exactly what
   * the pointer rose — so this publishes no channel invalidation and the next
   * synchronization compares equal.
   */
  applyRead(id: string, channel: string, sequence: string) {
    const old = this.snapshot.get(id);
    if (!old?.data) return;
    const marked = BigInt(sequence);
    let moved = false;
    const conversations = old.data.conversations.map((conversation) => {
      const read = BigInt(conversation.read_through);
      if (conversation.channel.id !== channel || read >= marked)
        return conversation;
      const pending =
        conversation.pending_read === null
          ? null
          : BigInt(conversation.pending_read);
      const unread = BigInt(conversation.unread);
      const effective = pending !== null && pending > read ? pending : read;
      const next = marked > effective ? marked : effective;
      moved = true;
      return {
        ...conversation,
        read_through: String(marked),
        // The agent's store clears a staged read the confirmation overtook
        // and keeps one it did not; mirror that rather than guess.
        pending_read:
          pending !== null && pending > marked ? String(pending) : null,
        unread: String(
          effective + unread > next ? effective + unread - next : 0n,
        ),
      };
    });
    if (!moved) return;
    this.publish(id, {
      ...old,
      data: { ...old.data, conversations },
    });
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
  private profileOf(id: string): string | undefined {
    return [...this.accounts.values()]
      .flatMap((account) => [...account.teams.values()])
      .find((team) => team.store.id === id)?.store.server;
  }
  private requestCatalogRefresh(profile: string | undefined) {
    if (!profile || !this.onCatalogRequired) return;
    const last = this.catalogRequests.get(profile);
    const now = this.clock.now();
    if (last !== undefined && now - last < 10_000) return;
    this.catalogRequests.set(profile, now);
    this.onCatalogRequired(profile);
  }
  handleError(id: string, cause: unknown, channel?: string): boolean {
    const error = normalizeCommandError(cause);
    const recovery = commandRecovery(error);
    if (error.code === 'catalog-required')
      this.requestCatalogRefresh(this.profileOf(id));
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
    const profile = this.profileOf(id);
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
  /**
   * Whether a degraded projection may invalidate a channel on no evidence
   * this pass, advancing that channel's backoff when it does.
   *
   * `moved` states what the comparison that ignores degradation already saw:
   * a channel it reports is invalidated by that change alone, so admitting it
   * here would buy nothing, and its backoff resets because the projection is
   * demonstrably still reporting this channel's arrivals. The channel the
   * user has open never backs off; it is rate-limited to the base interval so
   * that a burst of synchronizations cannot turn its thread into a poll.
   */
  private degradedAdmission(
    team: Team,
    moved: (channel: string) => boolean,
  ): (channel: string) => boolean {
    const now = this.clock.now();
    const open = this.open.get(team.store.id);
    return (channel: string) => {
      let state = team.degraded.get(channel);
      if (!state) {
        state = {
          at: -Infinity,
          interval: DEGRADED_REFRESH_BASE_MS,
          pending: false,
        };
        team.degraded.set(channel, state);
      }
      if (moved(channel)) {
        state.at = now;
        state.interval = DEGRADED_REFRESH_BASE_MS;
        state.pending = false;
        return false;
      }
      const wait = channel === open ? DEGRADED_REFRESH_BASE_MS : state.interval;
      if (now - state.at < wait) {
        state.pending = true;
        return false;
      }
      state.pending = false;
      state.at = now;
      state.interval =
        channel === open
          ? DEGRADED_REFRESH_BASE_MS
          : Math.min(state.interval * 2, DEGRADED_REFRESH_MAX_MS);
      return true;
    };
  }
  /** Release coalesced invalidations without another inbox RPC. */
  private flushDegraded(team: Team) {
    if (team.quarantine || !this.accessValid(team)) return;
    const old = this.snapshot.get(team.store.id);
    if (old?.state !== 'ready' || old.stale || !old.data?.degraded) return;
    const admit = this.degradedAdmission(team, () => false);
    let revisions: Map<string, number> | undefined;
    for (const [channel, state] of team.degraded) {
      if (!state.pending || !admit(channel)) continue;
      revisions ??= new Map(
        old.channelRefreshRevisions ?? old.channelRevisions,
      );
      revisions.set(channel, (revisions.get(channel) ?? 0) + 1);
    }
    if (revisions)
      this.publish(team.store.id, {
        ...old,
        channelRefreshRevisions: revisions,
      });
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
      for (const team of teams) this.flushDegraded(team);
      const team = teams.find(
        (candidate) =>
          !candidate.quarantine &&
          this.accessValid(candidate) &&
          !candidate.busy &&
          candidate.due <= now &&
          (this.visible || candidate.dirty),
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
    this.timer = this.clock.later(
      () => this.drain(),
      this.visible ? 250 : 2_000,
    );
  }
  private async sync(account: Account, team: Team) {
    const epoch = this.epoch;
    const generation = team.generation;
    const started = this.clock.now();
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
        () => false,
      );
      const moved = (channel: string) =>
        old?.channelRevisions.get(channel) !== revisions.get(channel);
      const refreshRevisions = contentRevisions(
        old?.data,
        data,
        old?.channelRefreshRevisions ?? old?.channelRevisions ?? new Map(),
        this.degradedAdmission(team, moved),
      );
      if (!data.degraded) team.degraded.clear();
      for (const channel of team.degraded.keys())
        if (!data.channels.some((row) => row.id === channel))
          team.degraded.delete(channel);
      const changed = [...revisions].some(
        ([id, value]) => old?.channelRevisions.get(id) !== value,
      );
      this.report({
        kind: 'sync',
        store: team.store.id,
        milliseconds: this.clock.now() - started,
        changed,
        conversations: data.conversations.length,
        outcome: 'ok',
      });
      if (team.bumpedAt !== undefined) {
        this.report({
          kind: 'arrival',
          store: team.store.id,
          milliseconds: this.clock.now() - team.bumpedAt,
        });
        team.bumpedAt = undefined;
      }
      this.publish(team.store.id, {
        state: 'ready',
        blockedChannels: new Set(team.blocked),
        scope: reply.scope,
        data,
        error: '',
        note: data.read_retry_pending
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
      team.due = team.dirty
        ? 0
        : this.clock.now() +
          (account.failure || data.degraded ? RESYNC_SHORT_MS : RESYNC_IDLE_MS);
    } catch (cause) {
      const error = normalizeCommandError(cause);
      this.report({
        kind: 'sync',
        store: team.store.id,
        milliseconds: this.clock.now() - started,
        changed: false,
        conversations: 0,
        outcome: error.code === 'cancelled' ? 'cancelled' : 'error',
        code: error.code,
      });
      if (!this.valid(account, team, epoch, generation)) return;
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
    const started = this.clock.now();
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
      this.report({
        kind: 'poll',
        account: account.key,
        milliseconds: this.clock.now() - started,
        bumped: result.bumped,
        outcome: 'ok',
      });
      if (result.bumped)
        for (const t of account.teams.values()) t.bumpedAt ??= this.clock.now();
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
      // A poll that succeeds after failing is the recovery the short cadence
      // was being kept for: what changed while no poll was running was never
      // reported, so every team of the account resynchronizes now and only
      // then returns to the idle cadence.
      if (account.failure)
        for (const t of account.teams.values()) {
          t.dirty = true;
          t.due = 0;
        }
      account.failure = undefined;
      account.due = this.clock.now() + 250;
    } catch (cause) {
      const error = normalizeCommandError(cause);
      this.report({
        kind: 'poll',
        account: account.key,
        milliseconds: this.clock.now() - started,
        bumped: false,
        outcome: error.code === 'cancelled' ? 'cancelled' : 'error',
        code: error.code,
      });
      if (!this.valid(account, team, epoch, generation)) return;
      if (error.code === 'cancelled') {
        account.due = 0;
        return;
      }
      if (this.handleError(team.store.id, error)) return;
      // Nothing reports arrivals while the poll is down, so the teams of this
      // account go back to the short cadence rather than waiting out an idle
      // interval that was scheduled while the poll was working. It is a
      // clamp, not a reset: a team already due sooner keeps its time, and a
      // team backing off from its own failure keeps that.
      if (!account.failure)
        for (const t of account.teams.values())
          if (!t.quarantine && t.due !== Infinity)
            t.due = Math.min(t.due, this.clock.now() + RESYNC_SHORT_MS);
      account.failure = error;
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
