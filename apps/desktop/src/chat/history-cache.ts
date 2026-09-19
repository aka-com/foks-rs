import type { ChatResult, ChatScope } from '../chat-contract';
import { CHAT_HISTORY_BYTES, CHAT_HISTORY_ROWS } from '../chat-limits';
import type { TeamInbox } from './inbox-service';
import { conversationResult, type HistoryWindow } from './conversation-model';
import { eventFromReply } from './conversation-events';
import { cancelled } from './errors';
import { sameScope } from './scope';
import { freezeDto, readonlyMap } from './snapshots';

export interface HistoryBinding {
  readonly store: string;
  readonly channel: string;
  readonly scope: ChatScope;
  readonly generation: number;
  readonly authority: string;
}

export class ChatHistoryCache {
  private bindings = new Map<string, Map<string, HistoryBinding>>();
  private entries = new Map<
    HistoryBinding,
    { history: HistoryWindow; bytes: number }
  >();
  private listeners = new Set<() => void>();
  private revision = 0;

  constructor(
    private limits = {
      channels: 16,
      rows: CHAT_HISTORY_ROWS,
      bytes: CHAT_HISTORY_BYTES,
    },
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
  binding(
    store: string,
    channel: string,
    generation: number,
  ): HistoryBinding | null {
    const binding = this.bindings.get(store)?.get(channel);
    return binding?.generation === generation ? binding : null;
  }
  private current(binding: HistoryBinding): boolean {
    return this.bindings.get(binding.store)?.get(binding.channel) === binding;
  }
  get(binding: HistoryBinding | null): HistoryWindow | null {
    if (!binding || !this.current(binding)) return null;
    const entry = this.entries.get(binding);
    if (!entry) return null;
    this.entries.delete(binding);
    this.entries.set(binding, entry);
    return entry.history;
  }
  update(store: string, inbox: TeamInbox, generation: number): void {
    if (inbox.state !== 'ready' || !inbox.scope || !inbox.data) {
      this.clear(store);
      return;
    }
    const previous = this.bindings.get(store);
    const next = new Map<string, HistoryBinding>();
    for (const channel of inbox.data.channels) {
      if (!channel.readable || inbox.blockedChannels.has(channel.id)) continue;
      const authority = JSON.stringify([channel.read_role, channel.admin]);
      const old = previous?.get(channel.id);
      next.set(
        channel.id,
        old &&
          old.generation === generation &&
          old.authority === authority &&
          sameScope(old.scope, inbox.scope)
          ? old
          : Object.freeze({
              store,
              channel: channel.id,
              scope: freezeDto(structuredClone(inbox.scope)),
              generation,
              authority,
            }),
      );
    }
    this.bindings.set(store, next);
    let changed = false;
    for (const binding of previous?.values() ?? []) {
      if (next.get(binding.channel) !== binding) {
        this.entries.delete(binding);
        changed = true;
      }
    }
    if (changed) this.publish();
  }
  accept(
    binding: HistoryBinding,
    page: Extract<ChatResult, { kind: 'history' }>,
    before: string | null,
  ): void {
    if (!this.current(binding) || page.channel !== binding.channel)
      throw cancelled();
    const event = eventFromReply(
      { action: 'history', channel: page.channel, before },
      page,
    );
    let result: HistoryWindow;
    try {
      result = conversationResult(
        { operations: [], history: this.get(binding) },
        event,
      ).history!;
    } catch (error) {
      if (
        before !== null ||
        (error as { code?: string } | null)?.code !== 'chat-limit'
      )
        throw error;
      result = conversationResult(
        { operations: [], history: null },
        event,
      ).history!;
    }
    const history = Object.freeze({
      ...result,
      messages: freezeDto(structuredClone(result.messages)),
      verification: readonlyMap(result.verification),
    });
    const bytes = history.messages.reduce(
      (sum, message) =>
        sum +
        (message.content.kind === 'text'
          ? new TextEncoder().encode(message.content.text).length
          : 0),
      0,
    );
    this.entries.delete(binding);
    this.entries.set(binding, { history, bytes });
    let rows = 0;
    let totalBytes = 0;
    for (const entry of this.entries.values()) {
      rows += entry.history.messages.length;
      totalBytes += entry.bytes;
    }
    for (const [key, entry] of this.entries) {
      if (
        this.entries.size <= this.limits.channels &&
        rows <= this.limits.rows &&
        totalBytes <= this.limits.bytes
      )
        break;
      this.entries.delete(key);
      rows -= entry.history.messages.length;
      totalBytes -= entry.bytes;
    }
    this.publish();
  }
  clear(store?: string): void {
    if (store === undefined) {
      if (!this.bindings.size && !this.entries.size) return;
      this.bindings.clear();
      this.entries.clear();
    } else {
      const bindings = this.bindings.get(store);
      if (!bindings) return;
      for (const binding of bindings.values()) this.entries.delete(binding);
      this.bindings.delete(store);
    }
    this.publish();
  }
}
