import type { Bridge } from '../bridge';
import { commandRecovery, normalizeMutationError } from '../bridge';
import type {
  ChatAction,
  ChatOperation,
  ChatReply,
  ChatScope,
} from '../chat-contract';
import { chatAvailable, storeOf } from '../model';
import type { AgentSnapshot, TeamStore } from '../model';
import { failure, preparationCanChange, submissionId } from './actions';
import { chatClient, integrity, sameScope } from './client';
import type { ChatInboxService } from './inbox-service';
import { freezeDto } from './snapshots';
import {
  channelDescriptionProblem,
  channelNameProblem,
  normalizeChannelName,
} from './presentation';

export const CHANNEL_CREATION_COMPLETION_LIMIT = 16;
export const CHANNEL_CREATION_RECORD_LIMIT = 64;

export interface ChannelCreationInput {
  name: string;
  description: string;
  admin: boolean;
}

export interface ChannelCreation {
  id: string;
  store: TeamStore;
  scope: ChatScope;
  input?: Readonly<ChannelCreationInput>;
  operation?: ChatOperation;
  blocked?: boolean;
  state: 'working' | 'unresolved' | 'review' | 'confirmed' | 'cancelled';
  error: string;
}

interface OwnedCreation extends ChannelCreation {
  submission?: Extract<ChatAction, { action: 'prepare-channel' }>;
  held: boolean;
  busy: boolean;
  invalidIdentity?: boolean;
}

/**
 * Where a team's pending-ledger check stands while `readyFor` is false: the
 * `pending` request is in flight, or it ended without the ledger being
 * trusted. A team that is ready has no check record.
 */
export interface ChannelCreationCheck {
  state: 'checking' | 'failed';
  /** Empty while checking. */
  error: string;
}

export interface ChannelCreationAccess {
  snapshot: AgentSnapshot;
  accessNow?: () => number;
  accessGenerations?: ReadonlyMap<string, number>;
}

export class ChannelCreationController {
  private records = new Map<string, OwnedCreation>();
  private snapshot: readonly ChannelCreation[] = Object.freeze([]);
  private listeners = new Set<() => void>();
  private clients = new Set<ReturnType<typeof chatClient>>();
  private scans = new Map<string, { scope: ChatScope; generation: number }>();
  private discovering = new Set<string>();
  private checks = new Map<string, ChannelCreationCheck>();
  private checkSnapshot: ReadonlyMap<string, ChannelCreationCheck> = new Map();
  private epoch = 0;

  readyFor = (store: string) => {
    const scan = this.scans.get(store);
    const scope = this.inbox.getSnapshot().get(store)?.scope;
    const access = this.access();
    const current = storeOf(access.snapshot, store);
    return Boolean(
      scan &&
      scope &&
      current &&
      sameScope(scan.scope, scope) &&
      scan.generation === (access.accessGenerations?.get(current.server) ?? 0),
    );
  };

  constructor(
    private bridge: Bridge,
    private inbox: Pick<
      ChatInboxService,
      'getSnapshot' | 'invalidate' | 'block' | 'handleError'
    >,
    private access: () => ChannelCreationAccess,
  ) {}

  getSnapshot = () => this.snapshot;
  /** The check standing behind each team that `readyFor` refuses, by store. */
  getChecks = () => this.checkSnapshot;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  private prune() {
    const terminal = [...this.records.values()].filter(
      (record) =>
        !record.busy &&
        (record.state === 'confirmed' || record.state === 'cancelled'),
    );
    for (const record of terminal.slice(
      0,
      Math.max(0, terminal.length - CHANNEL_CREATION_COMPLETION_LIMIT),
    ))
      this.records.delete(record.id);
  }

  private publish() {
    this.prune();
    this.snapshot = freezeDto(
      [...this.records.values()].map((record) =>
        structuredClone({
          id: record.id,
          store: record.store,
          scope: record.scope,
          input: record.input,
          operation: record.operation,
          blocked: Boolean(record.invalidIdentity),
          state: record.state,
          error: record.error,
        }),
      ),
    );
    for (const listener of this.listeners) listener();
  }

  private setCheck(store: string, check?: ChannelCreationCheck) {
    if (check) this.checks.set(store, Object.freeze({ ...check }));
    else this.checks.delete(store);
    this.checkSnapshot = new Map(this.checks);
    for (const listener of this.listeners) listener();
  }

