import type { Bridge } from '../bridge';
import { normalizeCommandError } from '../bridge';
import type {
  ChatAction,
  ChatMessage,
  ChatOperation,
  ChatReply,
  ChatScope,
} from '../chat-contract';
import {
  CHAT_HISTORY_BYTES,
  CHAT_PENDING_ROWS,
  CHAT_TEXT_BYTES,
} from '../chat-limits';
import { chatAvailable, storeOf } from '../model';
import type { AgentSnapshot } from '../model';
import { failure, preparationCanChange, submissionId } from './actions';
import { chatClient, channelIntegrity, integrity } from './client';
import { cancelled } from './errors';
import { eventFromReply } from './conversation-events';
import { reconcileOperations, observeMessages } from './operations';
import type { TrackedOperation } from './operations';
import { chatIntentPersistence } from './intent';
import type { ChatIntentPersistence } from './intent';
import { sameScope } from './scope';
import { systemChatClock } from './inbox-service';
import type { ChatClock, ChatInboxService } from './inbox-service';

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
export type SendPhase =
  | 'saving'
  | 'preparing'
  | 'sending'
  | 'not-sent'
  | 'unconfirmed'
  | 'sent'
  | 'cancelled'
  | 'paused';
export interface OutgoingMessage {
  id: string;
  channel: string;
  submission: string;
  text?: string;
  phase: SendPhase;
  operation?: ChatOperation;
  observed: boolean;
  running: boolean;
  intentPending: boolean;
  ambiguousPreparation: boolean;
  automatic: boolean;
  error: string;
  cleanupError: string;
  createdAt: number;
}
interface Team {
  storeId: string;
  profile: string;
  scope: ChatScope | null;
  client: ReturnType<typeof chatClient>;
  epoch: number;
  blocked: boolean;
  operations: TrackedOperation[];
  observations: Map<string, ChatMessage & { channel: string }>;
  messages: OutgoingMessage[];
  drafts: Map<string, string>;
  loaded: Set<string>;
  loading: Map<string, Promise<void>>;
  loadErrors: Map<string, string>;
  checks: Map<string, { due: number; failures: number }>;
  requests: Map<string, Promise<ChatReply>>;
  clears: Map<string, Promise<void>>;
  refresh?: Promise<void>;
  due: number;
  cleanupError: string;
}
const localActions = new Set([
  'pending',
  'status',
  'cleanup-pending',
  'cancel',
  'finalize',
]);
const terminal = (op: ChatOperation) =>
  ['confirmed', 'cancelled', 'rejected'].includes(op.state);
