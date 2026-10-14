interface Draft {
  text: string;
  /** The text's UTF-8 length, measured once when the text is stored. */
  bytes: number;
}

const encoder = new TextEncoder();

/**
 * The unsent text of every channel this session holds a draft for, keyed by
 * team and channel. All drafts share one byte quota. The store keeps their
 * total and each draft's length, so storing a text measures only that text.
 *
 * A change notifies only the listeners of the channel whose draft changed,
 * so typing in one composer re-renders nothing that reads another channel's
 * draft or the send service's revision.
 */
export class DraftStore {
  private drafts = new Map<string, Map<string, Draft>>();
  private listeners = new Map<string, Map<string, Set<() => void>>>();
  private total = 0;

  constructor(private readonly limit: number) {}

  get(store: string, channel: string): string {
    return this.drafts.get(store)?.get(channel)?.text ?? '';
  }
  /** The UTF-8 length of the channel's draft, measured when it was stored. */
  size(store: string, channel: string): number {
    return this.drafts.get(store)?.get(channel)?.bytes ?? 0;
  }
  /** The UTF-8 length of all drafts together. */
  bytes(): number {
    return this.total;
  }
  /**
   * Stores `text` as the channel's draft, or removes the draft when `text` is
   * empty. Returns false, and keeps the previous draft, when the new text
   * would take all drafts past the quota.
   */
  set(store: string, channel: string, text: string): boolean {
    if (!text) {
      this.delete(store, channel);
      return true;
    }
    const drafts = this.drafts.get(store);
    const previous = drafts?.get(channel);
    if (previous?.text === text) return true;
    const bytes = encoder.encode(text).length;
    if (this.total - (previous?.bytes ?? 0) + bytes > this.limit) return false;
    this.total += bytes - (previous?.bytes ?? 0);
    if (drafts) drafts.set(channel, { text, bytes });
    else this.drafts.set(store, new Map([[channel, { text, bytes }]]));
    this.notify(store, channel);
    return true;
  }
  /** Removes the channel's draft. Returns whether there was one. */
  delete(store: string, channel: string): boolean {
    const drafts = this.drafts.get(store);
    const previous = drafts?.get(channel);
    if (!drafts || !previous) return false;
    drafts.delete(channel);
    if (!drafts.size) this.drafts.delete(store);
    this.total -= previous.bytes;
    this.notify(store, channel);
    return true;
  }
  /** Removes every draft of the team. Returns whether there was one. */
  clear(store: string): boolean {
    let removed = false;
    for (const channel of [...(this.drafts.get(store)?.keys() ?? [])])
      removed = this.delete(store, channel) || removed;
    return removed;
  }
  /** Removes every draft of every team. */
  clearAll() {
    for (const store of [...this.drafts.keys()]) this.clear(store);
  }
  /** Removes the team's drafts for channels `live` does not hold. */
  retain(store: string, live: ReadonlySet<string>) {
    for (const channel of [...(this.drafts.get(store)?.keys() ?? [])])
      if (!live.has(channel)) this.delete(store, channel);
  }
  /** Calls `listener` whenever the channel's draft changes. */
  subscribe(store: string, channel: string, listener: () => void) {
    let channels = this.listeners.get(store);
    if (!channels) {
      channels = new Map<string, Set<() => void>>();
      this.listeners.set(store, channels);
    }
    let set = channels.get(channel);
    if (!set) {
      set = new Set();
      channels.set(channel, set);
    }
    set.add(listener);
    return () => {
      set.delete(listener);
      if (set.size || channels.get(channel) !== set) return;
      channels.delete(channel);
      if (!channels.size && this.listeners.get(store) === channels)
        this.listeners.delete(store);
    };
  }
  private notify(store: string, channel: string) {
    const set = this.listeners.get(store)?.get(channel);
    if (!set) return;
    for (const listener of [...set]) listener();
  }
}
