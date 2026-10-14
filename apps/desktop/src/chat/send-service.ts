import type { Bridge } from '../bridge';
import { normalizeCommandError } from '../bridge';
import type {
  ChatAction,
  ChatMessage,
  ChatOperation,
  ChatReply,
} from '../chat-contract';
import {
  CHAT_HISTORY_BYTES,
  CHAT_PENDING_ROWS,
  CHAT_TEXT_BYTES,
} from '../chat-limits';
import { chatAvailable, storeOf } from '../model';
import type { AgentSnapshot } from '../model';
import { failure, submissionId } from './actions';
import {
  chatClient,
  channelIntegrity,
  integrity,
  type ChatWorkPriority,
} from './client';
import { accessChanged, cancelled } from './errors';
import { eventFromReply } from './conversation-events';
import {
  delivered,
  deliveryUnconfirmed,
  operationActions,
  reconcileOperations,
  observeMessages,
  reportedOperation,
} from './operations';
import type { OperationActions, TrackedOperation } from './operations';
import { chatIntentPersistence } from './intent';
import type { ChatIntentPersistence } from './intent';
import { sameScope } from './scope';
import { systemChatClock } from './clock';
import type { ChatClock } from './clock';
import type { ChatInboxService } from './inbox-service';
import {
  copyOutgoing,
  intentHeld,
  move,
  newOutgoing,
  operationOf,
  outcomeUnknown,
  outgoingActions,
  preparationAmbiguous,
  running,
  terminal,
  unsent,
  deliveryOf,
  type OutgoingActions,
  type OutgoingMessage,
} from './send/outgoing';
import { DraftStore } from './send/drafts';
import { ChatRequestGateway, coalesce } from './send/gateway';
import { recover } from './send/recovery';
import { SendScheduler, teamHasWork } from './send/scheduler';
import { newTeam, retire, type Team } from './send/team';

type Inbox = Pick<
  ChatInboxService,
  | 'getSnapshot'
  | 'subscribe'
  | 'invalidate'
  | 'block'
  | 'blockChannel'
  | 'handleError'
  | 'isChannelBlocked'
>;
/**
 * One step of one outgoing message, timed from the moment it was submitted:
 * its intent saved, its operation prepared, its attempt answered, the message
 * observed in verified history, or a step failed. Never the text.
 */
export interface ChatSendTiming {
  store: string;
  message: string;
  step: 'saved' | 'prepared' | 'attempted' | 'observed' | 'failed';
  milliseconds: number;
  code?: string;
}
/**
 * How many messages one channel may hold behind the one it is sending. The
 * queue is this session's memory alone, so it is kept to a depth a person can
 * still account for when the window asks before discarding it.
 */
export const CHAT_QUEUE_ROWS = 64;
const problem = (message: string) => ({
  code: 'chat-limit',
  message,
  fatal: false,
  ambiguous: false,
  retryable: false,
});

export class ChatSendService {
  private snapshot?: AgentSnapshot;
  private generations: ReadonlyMap<string, number> = new Map();
  private teams = new Map<string, Team>();
  /**
   * Every team's drafts. A draft change notifies only its channel's
   * subscribers and does not publish, so typing neither re-renders the
   * service's subscribers nor re-arms the tick.
   */
  private draftStore = new DraftStore(CHAT_HISTORY_BYTES);
  private listeners = new Set<() => void>();
  private revision = 0;
  private active = false;
  private stopInbox?: () => void;
  private synchronizing = false;
  private observers = new Set<(event: ChatSendTiming) => void>();
  private gateway: ChatRequestGateway;
  private scheduler: SendScheduler;

  constructor(
    private bridge: Bridge,
    private inbox: Inbox,
    private clock: ChatClock = systemChatClock,
  ) {
    this.gateway = new ChatRequestGateway({
      current: (team, epoch) => this.current(team, epoch),
      available: (team) => this.available(team),
      readable: (team, channel) => this.readable(team, channel),
      writable: (team, channel) => this.writable(team, channel),
      channelBlocked: (team, channel) =>
        inbox.isChannelBlocked(team.storeId, channel),
      generation: (profile) => this.generation(profile),
      agentReady: () => this.snapshot?.agent.state === 'ready',
      inbox: {
        scope: (storeId) => inbox.getSnapshot().get(storeId)?.scope,
        handleError: (storeId, error, channel) =>
          inbox.handleError(storeId, error, channel),
        invalidate: (storeId) => inbox.invalidate(storeId),
      },
      apply: (team, reply, action) => this.apply(team, reply, action),
      failed: (team, action) => {
        if (action.action === 'attempt') {
          team.operations = team.operations.map((op) =>
            op.id === action.operation ? { ...op, statusUnknown: true } : op,
          );
        }
        this.publish();
      },
    });
    this.scheduler = new SendScheduler(clock, {
      active: () => this.active,
      teams: () => this.teams.values(),
      synchronize: () => this.synchronize(),
      outstanding: (team) => this.outstanding(team),
      current: (team) => this.current(team),
      run: (team) => this.background(team),
    });
  }

