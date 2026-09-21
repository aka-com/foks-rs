/**
 * A bounded in-memory timing log.
 *
 * Every layer records the same record: the renderer directly, the desktop
 * backend through a source the shell registers, the agent through the timing
 * each of its responses carries. Nothing here persists or leaves the process;
 * the Refresh status popover's Copy diagnostics is the only reader.
 *
 * What may be recorded is what the popover already prints: server ids, job
 * kinds, command and operation names, opaque store ids, and short hashes of
 * other identifiers. Never item paths, message text, aliases, error messages
 * or request values. Attribute maps are capped so one entry stays small.
 */

export type TimingLayer = 'renderer' | 'backend' | 'agent';
export type TimingOutcome = 'ok' | 'error' | 'cancelled' | 'busy' | 'retired';
export type TimingValue = number | boolean | string;

export interface TimingEvent {
  /** Milliseconds since the Unix epoch. */
  readonly at: number;
  readonly layer: TimingLayer;
  /** 'job.catalog', 'invoke', 'agent.op', 'chat.sync' … */
  readonly name: string;
  /** A server id, a store id, or a job key the popover already shows. */
  readonly scope?: string;
  /** Correlates the steps of one thing: a message id, a run sequence. */
  readonly id?: string;
  readonly phase?: string;
  /** Duration of this step, when it is a step. */
  readonly ms?: number;
  readonly outcome?: TimingOutcome;
  /** A command error code. Never the message. */
  readonly code?: string;
  readonly attrs?: Readonly<Record<string, TimingValue>>;
}
export type TimingInput = Omit<TimingEvent, 'at' | 'layer'> & {
  at?: number;
  layer?: TimingLayer;
};
export type TimingSource = () => Promise<readonly TimingEvent[]>;

export interface TimingClock {
  /** Wall clock, milliseconds since the Unix epoch. */
  now(): number;
  /** Monotonic milliseconds, for durations. */
  elapsed(): number;
}
export const systemTimingClock: TimingClock = {
  now: () => Date.now(),
  elapsed: () =>
    typeof performance === 'undefined' ? Date.now() : performance.now(),
};

export const TIMING_CAPACITY = 4096;
const MAX_ATTRS = 8;
const MAX_TEXT = 64;

function bounded(value: TimingValue): TimingValue {
  if (typeof value === 'string')
    return value.length > MAX_TEXT ? value.slice(0, MAX_TEXT) : value;
  if (typeof value === 'number')
    return Number.isFinite(value) ? Math.round(value * 100) / 100 : 0;
  return value;
}

function boundedAttrs(
  attrs: Readonly<Record<string, TimingValue>> | undefined,
): Readonly<Record<string, TimingValue>> | undefined {
  if (!attrs) return undefined;
  const entries = Object.entries(attrs)
    .filter(([, value]) => value !== undefined)
    .slice(0, MAX_ATTRS)
    .map(([key, value]) => [key.slice(0, MAX_TEXT), bounded(value)] as const);
  return entries.length
    ? Object.freeze(Object.fromEntries(entries))
    : undefined;
}

function text(value: string | undefined): string | undefined {
  return value === undefined ? undefined : value.slice(0, MAX_TEXT);
}

/** A short, stable hash for an identifier that must not be printed. */
export function hashId(value: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index++) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, '0').slice(0, 6);
}

/**
 * A request key with any store id it embeds replaced by that id's hash. Keys
 * such as `catalog:<profile>:roster:<store>` and the chat client's
 * `[<store>, <action>]` carry the store's JSON, which spells out account and
 * team aliases; the hash still correlates with the `store#` scopes elsewhere.
 */