  acknowledge(id: string) {
    const record = this.records.get(id);
    if (
      !record ||
      record.busy ||
      (record.state !== 'confirmed' && record.state !== 'cancelled')
    )
      return;
    this.records.delete(id);
    this.publish();
  }

  completedDestination(id: string) {
    const record = this.records.get(id);
    if (!record || record.state !== 'confirmed' || !record.operation)
      return null;
    const generation =
      this.access().accessGenerations?.get(record.store.server) ?? 0;
    this.authorize(record.store, record.scope, generation, 'before');
    return { ref: record.store.id, channel: record.operation.channel };
  }

  // The conversation guards every request on the shell's availability and on
  // the access generation the server is on; a channel created from this sheet
  // is the same kind of write and is guarded the same way.
  private authorize(
    store: TeamStore,
    scope: ChatScope,
    generation: number,
    phase: 'before' | 'after',
  ) {
    const access = this.access();
    const current = storeOf(access.snapshot, store.id);
    const entry = this.inbox.getSnapshot().get(store.id);
    if (entry?.scope && !sameScope(scope, entry.scope)) {
      this.inbox.block(store.id, 'The chat identity changed.');
      throw integrity();
    }
    if (
      current?.kind === 'team' &&
      current.server === store.server &&
      chatAvailable(access.snapshot, current, {
        nowSeconds: access.accessNow?.() ?? Date.now() / 1000,
      }) &&
      entry?.state !== 'blocked' &&
      entry?.scope &&
      generation === (access.accessGenerations?.get(store.server) ?? 0)
    )
      return;
    throw {
      code: 'chat-access-denied',
      message:
        'Chat access changed. The saved channel creation can be checked when access is available.',
      fatal: false,
      // A request that may already have been taken is not retryable on its
      // own: the saved preparation is what settles it.
      ambiguous: phase === 'after',
      retryable: phase === 'before',
    };
  }

  // The service holds the scope this team's chat has been established under.
  // A reply under a different identity is not this team's, so the account is
  // quarantined rather than trusted for one more request.
  private trust(store: TeamStore, scope: ChatScope, reply: ChatReply) {
    if (!sameScope(scope, reply.scope)) {
      this.inbox.block(store.id, 'The chat identity changed.');
      throw integrity();
    }
  }

  submit(store: TeamStore, input: ChannelCreationInput): string {
    const existing = [...this.records.values()].find(
      (record) =>
        record.store.id === store.id &&
        record.state !== 'confirmed' &&
        record.state !== 'cancelled',
    );
    if (existing) return existing.id;
    if (!this.readyFor(store.id))
      throw new Error(
        'Check saved channel creations before creating a new channel.',
      );
    const entry = this.inbox.getSnapshot().get(store.id);
    if (!entry?.scope || !entry.data || entry.state !== 'ready')
      throw new Error(
        'Wait for this team’s channels to load before creating a channel.',
      );
    const problem =
      channelNameProblem(
        input.name,
        entry.data.channels.map((channel) => channel.name),
      ) ?? channelDescriptionProblem(input.description);
    if (problem) throw new Error(problem);
    const generation = this.access().accessGenerations?.get(store.server) ?? 0;
    this.authorize(store, entry.scope, generation, 'before');
    this.prune();
    if (this.records.size >= CHANNEL_CREATION_RECORD_LIMIT)
      throw new Error(
        'Review or dismiss saved channel creations before submitting another.',
      );
    const id = submissionId();
    const frozen = Object.freeze({
      ...input,
      name: normalizeChannelName(input.name),
    });
    const record: OwnedCreation = {
      id,
      store: structuredClone(store),
      scope: structuredClone(entry.scope),
      input: frozen,
      submission: { action: 'prepare-channel', submission: id, ...frozen },
      state: 'working',
      error: '',
      held: false,
      busy: false,
    };
    this.records.set(id, record);
    void this.run(record, 'attempt');
    return id;
  }

  retry(id: string) {
    const record = this.records.get(id);
    if (
      !record ||
      record.busy ||
      record.invalidIdentity ||
      record.state === 'confirmed' ||
      record.state === 'cancelled'
    )
      return;
    if (record.state === 'review') return;
    void this.run(record, record.input ? 'retry' : 'reconcile');
  }

  check(id: string) {
    const record = this.records.get(id);
    if (
      !record ||
      record.busy ||
      record.invalidIdentity ||
      !record.operation ||
      record.state !== 'unresolved'
    )
      return;
    void this.run(record, 'reconcile');
  }