const phaseOf = (op: ChatOperation): SendPhase =>
  op.state === 'confirmed'
    ? 'sent'
    : op.state === 'uncertain'
      ? 'unconfirmed'
      : op.state === 'cancelled'
        ? 'cancelled'
        : 'not-sent';
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
  private listeners = new Set<() => void>();
  private revision = 0;
  private active = false;
  private stopInbox?: () => void;
  private timer: unknown;
  private cursor = 0;
  private recoveryProfiles = new Set<string>();
  private synchronizing = false;

  constructor(
    private bridge: Bridge,
    private inbox: Inbox,
    private clock: ChatClock = systemChatClock,
  ) {}

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  getSnapshot = () => this.revision;
  private publish() {
    this.revision++;
    for (const listener of this.listeners) listener();
  }
  start() {
    if (this.active) return;
    this.active = true;
    this.stopInbox = this.inbox.subscribe(() => this.synchronize());
    this.synchronize();
    this.arm();
  }
  stop() {
    this.active = false;
    this.stopInbox?.();
    this.clock.cancel(this.timer);
    for (const team of this.teams.values()) {
      team.epoch++;
      team.client.dispose();
      team.drafts.clear();
      team.observations.clear();
      for (const message of team.messages) message.text = undefined;
      team.messages.length = 0;
      team.operations.length = 0;
    }
    this.teams.clear();
    this.recoveryProfiles = new Set();
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
    const team: Team = {
      storeId,
      profile: store.server,
      scope: this.inbox.getSnapshot().get(storeId)?.scope ?? null,
      client: chatClient(this.bridge, store.server, storeId),
      epoch: 0,
      blocked: false,
      operations: [],
      observations: new Map(),
      messages: [],
      drafts: new Map(),
      loaded: new Set(),
      loading: new Map(),
      loadErrors: new Map(),
      checks: new Map(),
      requests: new Map(),
      clears: new Map(),
      due: 0,
      cleanupError: '',
    };
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
  private synchronize() {
    if (!this.active || this.synchronizing || !this.snapshot) return;
    this.synchronizing = true;
    let changed = false;
    try {
      for (const [id, entry] of this.inbox.getSnapshot()) {
        const store = storeOf(this.snapshot, id);
        if (store?.kind === 'team' && entry.scope) this.ensure(id);
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
          this.snapshot.servers.find((s) => s.id === team.profile)?.trust
            .status === 'blocked';
        if (removed || replaced || rejected) {
          team.epoch++;
          team.client.dispose();
          team.drafts.clear();
          team.observations.clear();
          team.loaded.clear();
          team.loadErrors.clear();
          for (const message of team.messages) message.text = undefined;
          team.messages = [];
          team.operations = [];
          team.blocked = true;
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
            if (team.drafts.delete(channel)) changed = true;
            team.loaded.delete(channel);
            for (const [id, observed] of team.observations)
              if (observed.channel === channel) team.observations.delete(id);
            for (const message of team.messages.filter(
              (m) => m.channel === channel,
            )) {
              if (message.text !== undefined) {
                message.text = undefined;
                message.phase = 'paused';
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
  }
  /**
   * The team's unsent messages, keyed by channel. The unlocked shell owns the
   * map, so drafts outlive conversation and team navigation.
   */
  drafts(storeId: string): Map<string, string> {
    return this.ensure(storeId).drafts;
  }
  draft(storeId: string, channel: string): string {
    return this.teams.get(storeId)?.drafts.get(channel) ?? '';
  }
  setDraft(storeId: string, channel: string, text: string) {
    const team = this.ensure(storeId);
    if (team.blocked) throw integrity();
    const bytes = new TextEncoder().encode(text).length;
    const kept = [...this.teams.values()].reduce(
      (sum, t) =>
        sum +
        [...t.drafts].reduce(
          (n, [id, body]) =>
            n +
            (t === team && id === channel
              ? 0
              : new TextEncoder().encode(body).length),
          0,
        ),
      0,
    );
    if (bytes + kept > CHAT_HISTORY_BYTES)
      throw problem(
        'Draft storage is full. Send or clear another draft before adding more text.',
      );
    if (text) team.drafts.set(channel, text);
    else team.drafts.delete(channel);
    this.publish();
  }
  messages(storeId: string, channel?: string): readonly OutgoingMessage[] {
    return (this.teams.get(storeId)?.messages ?? [])
      .filter((m) => channel === undefined || m.channel === channel)
      .map((m) => ({
        ...m,
        operation: m.operation
          ? {
              ...m.operation,
              receipt: m.operation.receipt ? { ...m.operation.receipt } : null,
            }
          : undefined,
      }));
  }
  operations(storeId: string): TrackedOperation[] {
    return this.teams.get(storeId)?.operations ?? [];
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
      !team.messages.some((m) => m.channel === channel && m.intentPending),
    );
  }
  private persistence(team: Team, channel: string): ChatIntentPersistence {
    if (!team.scope) throw cancelled();
    return chatIntentPersistence(
      this.bridge,
      team.storeId,
      team.scope,
      channel,
    );
  }
  open(storeId: string, channel: string, retry = false): Promise<void> {
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
        const saved = await this.persistence(team, channel).load();
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
          const message = this.newMessage(
            channel,
            saved.text,
            saved.submission,
          );
          message.ambiguousPreparation = true;
          team.messages.push(message);
          void this.drive(team, message);
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
      team.messages.filter(
        (m) => !m.observed || m.intentPending || m.cleanupError,
      ),
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
  private newMessage(
    channel: string,
    text: string,
    submission = submissionId(),
  ): OutgoingMessage {
    return {
      id: submission,
      submission,
      channel,
      text,
      phase: 'saving',
      observed: false,
      running: false,
      intentPending: true,
      ambiguousPreparation: false,
      automatic: true,
      error: '',
      cleanupError: '',
      createdAt: this.clock.now(),
    };
  }
  async submit(storeId: string, channel: string, text: string): Promise<void> {
    const team = this.ensure(storeId);
    if (!text.trim() || new TextEncoder().encode(text).length > CHAT_TEXT_BYTES)
      throw problem('The message is empty or exceeds the text limit.');
    if (!this.canSubmit(storeId, channel))
      throw problem(
        'The previous message is still being saved. Your draft is kept.',
      );
    const writable = this.inbox
      .getSnapshot()
      .get(storeId)
      ?.data?.channels.find((c) => c.id === channel)?.writable;
    if (!writable) throw problem('You cannot send messages to this channel.');
    team.messages = team.messages.filter(
      (m) => !m.observed || m.intentPending || m.cleanupError,
    );
    if (!this.canRetain(text))
      throw problem(
        'Resolve pending messages before sending more. Your draft is kept.',
      );
    const message = this.newMessage(channel, text);
    team.messages.push(message);
    if (team.drafts.get(channel) === text) team.drafts.delete(channel);
    this.publish();
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
        message.operation = reply.result.operation;
        const duplicate = team.messages.find(
          (m) => m !== message && m.operation?.id === message.operation?.id,
        );
        message.observed ||= duplicate?.observed ?? false;
        team.messages = team.messages.filter(
          (m) => m === message || m.operation?.id !== message.operation?.id,
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
        (m) => m.operation?.id === result.operation,
      );
      if (message && !message.observed) message.text = result.text;
      else if (operation && !message)
        team.messages.push({
          ...this.newMessage(result.channel, result.text, operation.id),
          submission: '',
          operation,
          phase: phaseOf(operation),
          automatic: false,
          intentPending: false,
        });
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
    for (const message of team.messages) {
      const op =
        reply.result.kind === 'operation' &&
        reply.result.operation.id === message.operation?.id
          ? reply.result.operation
          : team.operations.find((o) => o.id === message.operation?.id);
      if (!op) continue;
      message.operation = op;
      const observed = team.observations.get(op.id);
      if (observed) this.observeMessage(team, message, observed);
      message.observed ||= team.operations.some(
        (entry) => entry.id === op.id && entry.observed,
      );
      if (message.observed) {
        message.phase = 'sent';
        if (!message.intentPending) message.text = undefined;
      } else if (terminal(op) || op.state === 'uncertain') {
        message.phase = phaseOf(op);
        if (op.state === 'confirmed') message.error = '';
      }
    }
    for (const [id, observed] of team.observations) {
      if (
        !team.messages.some(
          (m) => m.channel === observed.channel && !m.operation,
        )
      )
        team.observations.delete(id);
    }
    this.publish();
  }
  request(storeId: string, action: ChatAction): Promise<ChatReply> {
    const team = this.ensure(storeId);
    const key = [
      'status',
      'reconcile',
      'finalize',
      'pending',
      'cleanup-pending',
    ].includes(action.action)
      ? JSON.stringify(action)
      : undefined;
    if (key && team.requests.has(key)) return team.requests.get(key)!;
    const work = this.perform(team, action);
    if (key) {
      team.requests.set(key, work);
      void work
        .finally(() => {
          if (team.requests.get(key) === work) team.requests.delete(key);
        })
        .catch(() => {});
    }
    return work;
  }
  private async perform(team: Team, action: ChatAction): Promise<ChatReply> {
    const epoch = team.epoch;
    const generation = this.generations.get(team.profile) ?? 0;
    const channel =
      'channel' in action
        ? action.channel
        : 'operation' in action
          ? team.operations.find((op) => op.id === action.operation)?.channel
          : undefined;
    const authorize = (phase: 'before' | 'after') => {
      if (!this.current(team, epoch)) throw cancelled();
      if (
        !localActions.has(action.action) &&
        (!this.available(team) ||
          generation !== (this.generations.get(team.profile) ?? 0) ||
          (phase === 'before' &&
            action.action === 'submit-message' &&
            (!this.readable(team, action.channel) ||
              !this.inbox
                .getSnapshot()
                .get(team.storeId)
                ?.data?.channels.some(
                  (c) => c.id === action.channel && c.writable,
                ))))
      )
        throw {
          code: 'access-changed',
          message:
            'Chat access changed. Saved work is kept until access returns.',
          fatal: false,
          ambiguous: phase === 'after',
          retryable: phase === 'before',
        };
      if (this.snapshot?.agent.state !== 'ready') throw cancelled();
      if (
        channel &&
        !localActions.has(action.action) &&
        this.inbox.isChannelBlocked(team.storeId, channel)
      )
        throw channelIntegrity();
    };
    try {
      const reply = await team.client.request(action, undefined, authorize);
      if (!this.current(team, epoch)) throw cancelled();
      const trusted = this.inbox.getSnapshot().get(team.storeId)?.scope;
      if (
        (team.scope && !sameScope(team.scope, reply.scope)) ||
        (trusted && !sameScope(trusted, reply.scope))
      )
        throw integrity();
      team.scope = reply.scope;
      this.apply(team, reply, action);
      return reply;
    } catch (error) {
      if (this.current(team, epoch)) {
        this.inbox.handleError(team.storeId, error, channel);
        // Immediately invalidate the team after an interactive send is denied,
        // matching conversation-history reads. Denials found by background
        // synchronization retain the inbox retry delay.
        if (normalizeCommandError(error).code === 'chat-access-denied')
          this.inbox.invalidate(team.storeId);
        if (action.action === 'attempt') {
          team.operations = team.operations.map((op) =>
            op.id === action.operation ? { ...op, statusUnknown: true } : op,
          );
        }
        this.publish();
      }
      throw error;
    }
  }
  private clearIntent(team: Team, message: OutgoingMessage): Promise<void> {
    const current = team.clears.get(message.id);
    if (current) return current;
    const work = this.removeIntent(team, message);
    team.clears.set(message.id, work);
    void work
      .finally(() => {
        if (team.clears.get(message.id) === work)
          team.clears.delete(message.id);
      })
      .catch(() => {});
    return work;
  }
  private async removeIntent(team: Team, message: OutgoingMessage) {
    if (!message.intentPending) return;
    const epoch = team.epoch;
    try {
      await this.persistence(team, message.channel).clear(message.submission);
      if (!this.current(team, epoch)) return;
      message.intentPending = false;
      message.cleanupError = '';
      if (message.observed) message.text = undefined;
    } catch (error) {
      if (this.current(team, epoch)) message.cleanupError = failure(error);
    }
    if (this.current(team, epoch)) this.publish();
  }
  private async drive(team: Team, message: OutgoingMessage) {
    if (message.running || !this.current(team)) return;
    if (
      !message.operation &&
      team.messages.some(
        (m) =>
          m !== message && m.channel === message.channel && m.intentPending,
      )
    ) {
      message.error = 'Another message is still being saved in this channel.';
      this.publish();
      return;
    }
    if (!this.available(team) || !this.readable(team, message.channel)) {
      message.phase = 'paused';
      this.publish();
      return;
    }
    const epoch = team.epoch;
    message.running = true;
    message.error = '';
    this.publish();
    try {
      if (!message.operation) {
        if (message.text === undefined) throw cancelled();
        message.phase = 'saving';
        message.intentPending = true;
        await this.persistence(team, message.channel).save({
          submission: message.submission,
          text: message.text,
        });
        if (!this.current(team, epoch)) return;
        message.phase = 'preparing';
        this.publish();
        const action = message.ambiguousPreparation
          ? 'prepare-message'
          : 'submit-message';
        if (action === 'submit-message') message.ambiguousPreparation = true;
        const reply = await this.request(team.storeId, {
          action,
          submission: message.submission,
          channel: message.channel,
          text: message.text,
        });
        if (!this.current(team, epoch)) return;
        if (reply.result.kind !== 'operation') throw integrity();
        message.operation = reply.result.operation;
      }
      const op = message.operation;
      const checking =
        message.observed ||
        message.phase === 'unconfirmed' ||
        op.state === 'uncertain' ||
        team.operations.some((o) => o.id === op.id && o.statusUnknown);
      message.phase = message.observed
        ? 'sent'
        : checking
          ? 'unconfirmed'
          : op.state === 'prepared'
            ? 'sending'
            : phaseOf(op);
      this.publish();
      void this.clearIntent(team, message);
      if (op.state === 'prepared' || op.state === 'uncertain') {
        const reply = await this.request(team.storeId, {
          action: checking ? 'reconcile' : 'attempt',
          operation: op.id,
        });
        if (this.current(team, epoch) && reply.result.kind === 'operation')
          message.phase = message.observed
            ? 'sent'
            : phaseOf(reply.result.operation);
      }
      if (this.current(team, epoch)) this.inbox.invalidate(team.storeId);
    } catch (error) {
      if (!this.current(team, epoch)) return;
      const typed = normalizeCommandError(error);
      message.error = failure(error);
      if (!message.operation) {
        message.ambiguousPreparation ||= typed.ambiguous;
        message.phase = message.ambiguousPreparation
          ? 'unconfirmed'
          : typed.code === 'access-changed'
            ? 'paused'
            : 'not-sent';
        if (!message.ambiguousPreparation && preparationCanChange(error))
          await this.clearIntent(team, message);
      } else {
        message.phase = message.observed ? 'sent' : 'unconfirmed';
        void this.request(team.storeId, {
          action: 'status',
          operation: message.operation.id,
        })
          .then((reply) => {
            if (
              this.current(team, epoch) &&
              reply.result.kind === 'operation'
            ) {
              message.operation = reply.result.operation;
              message.phase = message.observed
                ? 'sent'
                : typed.code === 'access-changed' &&
                    reply.result.operation.state === 'prepared'
                  ? 'paused'
                  : phaseOf(reply.result.operation);
              this.publish();
            }
          })
          .catch(() => {});
      }
    } finally {
      message.running = false;
      if (this.current(team, epoch)) this.publish();
    }
  }
  async retry(storeId: string, id: string): Promise<void> {
    const team = this.ensure(storeId);
    const message = team.messages.find((m) => m.id === id);
    if (!message || message.running) return;
    if (message.operation?.state === 'confirmed') {
      await this.clearIntent(team, message);
      return;
    }
    if (message.operation && terminal(message.operation)) return;
    await this.drive(team, message);
  }
  async restoreDraft(storeId: string, id: string): Promise<void> {
    const team = this.ensure(storeId);
    const message = team.messages.find((m) => m.id === id);
    if (
      !message ||
      message.running ||
      message.phase === 'unconfirmed' ||
      (message.ambiguousPreparation && !message.operation)
    )
      throw problem('Check the original message before editing it.');
    if (team.drafts.get(message.channel))
      throw problem(
        'Your current draft is kept. Clear it before restoring this message.',
      );
    if (message.operation?.state === 'prepared')
      await this.request(storeId, {
        action: 'cancel',
        operation: message.operation.id,
      });
    if (message.operation && !terminal(message.operation))
      throw problem('This message may already have been sent.');
    await this.clearIntent(team, message);
    if (message.intentPending) return;
    if (message.text) this.setDraft(storeId, message.channel, message.text);
    team.messages = team.messages.filter((m) => m !== message);
    this.publish();
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
    message.observed = true;
    message.phase = 'sent';
    if (!message.intentPending) message.text = undefined;
    team.operations = observeMessages(team.operations, new Set([observed.id]));
  }
  observeHistory(
    storeId: string,
    channel: string,
    messages: readonly ChatMessage[],
  ) {
    const team = this.teams.get(storeId);
    if (!team || !this.current(team)) return;
    const ids = new Set(messages.map((m) => m.id));
    if (team.messages.some((m) => m.channel === channel && !m.operation)) {
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
      if (
        message.channel !== channel ||
        !message.operation ||
        !ids.has(message.operation.id)
      )
        continue;
      this.observeMessage(
        team,
        message,
        messages.find((m) => m.id === message.operation!.id)!,
      );
    }
    team.operations = observeMessages(team.operations, ids);
    this.publish();
  }
  messageKey(storeId: string, operation: string): string {
    return (
      this.teams
        .get(storeId)
        ?.messages.find((m) => m.operation?.id === operation)?.id ?? operation
    );
  }
  refresh(storeId: string): Promise<void> {
    const team = this.ensure(storeId);
    if (team.refresh) return team.refresh;
    const work = this.recover(team);
    team.refresh = work;
    void work
      .finally(() => {
        if (team.refresh === work) team.refresh = undefined;
      })
      .catch(() => {});
    return work;
  }
  private async checkedWork(
    team: Team,
    key: string,
    work: () => Promise<unknown>,
  ) {
    const record = team.checks.get(key);
    if (record && record.due > this.clock.now()) return;
    let failed = false;
    try {
      await work();
    } catch {
      failed = true;
    }
    const failures = failed ? Math.min((record?.failures ?? 0) + 1, 6) : 0;
    team.checks.set(key, {
      failures,
      due:
        this.clock.now() +
        (failed ? Math.min(1000 * 2 ** failures, 30000) : 2000),
    });
  }
  private async recover(team: Team) {
    if (!this.current(team)) return;
    const epoch = team.epoch;
    await this.request(team.storeId, { action: 'pending' });
    if (!this.current(team, epoch)) return;
    const cleanup = await this.request(team.storeId, {
      action: 'cleanup-pending',
    });
    if (!this.current(team, epoch)) return;
    const obligations = new Map(
      cleanup.result.kind === 'cleanup-pending'
        ? cleanup.result.operations.map((op) => [op.id, op])
        : [],
    );
    const visited = new Set<string>();
    for (let count = 0; count < 16 && this.current(team, epoch); count++) {
      const jobs: { key: string; run: () => Promise<unknown> }[] = [];
      for (const op of team.operations) {
        if (team.messages.some((m) => m.operation?.id === op.id && m.running))
          continue;
        if (op.statusUnknown || op.state === 'uncertain')
          jobs.push({
            key: `status:${op.id}`,
            run: () =>
              this.request(team.storeId, {
                action:
                  this.available(team) && this.readable(team, op.channel)
                    ? 'reconcile'
                    : 'status',
                operation: op.id,
              }),
          });
        const message = team.messages.find((m) => m.operation?.id === op.id);
        if (
          op.kind === 'send-message' &&
          !op.observed &&
          !op.bodyUnavailable &&
          op.text === undefined &&
          message?.text === undefined &&
          this.available(team) &&
          this.readable(team, op.channel)
        ) {
          jobs.push({
            key: `body:${op.id}`,
            run: () =>
              this.request(team.storeId, {
                action: 'operation-body',
                operation: op.id,
                channel: op.channel,
              }),
          });
        }
      }
      for (const op of obligations.values()) {
        jobs.push({
          key: `cleanup:${op.id}`,
          run: async () => {
            try {
              if (
                op.state === 'rejected' &&
                !team.messages.some((m) => m.operation?.id === op.id) &&
                this.available(team) &&
                this.readable(team, op.channel)
              ) {
                await this.request(team.storeId, {
                  action: 'operation-body',
                  operation: op.id,
                  channel: op.channel,
                });
              }
              await this.request(team.storeId, {
                action: 'finalize',
                operation: op.id,
              });
              obligations.delete(op.id);
              if (!obligations.size) team.cleanupError = '';
            } catch (error) {
              team.cleanupError = failure(error);
              throw error;
            }
          },
        });
      }
      for (const message of team.messages) {
        if (message.running) continue;
        if (message.operation && message.intentPending)
          jobs.push({
            key: `intent:${message.id}`,
            run: async () => {
              await this.clearIntent(team, message);
              if (message.intentPending)
                throw new Error('Local intent cleanup is pending.');
            },
          });
        if (
          message.automatic &&
          ((!message.operation && message.intentPending) ||
            message.phase === 'paused') &&
          this.available(team) &&
          this.readable(team, message.channel)
        ) {
          jobs.push({
            key: `send:${message.id}`,
            run: async () => {
              await this.drive(team, message);
              if (!message.operation)
                throw new Error('Preparation is still pending.');
            },
          });
        }
      }
      const eligible = jobs
        .filter(
          (job) =>
            !visited.has(job.key) &&
            (team.checks.get(job.key)?.due ?? 0) <= this.clock.now(),
        )
        .sort(
          (a, b) =>
            (team.checks.get(a.key)?.due ?? 0) -
            (team.checks.get(b.key)?.due ?? 0),
        );
      const job = eligible[0];
      if (!job) break;
      visited.add(job.key);
      await this.checkedWork(team, job.key, job.run);
    }
    const live = new Set([
      ...team.operations.map((op) => op.id),
      ...team.messages.map((m) => m.id),
    ]);
    for (const key of team.checks.keys())
      if (!live.has(key.slice(key.indexOf(':') + 1))) team.checks.delete(key);
    if (this.current(team, epoch)) this.publish();
  }
  private arm() {
    this.clock.cancel(this.timer);
    if (this.active)
      this.timer = this.clock.later(() => {
        void this.tick();
      }, 1000);
  }
  private tick() {
    if (!this.active) return;
    this.synchronize();
    const teams = [...this.teams.values()];
    for (
      let count = 0;
      count < teams.length && this.recoveryProfiles.size < 4;
      count++
    ) {
      const team = teams[this.cursor++ % teams.length];
      if (
        !this.current(team) ||
        team.due > this.clock.now() ||
        this.recoveryProfiles.has(team.profile)
      )
        continue;
      const profiles = this.recoveryProfiles;
      profiles.add(team.profile);
      team.due = this.clock.now() + 2000;
      void (async () => {
        if (this.available(team)) {
          const channels =
            this.inbox.getSnapshot().get(team.storeId)?.data?.channels ?? [];
          for (const channel of channels
            .filter((c) => c.readable && !team.loaded.has(c.id))
            .slice(0, 4))
            await this.open(team.storeId, channel.id);
        }
        if (!this.current(team)) return;
        try {
          await this.refresh(team.storeId);
        } catch {
          team.due = this.clock.now() + 5000;
        }
      })()
        .finally(() => profiles.delete(team.profile))
        .catch(() => {});
    }
    this.arm();
  }
}