export function redactKey(key: string): string {
  const start = key.search(/[[{]/);
  if (start < 0) return key;
  const prefix = key.slice(0, start);
  const tail = key.slice(start);
  const store = (value: string) => `store#${hashId(value)}`;
  try {
    const parsed: unknown = JSON.parse(tail);
    if (Array.isArray(parsed))
      return (
        prefix +
        parsed
          .map((item) =>
            typeof item === 'string'
              ? item.startsWith('{')
                ? store(item)
                : item
              : '?',
          )
          .join(':')
      );
    if (parsed && typeof parsed === 'object') return prefix + store(tail);
  } catch {
    /* not JSON: hashed whole below */
  }
  return `${prefix}#${hashId(tail)}`;
}

export class DiagnosticLog {
  private entries: (TimingEvent | undefined)[];
  private next = 0;
  private count = 0;
  private sources = new Set<TimingSource>();

  constructor(
    private capacity = TIMING_CAPACITY,
    private clock: TimingClock = systemTimingClock,
  ) {
    this.entries = new Array<TimingEvent | undefined>(capacity);
  }

  /** Records one event; the oldest is dropped once the log is full. */
  record(input: TimingInput): TimingEvent {
    const event: TimingEvent = Object.freeze({
      at: input.at ?? this.clock.now(),
      layer: input.layer ?? 'renderer',
      name: input.name.slice(0, MAX_TEXT),
      ...(input.scope !== undefined ? { scope: text(input.scope) } : {}),
      ...(input.id !== undefined ? { id: text(input.id) } : {}),
      ...(input.phase !== undefined ? { phase: text(input.phase) } : {}),
      ...(input.ms !== undefined
        ? { ms: Math.max(0, Math.round(input.ms * 10) / 10) }
        : {}),
      ...(input.outcome !== undefined ? { outcome: input.outcome } : {}),
      ...(input.code !== undefined ? { code: text(input.code) } : {}),
      ...(boundedAttrs(input.attrs)
        ? { attrs: boundedAttrs(input.attrs) }
        : {}),
    });
    this.entries[this.next] = event;
    this.next = (this.next + 1) % this.capacity;
    if (this.count < this.capacity) this.count++;
    return event;
  }

  /**
   * Starts a step and answers the function that ends it. The ending records
   * one event with the elapsed time. Ending twice records nothing the second
   * time.
   */
  span(
    name: string,
    fields: Omit<TimingInput, 'name' | 'ms' | 'outcome' | 'at'> = {},
  ): (outcome: TimingOutcome, extra?: Partial<TimingInput>) => void {
    const started = this.clock.elapsed();
    let ended = false;
    return (outcome, extra = {}) => {
      if (ended) return;
      ended = true;
      this.record({
        ...fields,
        ...extra,
        name,
        outcome,
        ms: this.clock.elapsed() - started,
        attrs: { ...fields.attrs, ...extra.attrs },
      });
    };
  }

  /** The recorded events, oldest first. */
  events(): readonly TimingEvent[] {
    if (this.count < this.capacity)
      return this.entries.slice(0, this.count) as TimingEvent[];
    return [
      ...this.entries.slice(this.next),
      ...this.entries.slice(0, this.next),
    ] as TimingEvent[];
  }

  get length(): number {
    return this.count;
  }

  clear(): void {
    this.entries = new Array<TimingEvent | undefined>(this.capacity);
    this.next = 0;
    this.count = 0;
  }

  /**
   * Registers another layer's events, read when diagnostics are collected. A
   * source that fails contributes nothing; it cannot fail the copy.
   */
  addSource(source: TimingSource): () => void {
    this.sources.add(source);
    return () => {
      this.sources.delete(source);
    };
  }

  /** This log's events merged with every source's, ordered by time. */
  async collect(): Promise<TimingEvent[]> {
    const external = await Promise.all(
      [...this.sources].map((source) =>
        Promise.resolve()
          .then(source)
          .catch((): readonly TimingEvent[] => []),
      ),
    );
    return [...this.events(), ...external.flat()].sort((a, b) => a.at - b.at);
  }
}

/** The renderer's log. One per window; the shell wires its sources. */
export const diagnosticLog = new DiagnosticLog();
