import type { ChatResult, ChatScope } from '../chat-contract';
import { CHAT_HISTORY_BYTES, CHAT_HISTORY_ROWS } from '../chat-limits';
import type { TeamInbox } from './inbox-service';
import {
  HISTORY_WINDOW,
  mergeHistoryPage,
  type HistoryPlacement,
  type HistoryWindow,
  type WindowLimits,
} from './conversation-model';
import { historyEvent } from './conversation-events';
import { cancelled } from './errors';
import { sameScope } from './scope';
import {
  freezeDto,
  isFrozenDto,
  isReadonlySet,
  readonlyMap,
} from './snapshots';

export interface HistoryBinding {
  readonly store: string;
  readonly channel: string;
  readonly scope: ChatScope;
  readonly generation: number;
  readonly authority: string;
}

/**
 * Where the reader left a channel's message list: the first message row
 * whose bottom is below the list's top edge, and the distance in pixels from
 * that edge to the row's top.
 */
export interface ListPosition {
  readonly message: string;
  readonly offset: number;
}

/**
 * The bounds of the retained history. `channels`, `rows` and `bytes` are one
 * budget across every retained window; `window` is what each channel's
 * window is trimmed to, so one channel cannot take the whole budget.
 */
export interface HistoryCacheLimits {
  channels: number;
  rows: number;
  bytes: number;
  window?: WindowLimits;
}

export class ChatHistoryCache {
  private inputs = new Map<
    string,
    {
      scope: ChatScope;
      generation: number;
      channels: NonNullable<TeamInbox['data']>['channels'];
      blocked: TeamInbox['blockedChannels'];
    }
  >();
  private bindings = new Map<string, Map<string, HistoryBinding>>();
  private entries = new Map<
    HistoryBinding,
    { history: HistoryWindow; bytes: number; position?: ListPosition }
  >();
  private listeners = new Set<() => void>();
  private revision = 0;

  constructor(
    private limits: HistoryCacheLimits = {
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
  /**
   * The channel's retained window. Reading it leaves the eviction order
   * unchanged, so a render can call this; {@link touch} records a use.
   */
  get(binding: HistoryBinding | null): HistoryWindow | null {
    if (!binding || !this.current(binding)) return null;
    return this.entries.get(binding)?.history ?? null;
  }
  /**
   * Moves the channel's retained window to the newest end of the eviction
   * order. Recency is not observable to subscribers, so this does not
   * publish.
   */
  touch(binding: HistoryBinding | null): void {
    if (!binding || !this.current(binding)) return;
    const entry = this.entries.get(binding);
    if (!entry) return;
    this.entries.delete(binding);
    this.entries.set(binding, entry);
  }
  /**
   * Where the reader left the channel's retained window, or `null` when it
   * was left at the newest end or no window is retained.
   */
  position(binding: HistoryBinding | null): ListPosition | null {
    if (!binding || !this.current(binding)) return null;
    return this.entries.get(binding)?.position ?? null;
  }
  /**
   * Records where the reader leaves the channel's retained window, `null`
   * for its newest end. The position goes with the window: it is dropped
   * when the window is evicted or its binding retired. A thread reads it
   * only when it opens, so this does not publish.
   */
  setPosition(
    binding: HistoryBinding | null,
    position: ListPosition | null,
  ): void {
    if (!binding || !this.current(binding)) return;
    const entry = this.entries.get(binding);
    if (!entry) return;
    if (position) entry.position = Object.freeze({ ...position });
    else delete entry.position;
  }
  update(store: string, inbox: TeamInbox, generation: number): void {
    if (inbox.state !== 'ready' || !inbox.scope || !inbox.data) {
      this.clear(store);
      return;
    }
    // Identity is evidence only for runtime-protected collections and DTOs.
    // Mutable callers still take the validation path on every update.
    const reusable =
      isFrozenDto(inbox.scope) &&
      isFrozenDto(inbox.data.channels) &&
      isReadonlySet(inbox.blockedChannels);
    const inputs = this.inputs.get(store);
    if (
      reusable &&
      inputs &&
      inputs.generation === generation &&
      inputs.channels === inbox.data.channels &&
      inputs.blocked === inbox.blockedChannels &&
      sameScope(inputs.scope, inbox.scope)
    )
      return;
    const previous = this.bindings.get(store);
    const next = new Map<string, HistoryBinding>();
    const scope = isFrozenDto(inbox.scope)
      ? inbox.scope
      : freezeDto(structuredClone(inbox.scope));
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
              scope,
              generation,
              authority,
            }),
      );
    }
    this.bindings.set(store, next);
    if (reusable)
      this.inputs.set(store, {
        scope: inbox.scope,
        generation,
        channels: inbox.data.channels,
        blocked: inbox.blockedChannels,
      });
    else this.inputs.delete(store);
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
    placement: HistoryPlacement = {},
  ): HistoryWindow {
    if (!this.current(binding) || page.channel !== binding.channel)
      throw cancelled();
    const event = historyEvent(page, before, placement);
    const held = this.entries.get(binding);
    const result = mergeHistoryPage(
      placement.replace ? null : (held?.history ?? null),
      event,
      this.limits.window ?? HISTORY_WINDOW,
    );
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
    this.entries.set(binding, {
      history,
      bytes,
      ...(held?.position ? { position: held.position } : {}),
    });
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
    return history;
  }
  clear(store?: string): void {
    if (store === undefined) {
      this.inputs.clear();
      if (!this.bindings.size && !this.entries.size) return;
      this.bindings.clear();
      this.entries.clear();
    } else {
      this.inputs.delete(store);
      const bindings = this.bindings.get(store);
      if (!bindings) return;
      for (const binding of bindings.values()) this.entries.delete(binding);
      this.bindings.delete(store);
    }
    this.publish();
  }
}
