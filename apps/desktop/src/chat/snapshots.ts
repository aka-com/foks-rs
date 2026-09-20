import type {
  ChatChannel,
  ChatConversation,
  ChatPreview,
  ChatResult,
} from '../chat-contract';
import type { TeamStore } from '../model';

type Inbox = Extract<ChatResult, { kind: 'inbox' }>;

/** Length-prefixed components cannot collide when aliases contain separators. */
function key(parts: string[]): string {
  return parts.map((part) => `${part.length}:${part}`).join('');
}
export const accountKey = (store: TeamStore): string =>
  key([store.server, store.account]);
export const teamIdentity = (store: TeamStore): string =>
  key([store.server, store.account, store.alias, store.team_id_hex]);

/**
 * The newest sequence the inbox states this conversation holds: the read
 * pointer plus what is unread, where a pending local read stands in for a
 * read pointer it has not overtaken yet. The agent derives the unread count
 * from the same stored row's last message, so this equals that row's head
 * and never falls below it.
 */
export function lastPosition(conversation: ChatConversation): bigint {
  const read = BigInt(conversation.read_through);
  const pending = BigInt(conversation.pending_read ?? '0');
  return (read > pending ? read : pending) + BigInt(conversation.unread);
}
function samePreview(a: ChatPreview, b: ChatPreview): boolean {
  return (
    a.sender === b.sender &&
    a.send_time === b.send_time &&
    a.insert_time === b.insert_time &&
    a.content.kind === b.content.kind &&
    (a.content.kind !== 'text' ||
      (b.content.kind === 'text' && a.content.text === b.content.text))
  );
}
function sameReadAuthority(a: ChatChannel, b: ChatChannel): boolean {
  return (
    a.id === b.id &&
    a.readable === b.readable &&
    a.read_role === b.read_role &&
    a.admin === b.admin
  );
}

/**
 * Basic hosts have no content revision. Read/prefs/status changes are not
 * content.
 *
 * `conservative` decides, per channel, whether a degraded projection alone
 * invalidates it. A degraded host cannot say which threads moved, so the only
 * correct answer from the projection is "possibly all of them"; asking that
 * question of every channel on every synchronization is what turns a degraded
 * host into a polling loop, so the caller rate-limits it per channel and
 * passes the result here. A channel the predicate refuses is still
 * invalidated by every change the comparison below can actually see.
 */
export function contentRevisions(
  previous: Inbox | undefined,
  incoming: Inbox,
  revisions: ReadonlyMap<string, number>,
  conservative: (channel: string) => boolean = () => true,
): ReadonlyMap<string, number> {
  const channels = new Map(previous?.channels.map((c) => [c.id, c]));
  const conversations = new Map(
    previous?.conversations.map((c) => [c.channel.id, c]),
  );
  const nextConversations = new Map(
    incoming.conversations.map((c) => [c.channel.id, c]),
  );
  return new Map(
    incoming.channels.map((channel) => {
      const old = channels.get(channel.id);
      const a = conversations.get(channel.id);
      const b = nextConversations.get(channel.id);
      const changed =
        !old ||
        !sameReadAuthority(old, channel) ||
        (incoming.degraded && conservative(channel.id)) ||
        !!a !== !!b ||
        (!!a &&
          !!b &&
          (lastPosition(a) !== lastPosition(b) ||
            (!!a.preview &&
              !!b.preview &&
              !samePreview(a.preview, b.preview))));
      return [
        channel.id,
        (revisions.get(channel.id) ?? 0) + Number(changed),
      ] as const;
    }),
  );
}

const protectedMaps = new WeakSet<object>();
const protectedSets = new WeakSet<object>();
const frozenDtos = new WeakSet<object>();
export const isFrozenDto = (value: object): boolean => frozenDtos.has(value);
export const isReadonlySet = (value: object): boolean =>
  protectedSets.has(value);

/** No writable collection escapes the service, even through a runtime cast. */
export function readonlyMap<K, V>(
  entries: Iterable<readonly [K, V]>,
): ReadonlyMap<K, V> {
  if (protectedMaps.has(entries)) return entries as ReadonlyMap<K, V>;
  return mapView(new Map(entries));
}

/** Copy once, update privately, and expose no mutable backing-map reference. */
export function updatedReadonlyMap<K, V>(
  entries: ReadonlyMap<K, V>,
  key: K,
  value: V,
): ReadonlyMap<K, V> {
  const map = new Map(entries);
  map.set(key, value);
  return mapView(map);
}
function mapView<K, V>(map: Map<K, V>): ReadonlyMap<K, V> {
  const view: ReadonlyMap<K, V> = Object.freeze({
    get size() {
      return map.size;
    },
    get: (key: K) => map.get(key),
    has: (key: K) => map.has(key),
    entries: () => map.entries(),
    keys: () => map.keys(),
    values: () => map.values(),
    [Symbol.iterator]: () => map[Symbol.iterator](),
    forEach: (
      callback: (value: V, key: K, map: ReadonlyMap<K, V>) => void,
      thisArg?: unknown,
    ) => {
      map.forEach((value, key) => callback.call(thisArg, value, key, view));
    },
  });
  protectedMaps.add(view);
  return view;
}
export function readonlySet<T>(entries: Iterable<T>): ReadonlySet<T> {
  if (protectedSets.has(entries)) return entries as ReadonlySet<T>;
  const set = new Set(entries);
  const view: ReadonlySet<T> = Object.freeze({
    get size() {
      return set.size;
    },
    has: (value: T) => set.has(value),
    entries: () => set.entries(),
    keys: () => set.keys(),
    values: () => set.values(),
    [Symbol.iterator]: () => set[Symbol.iterator](),
    forEach: (
      callback: (value: T, key: T, set: ReadonlySet<T>) => void,
      thisArg?: unknown,
    ) => {
      set.forEach((value) => callback.call(thisArg, value, value, view));
    },
  });
  protectedSets.add(view);
  return view;
}
/** DTOs contain only plain objects, arrays and primitives; never call on a model. */
export function freezeDto<T>(value: T): T {
  if (value && typeof value === 'object' && !frozenDtos.has(value)) {
    for (const child of Object.values(value)) freezeDto(child);
    Object.freeze(value);
    frozenDtos.add(value);
  }
  return value;
}