  cancel(id: string) {
    const record = this.records.get(id);
    if (
      !record ||
      record.busy ||
      record.invalidIdentity ||
      record.operation?.state !== 'prepared'
    )
      return;
    void this.run(record, 'cancel');
  }

  review(id: string) {
    const record = this.records.get(id);
    if (
      !record ||
      record.busy ||
      record.invalidIdentity ||
      record.operation?.state !== 'rejected'
    )
      return;
    this.records.delete(id);
    this.publish();
  }

  private accept(record: OwnedCreation, reply: ChatReply) {
    this.trust(record.store, record.scope, reply);
    if (
      reply.result.kind !== 'operation' ||
      reply.result.operation.kind !== 'create-channel'
    )
      throw integrity();
    const operation = reply.result.operation;
    if (
      record.operation &&
      (operation.id !== record.operation.id ||
        operation.channel !== record.operation.channel)
    )
      throw integrity();
    for (const [id, other] of this.records) {
      if (
        id !== record.id &&
        !other.submission &&
        other.store.id === record.store.id &&
        other.operation?.id === operation.id &&
        sameScope(other.scope, record.scope)
      )
        this.records.delete(id);
    }
    record.operation = structuredClone(operation);
    record.held = true;
    // Only a settled attempt retires the submission: until then it is what a
    // recovery re-issues.
    if (operation.state === 'confirmed') {
      if (operation.confirmation?.kind !== 'channel-created') throw integrity();
      record.state = 'confirmed';
      record.error = '';
      record.submission = undefined;
      record.input = undefined;
    } else if (operation.state === 'cancelled') {
      record.state = 'cancelled';
      record.submission = undefined;
      record.input = undefined;
    } else if (operation.state === 'rejected') {
      record.state = 'review';
      record.error =
        'Channel creation was rejected. Review the original channel name and audience before submitting a new preparation.';
    } else {
      record.state = 'unresolved';
      record.error =
        operation.state === 'uncertain'
          ? 'The result is not confirmed. Check again without creating a duplicate.'
          : 'Channel creation is saved but has not been confirmed.';
    }
  }

  private async run(
    record: OwnedCreation,
    mode: 'attempt' | 'reconcile' | 'retry' | 'cancel',
  ) {
    if (record.busy) return;
    record.busy = true;
    record.state = 'working';
    record.error = '';
    this.publish();
    const epoch = this.epoch;
    const generation =
      this.access().accessGenerations?.get(record.store.server) ?? 0;
    const client = chatClient(
      this.bridge,
      record.store.server,
      record.store.id,
    );
    this.clients.add(client);
    const authorize = (phase: 'before' | 'after') =>
      this.authorize(record.store, record.scope, generation, phase);
    try {
      if (!record.operation) {
        if (!record.submission) throw integrity();
        const reply = await client.request(
          record.submission,
          undefined,
          authorize,
        );
        if (epoch !== this.epoch) return;
        this.accept(record, reply);
      }
      const operation = record.operation;
      if (!operation) throw integrity();
      if (
        operation.state === 'prepared' ||
        operation.state === 'uncertain' ||
        mode === 'cancel'
      ) {
        const reply = await client.request(
          {
            action:
              mode === 'retry' ||
              (mode === 'attempt' && operation.state === 'uncertain')
                ? 'reconcile'
                : mode,
            operation: operation.id,
          },
          undefined,
          authorize,
        );
        if (epoch !== this.epoch) return;
        this.accept(record, reply);
        if (
          mode === 'retry' &&
          record.operation?.state === 'prepared' &&
          record.input
        ) {
          const entry = this.inbox.getSnapshot().get(record.store.id);
          const conflict = entry?.data?.channels.some(
            (channel) =>
              normalizeChannelName(channel.name) ===
                normalizeChannelName(record.input!.name) &&
              channel.id !== record.operation?.channel,
          );
          if (conflict) {
            record.state = 'review';
            record.error =
              'This channel name is now in use. Cancel this preparation and review the name.';
          } else {
            const retried = await client.request(
              { action: 'attempt', operation: operation.id },
              undefined,
              authorize,
            );
            if (epoch !== this.epoch) return;
            this.accept(record, retried);
          }
        }
      }
    } catch (cause) {
      if (epoch !== this.epoch) return;
      const failed = normalizeMutationError(cause);
      const quarantined = commandRecovery(failed).kind === 'quarantine';
      // An ambiguous failure — the request may have been taken, the reply was
      // lost — leaves the submission where it is, and marks it as one the
      // agent may hold from here on.
      record.held ||= failed.ambiguous;
      // A fatal failure ends the session this submission was made in: there
      // is nothing left to recover it with, so the sheet states the reason
      // and can be closed without preventing the user from navigating away.
      record.invalidIdentity ||= quarantined;
      this.inbox.handleError(record.store.id, failed);
      record.state =
        quarantined ||
        (!record.held && preparationCanChange(cause)) ||
        failed.code === 'chat-reprepare-required' ||
        /(?:stale|key|name-conflict)/.test(failed.code)
          ? 'review'
          : 'unresolved';
      record.error = failed.message;
      // A preparation the agent refused on its content can be changed and
      // sent again; one it may have taken cannot, and is recovered as it
      // stands. That includes a later refusal of a request that never
      // reached the agent: it says nothing about the one that did.
      if (
        record.state === 'review' &&
        !record.operation &&
        !record.held &&
        !quarantined
      ) {
        record.state = 'cancelled';
        record.input = undefined;
        record.submission = undefined;
      }
    } finally {
      client.dispose();
      this.clients.delete(client);
      if (epoch === this.epoch) {
        record.busy = false;
        // The column reads the service, so the new channel is listed only once
        // the team has been synchronized again.
        // Whatever the agent is still holding belongs in the conversation's
        // "Needs attention" as well, in case the sheet is closed on it.
        this.inbox.invalidate(record.store.id);
        this.publish();
      }
    }
  }