  /** Opt-in timings; an observer cannot change what the service does. */
  observe(observer: (event: ChatSendTiming) => void): () => void {
    this.observers.add(observer);
    return () => {
      this.observers.delete(observer);
    };
  }
  private report(
    team: Team,
    message: OutgoingMessage,
    step: ChatSendTiming['step'],
    code?: string,
  ) {
    const event = Object.freeze({
      store: team.storeId,
      message: message.id,
      step,
      milliseconds: Math.max(0, this.clock.now() - message.createdAt),
      ...(code ? { code } : {}),
    });
    for (const observer of this.observers) {
      try {
        observer(event);
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
  getSnapshot = () => this.revision;
  private publish() {
    this.revision++;
    // A send, a recovery or a failure changes what the tick has to do, and
    // each of them publishes; re-arming here brings the tick forward as soon
    // as there is work, without any of them having to ask.
    this.scheduler.arm();
    for (const listener of this.listeners) listener();
  }
  start() {
    if (this.active) return;
    this.active = true;
    this.stopInbox = this.inbox.subscribe(() => this.synchronize());
    this.synchronize();
    this.scheduler.arm(true);
  }
  /** Suspend agent work without discarding this window's unsaved queue. */
  pause() {
    this.active = false;
    this.stopInbox?.();
    this.stopInbox = undefined;
    this.scheduler.cancel();
    this.publish();
  }
  stop() {
    this.pause();
    for (const team of this.teams.values()) retire(team);
    this.draftStore.clearAll();
    this.teams.clear();
    this.scheduler.forget();
    this.publish();
  }
  update(
    snapshot: AgentSnapshot,
    generations: ReadonlyMap<string, number> = new Map(),
  ) {
    this.snapshot = snapshot;
    this.generations = generations;
    if (this.active) this.synchronize();
  }
  private ensure(storeId: string): Team {
    const existing = this.teams.get(storeId);
    if (existing) return existing;
    const store = this.snapshot && storeOf(this.snapshot, storeId);
    if (!store || store.kind !== 'team') throw cancelled();
    const team = newTeam({
      storeId,
      profile: store.server,
      scope: this.inbox.getSnapshot().get(storeId)?.scope ?? null,
      client: chatClient(this.bridge, store.server, storeId),
    });
    this.teams.set(storeId, team);
    return team;
  }
  private current(team: Team, epoch = team.epoch): boolean {
    return (
      this.active &&
      this.teams.get(team.storeId) === team &&
      team.epoch === epoch &&
      !team.blocked
    );
  }
  private available(team: Team): boolean {
    const store = this.snapshot && storeOf(this.snapshot, team.storeId);
    return Boolean(
      this.snapshot &&
      store &&
      chatAvailable(this.snapshot, store, {
        nowSeconds: this.clock.now() / 1000,
      }),
    );
  }
  private readable(team: Team, channel: string): boolean {
    return (
      Boolean(
        this.inbox
          .getSnapshot()
          .get(team.storeId)
          ?.data?.channels.some((c) => c.id === channel && c.readable),
      ) && !this.inbox.isChannelBlocked(team.storeId, channel)
    );
  }
  private writable(team: Team, channel: string): boolean {
    return Boolean(
      this.inbox
        .getSnapshot()
        .get(team.storeId)
        ?.data?.channels.some((c) => c.id === channel && c.writable),
    );
  }
  private generation(profile: string): number {
    return this.generations.get(profile) ?? 0;
  }
  private synchronize() {
    if (!this.active || this.synchronizing || !this.snapshot) return;
    this.synchronizing = true;
    let changed = false;
    let created = false;
    try {
      for (const [id, entry] of this.inbox.getSnapshot()) {
        const store = storeOf(this.snapshot, id);
        if (store?.kind === 'team' && entry.scope && !this.teams.has(id)) {
          this.ensure(id);
          created = true;
        }
      }
      for (const [id, team] of this.teams) {
        const entry = this.inbox.getSnapshot().get(id);
        const store = storeOf(this.snapshot, id);
        const removed =
          !store &&
          (this.snapshot.profileInventory.find(
            (p) => p.profile === team.profile,
          )?.teams === 'complete' ||
            (this.snapshot.profileInventoryStatus === 'complete' &&
              !this.snapshot.catalogProfiles.includes(team.profile)));
        const replaced =
          entry?.scope && team.scope && !sameScope(entry.scope, team.scope);
        const rejected =
          entry?.state === 'blocked' ||
          this.snapshot.servers.find((s) => s.profileName === team.profile)
            ?.trust.status === 'blocked';
        if (removed || replaced || rejected) {
          retire(team);
          this.draftStore.clear(id);
          if (replaced && !rejected)
            this.inbox.block(
              id,
              'The chat identity changed. Lock and unlock to revalidate.',
            );
          if (removed) this.teams.delete(id);
          changed = true;
          continue;
        }
        if (!team.scope && entry?.scope) {
          team.scope = entry.scope;
          changed = true;
        }
        if (entry?.state === 'ready' && entry.data) {
          const revoked = new Set(
            entry.data.channels
              .filter((c) => !c.readable || entry.blockedChannels.has(c.id))
              .map((c) => c.id),
          );
          for (const channel of revoked) {
            this.draftStore.delete(id, channel);
            team.loaded.delete(channel);
            // A queued message exists only in this session, and a revoked
            // channel takes its text with the drafts. Nothing is left to send
            // or to recover, so the row goes rather than remaining as a
            // message that can never leave.
            const queued = (m: OutgoingMessage) =>
              m.channel === channel && m.delivery.stage === 'queued';
            if (team.messages.some(queued)) {
              team.messages = team.messages.filter((m) => !queued(m));
              changed = true;
            }
            for (const [id, observed] of team.observations)
              if (observed.channel === channel) team.observations.delete(id);
            for (const message of team.messages.filter(
              (m) => m.channel === channel,
            )) {
              if (message.text !== undefined) {
                move(message, { type: 'revoked' });
                changed = true;
              }
            }
            team.operations = team.operations.map((op) =>
              op.channel === channel ? { ...op, text: undefined } : op,
            );
          }
        }
      }
    } finally {
      this.synchronizing = false;
    }
    if (changed) this.publish();
    // A team first known here has never been recovered, so it is work for
    // the tick. Its record alters nothing a subscriber reads, so it does not
    // publish; it only brings the tick forward from the idle wait.
    else if (created) this.scheduler.arm();
  }
  /**
   * Removes the team's drafts for channels other than `live`. The unlocked
   * shell owns the drafts, so they outlive conversation and team navigation;
   * a channel that is no longer listed cannot be returned to, so its draft is
   * not held for an address that no longer resolves.
   */
  retainDrafts(storeId: string, live: ReadonlySet<string>) {
    this.draftStore.retain(storeId, live);
  }
  draft(storeId: string, channel: string): string {
    return this.draftStore.get(storeId, channel);
  }
  /** The UTF-8 length of the channel's draft, as the draft quota counts it. */
  draftBytes(storeId: string, channel: string): number {
    return this.draftStore.size(storeId, channel);
  }
  /**
   * Calls `listener` whenever the channel's draft changes. Draft changes are
   * not publications: {@link subscribe} does not report them.
   */
  subscribeDraft(storeId: string, channel: string, listener: () => void) {
    return this.draftStore.subscribe(storeId, channel, listener);
  }
  setDraft(storeId: string, channel: string, text: string) {
    const team = this.ensure(storeId);
    if (team.blocked) throw integrity();
    if (!this.draftStore.set(storeId, channel, text))
      throw problem(
        'Draft storage is full. Send or clear another draft before adding more text.',
      );
  }
  messages(storeId: string, channel?: string): readonly OutgoingMessage[] {
    return (this.teams.get(storeId)?.messages ?? [])
      .filter((m) => channel === undefined || m.channel === channel)
      .map(copyOutgoing);
  }
  /** Copies of the team's tracked operations, like {@link messages}. */
  operations(storeId: string): TrackedOperation[] {
    return (this.teams.get(storeId)?.operations ?? []).map((op) => ({
      ...op,
      confirmation: op.confirmation ? { ...op.confirmation } : null,
    }));
  }
  /**
   * How many submissions this session is the only copy of: the queued
   * messages, and the ones whose save the agent has not acknowledged. Every
   * other outgoing message is either a prepared agent operation or a saved
   * intent, both of which the next session recovers. Closing the window
   * discards these, which is what the window asks about first.
   */
  unsentCount(): number {
    let count = 0;
    for (const team of this.teams.values())
      for (const message of team.messages) if (unsent(message)) count++;
    return count;
  }
  loadError(storeId: string, channel: string): string {
    return this.teams.get(storeId)?.loadErrors.get(channel) ?? '';
  }
  cleanupError(storeId: string): string {
    return this.teams.get(storeId)?.cleanupError ?? '';
  }
  canSubmit(storeId: string, channel: string): boolean {
    const team = this.teams.get(storeId);
    return Boolean(
      team &&
      this.current(team) &&
      this.available(team) &&
      team.scope &&
      team.loaded.has(channel) &&
      !team.loadErrors.has(channel) &&
      this.queueDepth(team, channel) < CHAT_QUEUE_ROWS,
    );
  }
  /** How many of this channel's submissions are waiting behind another. */
  private queueDepth(team: Team, channel: string): number {
    return team.messages.filter(
      (m) => m.channel === channel && m.delivery.stage === 'queued',
    ).length;
  }
  /** Whether this channel's queue is at the depth the composer stops at. */
  queueFull(storeId: string, channel: string): boolean {
    const team = this.teams.get(storeId);
    return Boolean(team && this.queueDepth(team, channel) >= CHAT_QUEUE_ROWS);
  }
  /**
   * Whether this message is the one its channel may work on now. The agent
   * keeps one saved message per channel, so a submission waits for the one
   * ahead of it to reach an operation and have its saved copy cleared. A
   * message the agent has already prepared is ordered by the agent, not here,
   * and a message already holding the channel's saved copy is the only one
   * that can be holding it, wherever it sits in the list.
   */
  private admitted(team: Team, message: OutgoingMessage): boolean {
    if (operationOf(message) || intentHeld(message)) return true;
    return (
      this.head(team, message.channel) === message &&
      !team.messages.some(
        (m) => m !== message && m.channel === message.channel && intentHeld(m),
      )
    );
  }
  /**
   * The channel's oldest message the agent has not prepared and this session
   * can still send. A message whose text went with the channel's access can
   * never leave, so the queue is not held up on its behalf.
   */
  private head(team: Team, channel: string): OutgoingMessage | undefined {
    return team.messages.find(
      (m) => m.channel === channel && !operationOf(m) && m.text !== undefined,
    );
  }
  /**
   * Starts the channel's next queued message once the one ahead of it is out
   * of the way. Only the oldest unprepared message is ever driven, so the
   * queue reaches the agent in the order the messages were sent in.
   */
  private drain(
    team: Team,
    channel: string,
    priority: ChatWorkPriority = 'foreground',
  ) {
    if (!this.current(team)) return;
    const next = this.head(team, channel);
    if (next?.delivery.stage !== 'queued' || !this.admitted(team, next)) return;
    void this.drive(team, next, priority);
  }
  private persistence(
    team: Team,
    channel: string,
    priority: ChatWorkPriority = 'foreground',
  ): ChatIntentPersistence {
    if (!team.scope) throw cancelled();
    return chatIntentPersistence(
      this.bridge,
      team.storeId,
      team.scope,
      channel,
      team.client,
      priority,
    );
  }
  open(
    storeId: string,
    channel: string,
    retry = false,
    priority: ChatWorkPriority = 'foreground',
  ): Promise<void> {
    const team = this.ensure(storeId);
    if (team.loading.has(channel)) return team.loading.get(channel)!;
    if (team.loaded.has(channel) && !retry) return Promise.resolve();
    if (
      !team.scope ||
      !this.current(team) ||
      !this.available(team) ||
      !this.readable(team, channel)
    )
      return Promise.resolve();
    const epoch = team.epoch;
    const work = (async () => {
      try {
        const saved = await this.persistence(team, channel, priority).load();
        if (!this.current(team, epoch) || !this.readable(team, channel)) return;
        team.loadErrors.delete(channel);
        team.loaded.add(channel);
        if (
          saved &&
          !team.messages.some((m) => m.submission === saved.submission)
        ) {
          if (!this.canRetain(saved.text))
            throw problem(
              'Resolve pending messages before loading more saved messages.',
            );
          // It was read back out of the agent's own store, so it is saved and
          // its copy there is this channel's until cleanup clears it. Whether
          // an earlier session submitted it is unknown, so it is prepared, not
          // submitted again.
          const message = newOutgoing({
            channel,
            submission: saved.submission,
            text: saved.text,
            createdAt: this.clock.now(),
            delivery: { stage: 'preparing', ambiguous: true },
            intent: 'pending',
          });
          team.messages.push(message);
          void this.drive(team, message, priority);
        }
      } catch (error) {
        if (this.current(team, epoch))
          team.loadErrors.set(channel, failure(error));
      } finally {
        team.loading.delete(channel);
        if (this.current(team, epoch)) this.publish();
      }
    })();
    team.loading.set(channel, work);
    return work;
  }
  private canRetain(text: string): boolean {
    const messages = [...this.teams.values()].flatMap((team) =>
      team.messages.filter((m) => !m.observed || intentHeld(m)),
    );
    return (
      messages.length < CHAT_PENDING_ROWS &&
      messages.reduce(
        (sum, m) => sum + new TextEncoder().encode(m.text ?? '').length,
        0,
      ) +
        new TextEncoder().encode(text).length <=
        CHAT_HISTORY_BYTES
    );
  }
  async submit(storeId: string, channel: string, text: string): Promise<void> {
    const team = this.ensure(storeId);
    if (!text.trim() || new TextEncoder().encode(text).length > CHAT_TEXT_BYTES)
      throw problem('The message is empty or exceeds the text limit.');
    if (!this.canSubmit(storeId, channel))
      throw problem(
        this.queueFull(storeId, channel)
          ? 'Too many messages are waiting to be sent in this channel. Your draft is kept.'
          : 'This conversation cannot accept messages yet. Your draft is kept.',
      );
    if (!this.writable(team, channel))
      throw problem('You cannot send messages to this channel.');
    team.messages = team.messages.filter((m) => !m.observed || intentHeld(m));
    if (!this.canRetain(text))
      throw problem(
        'Resolve pending messages before sending more. Your draft is kept.',
      );
    const submission = submissionId();
    const message = newOutgoing({
      channel,
      submission,
      text,
      createdAt: this.clock.now(),
    });
    team.messages.push(message);
    if (this.draftStore.get(storeId, channel) === text)
      this.draftStore.delete(storeId, channel);
    this.publish();
    // A channel already working on a message queues this one and returns at
    // once, so the composer is free for the next message either way.
    await this.drive(team, message);
  }
  private apply(team: Team, reply: ChatReply, action: ChatAction) {
    if (
      reply.result.kind === 'operation' &&
      (action.action === 'prepare-message' ||
        action.action === 'submit-message')
    ) {
      const message = team.messages.find(
        (m) => m.submission === action.submission,
      );
      if (message) {
        const id = reply.result.operation.id;
        move(message, { type: 'status', op: reply.result.operation });
        const duplicate = team.messages.find(
          (m) => m !== message && operationOf(m)?.id === id,
        );
        if (duplicate?.observed) move(message, { type: 'observed' });
        team.messages = team.messages.filter(
          (m) => m === message || operationOf(m)?.id !== id,
        );
      }
    }
    const result = reply.result;
    if (
      result.kind === 'operation-body' &&
      result.text !== null &&
      this.canRetain(result.text)
    ) {
      const operation = team.operations.find(
        (op) => op.id === result.operation,
      );
      const message = team.messages.find(
        (m) => operationOf(m)?.id === result.operation,
      );
      if (message) move(message, { type: 'body', text: result.text });
      else if (operation)
        team.messages.push(
          newOutgoing({
            id: operation.id,
            channel: result.channel,
            submission: '',
            text: result.text,
            createdAt: this.clock.now(),
            automatic: false,
            delivery: deliveryOf(reportedOperation(operation)),
          }),
        );
    }
    if (reply.result.kind === 'cleanup-pending') {
      for (const operation of reply.result.operations) {
        team.operations = reconcileOperations(team.operations, {
          kind: 'operation',
          operation,
          discard: false,
        });
      }
    } else if (reply.result.kind !== 'history') {
      const event = eventFromReply(action, reply.result);
      if (
        event.kind === 'operation' &&
        event.operation.kind === 'send-message' &&
        event.operation.state === 'confirmed'
      )
        event.discard = false;
      team.operations = reconcileOperations(team.operations, event);
    }
    // The operations this reply reports. Requests on the chat lane run one at
    // a time and each reply reports the agent's state when its request ran,
    // so a report is never older than the state a message holds, even when a
    // foreground request overtook a background one. The transition still
    // ignores a report that would reopen a terminal operation.
    const reported = new Map(
      (reply.result.kind === 'operation'
        ? [reply.result.operation]
        : reply.result.kind === 'pending' ||
            reply.result.kind === 'cleanup-pending'
          ? reply.result.operations
          : []
      ).map((op) => [op.id, op]),
    );
    for (const message of team.messages) {
      const id = operationOf(message)?.id;
      if (id === undefined) continue;
      const op = reported.get(id);
      if (op) move(message, { type: 'status', op });
      const observed = team.observations.get(id);
      if (observed) this.observeMessage(team, message, observed);
      if (
        !message.observed &&
        team.operations.some((entry) => entry.id === id && entry.observed)
      )
        move(message, { type: 'observed' });
    }
    for (const [id, observed] of team.observations) {
      if (
        !team.messages.some(
          (m) => m.channel === observed.channel && !operationOf(m),
        )
      )
        team.observations.delete(id);
    }
    this.publish();
  }
  /**
   * Sends a chat request for the team through the gateway, which coalesces,
   * authorizes and reconciles it. The conversation sends every action other
   * than history reads through here.
   */
  request(
    storeId: string,
    action: ChatAction,
    priority: ChatWorkPriority = 'foreground',
  ): Promise<ChatReply> {
    return this.gateway.request(this.ensure(storeId), action, priority);
  }
  private clearIntent(team: Team, message: OutgoingMessage): Promise<void> {
    return coalesce(team.clears, message.id, () =>
      this.removeIntent(team, message),
    );
  }
  private async removeIntent(team: Team, message: OutgoingMessage) {
    if (!intentHeld(message)) return;
    const epoch = team.epoch;
    try {
      await this.persistence(team, message.channel).clear(message.submission);
      if (!this.current(team, epoch)) return;
      move(message, { type: 'intent-cleared' });
    } catch (error) {
      if (this.current(team, epoch))
        move(message, { type: 'cleanup-failed', error: failure(error) });
    }
    if (this.current(team, epoch)) {
      this.publish();
      // The channel's one saved message is free again, so the next queued
      // message can take it.
      this.drain(team, message.channel);
    }
  }
  private async drive(
    team: Team,
    message: OutgoingMessage,
    priority: ChatWorkPriority = 'foreground',
  ) {
    const request = (action: ChatAction) =>
      this.request(team.storeId, action, priority);
    if (running(message) || !this.current(team)) return;
    if (
      // A message with no text and no operation can never be sent, so it is
      // neither queued nor something the queue waits behind; driving it
      // reports that below rather than leaving a row that reads as waiting.
      (operationOf(message) || message.text !== undefined) &&
      !this.admitted(team, message)
    ) {
      // Behind another message in this channel. It waits rather than failing;
      // the channel's drain starts it as soon as the one ahead is out of the
      // way, so the messages reach the agent in the order they were sent in.
      move(message, { type: 'queued' });
      this.publish();
      return;
    }
    if (!this.available(team) || !this.readable(team, message.channel)) {
      move(message, { type: 'waiting' });
      this.publish();
      return;
    }
    const epoch = team.epoch;
    const generation = this.generation(team.profile);
    const checking = (op: ChatOperation) =>
      message.observed ||
      message.delivery.stage === 'uncertain' ||
      this.unconfirmed(team, op);
    const prepared = operationOf(message);
    move(message, {
      type: 'started',
      step: !prepared ? 'save' : checking(prepared) ? 'check' : 'deliver',
    });
    // Every later event of this drive carries its token, so nothing this
    // drive does after a newer one has started can change the message.
    const token = message.attempt.token;
    this.publish();
    try {
      if (!prepared) {
        if (message.text === undefined) throw cancelled();
        move(message, { type: 'saving', token });
        await this.persistence(team, message.channel, priority).save({
          submission: message.submission,
          text: message.text,
        });
        if (!this.current(team, epoch)) return;
        // Acknowledged: the agent now holds a copy the next session recovers,
        // so closing the window no longer loses this submission.
        move(message, { type: 'saved', token });
        if (generation !== this.generation(team.profile))
          throw accessChanged(
            'Chat access changed. Your saved message has been kept.',
            'before',
          );
        this.report(team, message, 'saved');
        // A submit whose reply was lost may have reached the agent, so every
        // later preparation of this submission only asks it to prepare.
        const action = preparationAmbiguous(message)
          ? 'prepare-message'
          : 'submit-message';
        move(message, { type: 'preparing', token });
        this.publish();
        const reply = await request({
          action,
          submission: message.submission,
          channel: message.channel,
          text: message.text,
        });
        if (!this.current(team, epoch)) return;
        if (reply.result.kind !== 'operation') throw integrity();
        const op = reply.result.operation;
        move(message, {
          type: 'prepared',
          token,
          op,
          check: checking(op),
        });
        this.report(team, message, 'prepared');
      }
      const op = operationOf(message)!;
      this.publish();
      // Admit delivery before asynchronous cleanup to the shared chat lane.
      // A slow clear acknowledgement must not hold up the send it follows.
      const delivery =
        op.state === 'prepared' || op.state === 'uncertain'
          ? request({
              action:
                message.attempt.step === 'check' ? 'reconcile' : 'attempt',
              operation: op.id,
            })
          : undefined;
      void this.clearIntent(team, message);
      if (delivery) {
        const reply = await delivery;
        if (this.current(team, epoch) && reply.result.kind === 'operation') {
          move(message, {
            type: 'attempted',
            token,
            op: reply.result.operation,
          });
          this.report(team, message, 'attempted');
        }
      }
      if (this.current(team, epoch)) this.inbox.invalidate(team.storeId);
    } catch (error) {
      if (!this.current(team, epoch)) return;
      const typed = normalizeCommandError(error);
      this.report(team, message, 'failed', typed.code);
      // Nothing is cleared here. A failed save keeps the same text and
      // submission ID for an idempotent save retry, and a failed preparation
      // is ambiguous, so its saved copy stays until an operation is known.
      move(message, {
        type: 'failed',
        token,
        error: failure(error),
        access: typed.code === 'access-changed',
      });
      const op = operationOf(message);
      // The status reply is applied like any other operation report; this
      // drive keeps no continuation that could outlive a later attempt.
      if (op)
        void request({ action: 'status', operation: op.id }).catch(() => {});
    } finally {
      move(message, { type: 'finished', token });
      if (this.current(team, epoch)) {
        this.publish();
        // Whatever this attempt ended as, the channel may have room for the
        // message behind it now.
        this.drain(team, message.channel, priority);
      }
    }
  }
  async retry(storeId: string, id: string): Promise<void> {
    const team = this.ensure(storeId);
    const message = team.messages.find((m) => m.id === id);
    if (!message || running(message)) return;
    const op = operationOf(message);
    if (op?.state === 'confirmed') {
      await this.clearIntent(team, message);
      return;
    }
    if (op && terminal(op)) return;
    await this.drive(team, message);
  }
  /**
   * Whether `op` may already have reached the server. The state comes from
   * `op`, which a message may hold; the tracked operation adds what this
   * device learned outside any message: an unknown status after a failed
   * request, or an observation in accepted history.
   */
  private unconfirmed(team: Team, op: ChatOperation): boolean {
    const tracked = team.operations.find((o) => o.id === op.id);
    return deliveryUnconfirmed({
      state: op.state,
      observed: tracked?.observed,
      statusUnknown: tracked?.statusUnknown,
    });
  }
  /**
   * What the row of an outgoing message offers. Unlike `outgoingActions` on
   * the message alone, this includes the tracked operation's unknown status,
   * so the row offers what a drive of the message would send.
   */
  outgoingActions(storeId: string, message: OutgoingMessage): OutgoingActions {
    const team = this.teams.get(storeId);
    const op = operationOf(message);
    return outgoingActions(
      message,
      !!team && !!op && this.unconfirmed(team, op),
    );
  }
  /**
   * The requests a tracked operation accepts, for a row that shows the
   * operation rather than an outgoing message. While a message that carries
   * the operation is being driven, that drive sends its requests and the row
   * offers none.
   */
  operationActions(storeId: string, id: string): OperationActions {
    const team = this.teams.get(storeId);
    const op = team?.operations.find((o) => o.id === id);
    if (
      !team ||
      !op ||
      team.messages.some((m) => operationOf(m)?.id === id && running(m))
    )
      return { cancel: false };
    return operationActions(op);
  }
  /**
   * Sends the delivery request {@link operationActions} offers for a tracked
   * operation: an attempt, or a reconcile when the operation may already have
   * reached the server. Sends nothing when the operation accepts neither, and
   * resolves to whether a request was sent.
   */
  deliverOperation(storeId: string, id: string): Promise<boolean> {
    return this.operationRequest(
      storeId,
      id,
      this.operationActions(storeId, id).deliver,
    );
  }
  /**
   * Cancels a tracked operation if it accepts cancellation, and resolves to
   * whether a request was sent.
   */
  cancelOperation(storeId: string, id: string): Promise<boolean> {
    return this.operationRequest(
      storeId,
      id,
      this.operationActions(storeId, id).cancel ? 'cancel' : undefined,
    );
  }
  private async operationRequest(
    storeId: string,
    id: string,
    action: 'attempt' | 'reconcile' | 'cancel' | undefined,
  ): Promise<boolean> {
    if (!action) return false;
    try {
      await this.request(storeId, { action, operation: id });
      return true;
    } catch (error) {
      // A failed request leaves the operation's state unknown to this device,
      // so its status is read again before the failure is reported. The reply
      // is applied like any other operation report.
      await this.request(storeId, { action: 'status', operation: id }).catch(
        () => {},
      );
      throw error;
    }
  }
  async restoreDraft(storeId: string, id: string): Promise<void> {
    const team = this.ensure(storeId);
    const message = team.messages.find((m) => m.id === id);
    const carried = message && operationOf(message);
    if (
      !message ||
      running(message) ||
      outcomeUnknown(message) ||
      (carried && this.unconfirmed(team, carried))
    )
      throw problem('Check the original message before editing it.');
    if (this.draftStore.get(storeId, message.channel))
      throw problem(
        'Your current draft is kept. Clear it before restoring this message.',
      );
    const prepared = operationOf(message);
    if (prepared?.state === 'prepared')
      await this.request(storeId, {
        action: 'cancel',
        operation: prepared.id,
      });
    const op = operationOf(message);
    if (op && !terminal(op))
      throw problem('This message may already have been sent.');
    await this.clearIntent(team, message);
    if (intentHeld(message)) return;
    if (message.text) this.setDraft(storeId, message.channel, message.text);
    team.messages = team.messages.filter((m) => m !== message);
    this.publish();
    this.drain(team, message.channel);
  }
  private observeMessage(
    team: Team,
    message: OutgoingMessage,
    observed: ChatMessage,
  ) {
    if (
      (message.text !== undefined &&
        (observed.content.kind !== 'text' ||
          message.text !== observed.content.text)) ||
      (observed.sender !== null && observed.sender !== team.scope?.actor)
    )
      throw channelIntegrity(
        'The sent message does not match verified history.',
      );
    if (!message.observed) this.report(team, message, 'observed');
    move(message, { type: 'observed' });
    team.operations = observeMessages(team.operations, new Set([observed.id]));
  }
  observeHistory(
    storeId: string,
    channel: string,
    rows: readonly ChatMessage[],
  ) {
    const team = this.teams.get(storeId);
    if (!team || !this.current(team)) return;
    const ids = delivered(rows);
    const messages = rows.filter((m) => ids.has(m.id));
    if (team.messages.some((m) => m.channel === channel && !operationOf(m))) {
      for (const message of messages)
        team.observations.set(message.id, {
          ...message,
          channel,
          content: { ...message.content },
        });
      let bytes = [...team.observations.values()].reduce(
        (sum, m) =>
          sum +
          (m.content.kind === 'text'
            ? new TextEncoder().encode(m.content.text).length
            : 0),
        0,
      );
      for (const [id, message] of team.observations) {
        if (
          team.observations.size <= CHAT_PENDING_ROWS &&
          bytes <= CHAT_HISTORY_BYTES
        )
          break;
        bytes -=
          message.content.kind === 'text'
            ? new TextEncoder().encode(message.content.text).length
            : 0;
        team.observations.delete(id);
      }
    }
    for (const message of team.messages) {
      const op = operationOf(message);
      if (message.channel !== channel || !op || !ids.has(op.id)) continue;
      this.observeMessage(
        team,
        message,
        messages.find((m) => m.id === op.id)!,
      );
    }
    team.operations = observeMessages(team.operations, ids);
    this.publish();
  }
  messageKey(storeId: string, operation: string): string {
    return (
      this.teams
        .get(storeId)
        ?.messages.find((m) => operationOf(m)?.id === operation)?.id ??
      operation
    );
  }
  refresh(
    storeId: string,
    priority: ChatWorkPriority = 'foreground',
  ): Promise<void> {
    const team = this.ensure(storeId);
    if (team.refresh) return team.refresh;
    const work = recover(team, {
      current: (epoch) => this.current(team, epoch),
      deliverable: (channel) =>
        this.available(team) && this.readable(team, channel),
      now: () => this.clock.now(),
      request: (action) => this.request(team.storeId, action, priority),
      drain: (channel) => this.drain(team, channel, priority),
      drive: (message) => this.drive(team, message, priority),
      clearIntent: (message) => this.clearIntent(team, message),
      publish: () => this.publish(),
    });
    team.refresh = work;
    void work
      .finally(() => {
        if (team.refresh === work) team.refresh = undefined;
      })
      .catch(() => {});
    return work;
  }
  /** Whether the scheduler has anything to do for this team. */
  private outstanding(team: Team): boolean {
    return (
      this.current(team) &&
      teamHasWork(
        team,
        () =>
          this.available(team) &&
          (
            this.inbox.getSnapshot().get(team.storeId)?.data?.channels ?? []
          ).some((c) => c.readable && !team.loaded.has(c.id)),
      )
    );
  }
  /**
   * The scheduler's background pass: opens up to four readable channels not
   * opened yet, so their saved messages are recovered, then recovers the
   * team.
   */
  private async background(team: Team) {
    if (this.available(team)) {
      const channels =
        this.inbox.getSnapshot().get(team.storeId)?.data?.channels ?? [];
      for (const channel of channels
        .filter((c) => c.readable && !team.loaded.has(c.id))
        .slice(0, 4))
        await this.open(team.storeId, channel.id, false, 'background');
    }
    if (!this.current(team)) return;
    await this.refresh(team.storeId, 'background');
  }
}