  async discover(store: TeamStore) {
    const entry = this.inbox.getSnapshot().get(store.id);
    if (
      !entry?.scope ||
      entry.state !== 'ready' ||
      this.readyFor(store.id) ||
      this.discovering.has(store.id)
    )
      return;
    this.discovering.add(store.id);
    this.setCheck(store.id, { state: 'checking', error: '' });
    // What the check leaves behind: nothing once the ledger is trusted, the
    // reason otherwise, so the sheet can say why rather than offer a retry
    // that reads as a prerequisite the reader has to satisfy.
    let check: ChannelCreationCheck | undefined;
    const epoch = this.epoch;
    const scope = entry.scope;
    const generation = this.access().accessGenerations?.get(store.server) ?? 0;
    const client = chatClient(this.bridge, store.server, store.id);
    this.clients.add(client);
    try {
      const reply = await client.request(
        { action: 'pending' },
        undefined,
        (phase) => this.authorize(store, scope, generation, phase),
      );
      if (epoch !== this.epoch) return;
      this.trust(store, scope, reply);
      let complete = true;
      this.prune();
      for (const operation of reply.result.operations) {
        if (
          operation.kind !== 'create-channel' ||
          [...this.records.values()].some(
            (record) =>
              record.store.id === store.id &&
              record.operation?.id === operation.id,
          )
        )
          continue;
        if (this.records.size >= CHANNEL_CREATION_RECORD_LIMIT) {
          complete = false;
          continue;
        }
        const id = `${store.id}:${operation.id}`;
        this.records.set(id, {
          id,
          store: structuredClone(store),
          scope: structuredClone(scope),
          operation: structuredClone(operation),
          state: 'unresolved',
          held: true,
          busy: false,
          error:
            'A saved channel creation needs attention. Check its status or cancel it; its original form is not stored on this device.',
        });
      }
      if (complete)
        this.scans.set(store.id, { scope: structuredClone(scope), generation });
      else {
        this.scans.delete(store.id);
        check = {
          state: 'failed',
          error:
            'Review or dismiss saved channel creations before creating another.',
        };
      }
      this.setCheck(store.id, check);
      this.publish();
    } catch (cause) {
      // A disposed controller has already cleared its checks.
      if (epoch !== this.epoch) return;
      this.scans.delete(store.id);
      this.setCheck(store.id, { state: 'failed', error: failure(cause) });
    } finally {
      client.dispose();
      this.clients.delete(client);
      this.discovering.delete(store.id);
    }
  }

  dispose() {
    this.epoch++;
    for (const client of this.clients) client.dispose();
    this.clients.clear();
    this.scans.clear();
    this.discovering.clear();
    this.checks.clear();
    this.checkSnapshot = new Map();
    this.records.clear();
    this.publish();
  }
}
