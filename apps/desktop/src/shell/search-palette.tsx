import { CollectionStatus } from '../components/collection-status';
import {
  inventoryReadiness,
  itemsReadiness,
  groupDetailReadiness,
  combineReadiness,
} from '../model';
import type { CollectionReadiness } from '../model';
/**
 * The global search palette (⌘K).
 *
 * One field over whatever page is open, reaching every kind the shell knows:
 * item and folder names and paths, vaults, teams and shares, the people on a
 * roster, the chat channels the caller can name, and the shell's commands. It
 * replaces the "All items" page and the per-page search field, which only
 * filtered paths inside one store. Item values are not searched.
 *
 * Enter opens the highlighted result. On an item, ⌘C copies its password or
 * value and ⇧⌘C a login's user name through the host's clipboard commands,
 * without opening it, ⌘O opens a login's website in the default browser, and
 * ⌘D adds it to Favorites or removes it. An empty
 * query lists Recent, Favorites and the commands (`recent-favorites.ts`).
 *
 * The palette is a sheet, not a location: it carries no `?state=` and pushes no
 * history entry. Opening a result navigates, which is what the history records.
 *
 * `searchIndex`, `parseSearchQuery` and `searchResults` are pure so the index,
 * the filter tokens and the ranking can be tested without a DOM.
 */

import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from 'react';

import {
  DismissibleDialog,
  anyDialogOpen,
  useDismissOnOverlayBlock,
} from '../../kit/overlay-primitives';
import { Icon } from '../components/icon';
import type { FoksIconName } from '../icons';
import {
  chordLabel,
  listedChords,
  matchesShortcut,
  shortcutLabel,
} from '../shortcuts';
import { GroupMark } from '../screens/group-mark';
import { copyActions, opensWebsite } from '../screens/item-copy';
import type { CopyAction } from '../screens/item-copy';
import type { ItemField } from '../bridge';
import {
  isFavorite,
  itemRefOf,
  rememberRecent,
  toggleFavorite,
  useRecentFavorites,
} from '../recent-favorites';
import type { ItemRef, RecentFavorites } from '../recent-favorites';
import {
  KINDS,
  hue,
  initials,
  isMachine,
  isTrashPath,
  kindOf,
  nameOf,
  parseRole,
  prefixOf,
  roleName,
  serverDisplayLabelForStore,
  storeDescription,
  storeNavigationOrder,
  storeOf,
} from '../model';
import type { AgentSnapshot, Store, StoreRef } from '../model';
import type { Location } from '../location';

/* ---------------------------------------------------------------- scopes -- */

/**
 * The chip a result is filtered by. `all` filters nothing. `actions` has no
 * chip: commands are listed under All only.
 */
export type SearchScope =
  'all' | 'items' | 'stores' | 'people' | 'channels' | 'actions';

/** The chips in the order the strip draws them, which is the order Tab walks. */
export const SEARCH_SCOPES: readonly {
  id: SearchScope;
  label: string;
}[] = [
  { id: 'all', label: 'All' },
  { id: 'items', label: 'Items' },
  { id: 'stores', label: 'Vaults & teams' },
  { id: 'people', label: 'People' },
  { id: 'channels', label: 'Channels' },
];

/** The heading each scope's results sit under, in group order. */
const GROUP_LABELS: Readonly<Record<Exclude<SearchScope, 'all'>, string>> = {
  items: 'Items',
  stores: 'Vaults, teams and shares',
  people: 'People',
  channels: 'Channels',
  actions: 'Actions',
};

/** The headings of the two lists an empty query shows above Actions. */
export const RECENT_GROUP = 'Recent';
export const FAVORITES_GROUP = 'Favorites';

const GROUP_ORDER = Object.keys(GROUP_LABELS) as Exclude<SearchScope, 'all'>[];

/**
 * How many results one group contributes. A palette is a shortcut, not a
 * listing: past a handful of rows in a kind, the page for that kind is the
 * better answer.
 */
export const SEARCH_GROUP_LIMIT = 6;

/* ----------------------------------------------------------------- index -- */

/** What opening a result does. */
export type SearchTarget =
  | { kind: 'item'; store: StoreRef; path: string }
  | { kind: 'folder'; store: StoreRef; path: string }
  | { kind: 'store'; ref: StoreRef }
  | { kind: 'person'; ref: StoreRef; username: string }
  | { kind: 'channel'; ref: StoreRef; channel?: string }
  | { kind: 'action'; id: string };

/** One searchable thing, already rendered down to the three lines a row draws. */
export interface SearchEntry {
  /** Stable within one index; used as the React key and the option id. */
  id: string;
  /** The scope chip that admits this entry, and its group heading. */
  scope: Exclude<SearchScope, 'all'>;
  group: string;
  /** The bold first line. */
  name: string;
  /** The quiet second line. */
  detail: string;
  /** The right-hand column: the server the thing lives on. */
  where: string;
  /**
   * Everything the query is matched against, most significant first. The name
   * is always first, so a name match outranks a match on a path or an alias.
   */
  terms: readonly string[];
  /**
   * The names and aliases of the stores the entry is in, which an `in:` token
   * matches. Empty for a store and for an action.
   */
  within: readonly string[];
  /** The tile drawn at the head of the row. */
  glyph:
    | { kind: 'item'; itemKind: keyof typeof KINDS }
    | { kind: 'folder' }
    | { kind: 'store'; store: Store }
    | { kind: 'person'; username: string }
    | { kind: 'channel' }
    | { kind: 'action'; icon: FoksIconName };
  target: SearchTarget;
  /** An item's or a folder's identifiers, for Recent and Favorites. */
  ref?: ItemRef;
  /**
   * What ⌘C and ⇧⌘C copy: a Password's password and user name, or a text
   * Document's value. Absent for a file and a folder.
   */
  copies?: readonly CopyAction[];
  /** Whether ⌘O opens the item's website: set on a Password. */
  website?: true;
  /** An action's own shortcut, drawn where other rows name their server. */
  keys?: string;
}

/**
 * A command the palette runs, such as New password or Lock. The shell passes
 * the ones that apply where the reader is; the palette closes before running
 * one.
 */
export interface SearchAction {
  id: string;
  label: string;
  detail: string;
  icon: FoksIconName;
  /** The shortcut that runs it outside the palette, as the platform writes it. */
  keys?: string;
  run: () => void;
}

/**
 * A channel the caller can name. The shell learns channel names from the chat
 * inbox, which the palette does not load itself; `id` is the channel's chat id
 * where the caller has it, and without one the result opens the team's inbox.
 */
export interface SearchChannel {
  store: StoreRef;
  name: string;
  id?: string;
}

/** What a store is called in a result's second line. */
function storeWord(store: Store): string {
  if (store.kind === 'account') return 'Vault';
  return store.team_kind === 'adhoc' ? 'Share' : 'Team';
}

/** A store's own alias, which is a second name people search by. */
function storeAlias(store: Store): string {
  return store.kind === 'account' ? store.account : store.alias;
}

/** What an `in:` token matches a store by. */
function storeNames(store: Store): string[] {
  return [store.name, storeAlias(store)];
}

/**
 * Items and folders, in snapshot order. A folder result opens the folder in
 * its store's tree; an item result selects the item.
 */
function itemEntries(snapshot: AgentSnapshot): SearchEntry[] {
  const entries: SearchEntry[] = [];
  for (const item of snapshot.items) {
    const kind = kindOf(item);
    // Trash has its own view; its entries are not search results.
    if (isTrashPath(item.path)) continue;
    const store = storeOf(snapshot, item.store);
    if (!store) continue;
    const name = nameOf(item.path);
    const ref = itemRefOf(item);
    const common = {
      scope: 'items' as const,
      group: GROUP_LABELS.items,
      name,
      where: serverDisplayLabelForStore(snapshot, store),
      terms: [name, item.path],
      within: storeNames(store),
      ...(ref ? { ref } : {}),
    };
    const parent = prefixOf(item.path);
    if (kind === 'Folder') {
      entries.push({
        ...common,
        id: `folder:${item.store}|${item.path}`,
        detail: ['Folder', parent, store.name].filter(Boolean).join(' · '),
        glyph: { kind: 'folder' },
        target: { kind: 'folder', store: item.store, path: item.path },
      });
      continue;
    }
    const copies = copyActions(item);
    entries.push({
      ...common,
      id: `item:${item.store}|${item.path}`,
      detail: [KINDS[kind].label, parent, store.name]
        .filter(Boolean)
        .join(' · '),
      glyph: { kind: 'item', itemKind: kind },
      target: { kind: 'item', store: item.store, path: item.path },
      ...(copies.length ? { copies } : {}),
      ...(opensWebsite(item) ? { website: true as const } : {}),
    });
  }
  return entries;
}

function storeEntries(snapshot: AgentSnapshot): SearchEntry[] {
  return storeNavigationOrder(snapshot).map((store) => ({
    id: `store:${store.id}`,
    scope: 'stores' as const,
    group: GROUP_LABELS.stores,
    name: store.name,
    detail: `${storeWord(store)} · ${storeDescription(snapshot, store)}`,
    where: serverDisplayLabelForStore(snapshot, store),
    terms: [store.name, storeAlias(store)],
    within: [],
    glyph: { kind: 'store' as const, store },
    target: { kind: 'store' as const, ref: store.id },
  }));
}

/**
 * One row per person, not per membership: a person on two rosters is one
 * result naming both groups. Identity is server-scoped, so the same username
 * on two servers stays two people.
 */
function peopleEntries(snapshot: AgentSnapshot): SearchEntry[] {
  const order: string[] = [];
  const seen = new Map<
    string,
    {
      store: Store;
      username: string;
      groups: string[];
      within: string[];
      role: string;
    }
  >();
  for (const party of snapshot.parties) {
    if (party.party_kind !== 'user' || !party.username) continue;
    const store = storeOf(snapshot, party.store);
    if (!store) continue;
    const key = `${store.server}|${party.username}`;
    const role = parseRole(party.destination_role);
    const found = seen.get(key);
    if (found) {
      if (!found.groups.includes(store.name)) {
        found.groups.push(store.name);
        found.within.push(...storeNames(store));
      }
      continue;
    }
    order.push(key);
    seen.set(key, {
      store,
      username: party.username,
      groups: [store.name],
      within: storeNames(store),
      // A machine is an ordinary party; its note is the only thing that says
      // so, and that is more use in a result line than its role band.
      role: isMachine(party) ? 'Machine' : role ? roleName(role) : 'Member',
    });
  }
  return order.map((key) => {
    const person = seen.get(key);
    // `order` is written only alongside the map entry it names.
    if (!person) throw new Error(`unindexed person: ${key}`);
    return {
      id: `person:${key}`,
      scope: 'people' as const,
      group: GROUP_LABELS.people,
      name: person.username,
      detail: `${person.groups.join(', ')} · ${person.role}`,
      where: serverDisplayLabelForStore(snapshot, person.store),
      terms: [person.username],
      within: person.within,
      glyph: { kind: 'person' as const, username: person.username },
      target: {
        kind: 'person' as const,
        ref: person.store.id,
        username: person.username,
      },
    };
  });
}

function channelEntries(
  snapshot: AgentSnapshot,
  channels: readonly SearchChannel[],
): SearchEntry[] {
  const entries: SearchEntry[] = [];
  for (const channel of channels) {
    const store = storeOf(snapshot, channel.store);
    if (!store) continue;
    const alias = storeAlias(store);
    entries.push({
      id: `channel:${channel.store}|${channel.id ?? channel.name}`,
      scope: 'channels',
      group: GROUP_LABELS.channels,
      name: `${alias}#${channel.name}`,
      detail: `Channel · ${store.name}`,
      where: serverDisplayLabelForStore(snapshot, store),
      // `#deploys` first, so typing the hash reaches the channel and typing
      // the group name reaches every channel in it.
      terms: [`#${channel.name}`, channel.name, store.name],
      within: storeNames(store),
      glyph: { kind: 'channel' },
      target: {
        kind: 'channel',
        ref: channel.store,
        ...(channel.id ? { channel: channel.id } : {}),
      },
    });
  }
  return entries;
}

function actionEntries(actions: readonly SearchAction[]): SearchEntry[] {
  return actions.map((action) => ({
    id: `action:${action.id}`,
    scope: 'actions' as const,
    group: GROUP_LABELS.actions,
    name: action.label,
    detail: action.detail,
    where: '',
    terms: [action.label],
    within: [],
    glyph: { kind: 'action' as const, icon: action.icon },
    target: { kind: 'action' as const, id: action.id },
    ...(action.keys ? { keys: action.keys } : {}),
  }));
}

/**
 * Everything the palette can find, in group order. Channels and actions come
 * from the caller because the palette does not mount the chat inbox and does
 * not own the shell's commands; with none, the other kinds still search.
 */
export function searchIndex(
  snapshot: AgentSnapshot,
  channels: readonly SearchChannel[] = [],
  actions: readonly SearchAction[] = [],
): readonly SearchEntry[] {
  return [
    ...itemEntries(snapshot),
    ...storeEntries(snapshot),
    ...peopleEntries(snapshot),
    ...channelEntries(snapshot, channels),
    ...actionEntries(actions),
  ];
}

/* --------------------------------------------------------------- tokens -- */

/**
 * A query split into its text and its filter tokens. `in:<store>` keeps the
 * results that are in a vault, team or share whose name or alias starts with
 * the value. `kind:<kind>` keeps the items whose kind (`password`, `document`
 * or `folder`) starts with the value. A value with a space is quoted, as in
 * `in:"work (acme)"`. Tokens of one key widen each other and the two keys
 * narrow each other; a token with no value yet filters nothing.
 */
export interface SearchQuery {
  /** The rest of the query, trimmed and lowercased. */
  needle: string;
  within: readonly string[];
  kinds: readonly string[];
}

const TOKEN = /(^|\s)(in|kind):("[^"]*"?|\S*)/gi;

export function parseSearchQuery(query: string): SearchQuery {
  const within: string[] = [];
  const kinds: string[] = [];
  const rest = query.replace(
    TOKEN,
    (_token, lead: string, key: string, raw: string) => {
      const value = raw.replace(/^"|"$/g, '').trim().toLowerCase();
      if (value) (key.toLowerCase() === 'in' ? within : kinds).push(value);
      return lead;
    },
  );
  return { needle: rest.trim().toLowerCase(), within, kinds };
}

function entryKind(entry: SearchEntry): string | null {
  if (entry.glyph.kind === 'item') return entry.glyph.itemKind.toLowerCase();
  if (entry.glyph.kind === 'folder') return 'folder';
  return null;
}

/** Whether the entry passes the query's `in:` and `kind:` tokens. */
function admitted(entry: SearchEntry, query: SearchQuery): boolean {
  if (query.kinds.length) {
    const kind = entryKind(entry);
    if (!kind || !query.kinds.some((value) => kind.startsWith(value)))
      return false;
  }
  if (query.within.length)
    return entry.within.some((name) => {
      const lower = name.toLowerCase();
      return query.within.some((value) => lower.startsWith(value));
    });
  return true;
}

/* --------------------------------------------------------------- ranking -- */

/**
 * How well one term matches: 0 a prefix, 1 the start of a word inside it, 2 a
 * substring anywhere, and `null` for no match at all.
 */
function termRank(term: string, needle: string): number | null {
  const haystack = term.toLowerCase();
  const at = haystack.indexOf(needle);
  if (at < 0) return null;
  if (at === 0) return 0;
  return /[\s/._\-#@]/.test(haystack[at - 1]) ? 1 : 2;
}

/**
 * The entry's rank: the best match over its terms, with the term's own
 * position as the tie-break, so a name match always beats the same kind of
 * match on a path or an alias. Lower is better.
 */
function entryRank(entry: SearchEntry, needle: string): number | null {
  let best: number | null = null;
  entry.terms.forEach((term, index) => {
    const rank = termRank(term, needle);
    if (rank === null) return;
    const score = rank * 4 + Math.min(index, 3);
    if (best === null || score < best) best = score;
  });
  return best;
}

function refKey(ref: ItemRef): string {
  return `${ref.store}\n${ref.dirent}`;
}

/**
 * What an empty query shows under All: Recent, then every favorite, each
 * resolved against the index so that only items the snapshot holds now
 * appear, then every action. A row in both lists appears in both, under its
 * own id.
 */
function startResults(
  index: readonly SearchEntry[],
  lists: RecentFavorites,
): SearchEntry[] {
  const byRef = new Map<string, SearchEntry>();
  for (const entry of index) if (entry.ref) byRef.set(refKey(entry.ref), entry);
  const resolve = (
    refs: readonly ItemRef[],
    group: string,
    prefix: string,
  ): SearchEntry[] =>
    refs.flatMap((ref) => {
      const entry = byRef.get(refKey(ref));
      return entry ? [{ ...entry, id: `${prefix}:${entry.id}`, group }] : [];
    });
  return [
    ...resolve(lists.recent, RECENT_GROUP, 'recent'),
    ...resolve(lists.favorites, FAVORITES_GROUP, 'favorite'),
    ...index.filter((entry) => entry.scope === 'actions'),
  ];
}

/**
 * The results for a query, ranked and capped per group. An empty query under
 * All lists Recent, Favorites and Actions when `start` is given; otherwise an
 * empty query is not a filter and lists the head of every group the scope
 * admits.
 */
export function searchResults(
  index: readonly SearchEntry[],
  query: string,
  scope: SearchScope = 'all',
  limit: number = SEARCH_GROUP_LIMIT,
  start?: RecentFavorites,
): readonly SearchEntry[] {
  const parsed = parseSearchQuery(query);
  const { needle } = parsed;
  const filtered = parsed.within.length > 0 || parsed.kinds.length > 0;
  if (start && scope === 'all' && needle === '' && !filtered)
    return startResults(index, start);
  const ranked: { entry: SearchEntry; rank: number; order: number }[] = [];
  index.forEach((entry, order) => {
    if (scope !== 'all' && entry.scope !== scope) return;
    if (!admitted(entry, parsed)) return;
    const rank = needle === '' ? 0 : entryRank(entry, needle);
    if (rank === null) return;
    ranked.push({ entry, rank, order });
  });
  ranked.sort(
    (left, right) => left.rank - right.rank || left.order - right.order,
  );

  const results: SearchEntry[] = [];
  for (const group of GROUP_ORDER) {
    for (const candidate of ranked) {
      if (candidate.entry.scope !== group) continue;
      if (results.filter((entry) => entry.scope === group).length >= limit)
        break;
      results.push(candidate.entry);
    }
  }
  return results;
}

/* -------------------------------------------------------------- shortcut -- */

/**
 * Opens the palette on ⌘K, or Ctrl-K away from a Mac.
 *
 * The shortcut is not suppressed inside a text field. ⌘K types nothing, so
 * taking it never interrupts typing, and the field a reader is most likely to
 * be in when they reach for it is the palette's own.
 */
export function useSearchShortcut(onOpen: () => void): void {
  const onOpenRef = useRef(onOpen);
  onOpenRef.current = onOpen;
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (!matchesShortcut(event, 'search')) return;
      // This listener runs on document, beyond a dialog's event boundary.
      // Disable the shortcut while any dialog is open, including the palette
      // itself.
      if (anyDialogOpen()) return;
      event.preventDefault();
      onOpenRef.current();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
    };
  }, []);
}

/* ------------------------------------------------------------- the sheet -- */

export interface SearchPaletteProps {
  snapshot: AgentSnapshot;
  open: boolean;
  onClose: () => void;
  /**
   * Destination for a search result. The rail follows the location, so a
   * channel opens in Chat and a person opens in Teams.
   */
  onNavigate: (location: Location) => void;
  /**
   * Opens an item by navigating to its store and selecting it. Selection is not
   * part of `Location`, so one callback applies navigation and selection
   * together when a caller guards navigation. Without this callback, an item
   * result opens the store without selecting the item.
   */
  onOpenItem?: (store: StoreRef, path: string) => void;
  /**
   * Opens a folder in its store's tree, as one guarded move for the same
   * reason as `onOpenItem`. Without it, a folder result opens the store.
   */
  onOpenFolder?: (store: StoreRef, path: string) => void;
  /**
   * Copies one field of an item through the host, after the palette has
   * closed. Without it, ⌘C in the palette copies query text only.
   */
  onCopyItem?: (store: StoreRef, path: string, field: ItemField) => void;
  /**
   * Opens a login's website through the host, after the palette has closed.
   * Without it, ⌘O does nothing in the palette.
   */
  onOpenWebsite?: (store: StoreRef, path: string) => void;
  /** The channels the caller knows; the palette loads none of its own. */
  channels?: readonly SearchChannel[];
  channelReadiness?: CollectionReadiness;
  /** The commands listed under Actions. */
  actions?: readonly SearchAction[];
}

/** The keyboard hints along the foot, which never change. */
function footKeys(): { keys: readonly string[]; label: string }[] {
  return [
    {
      keys: listedChords('paletteMove').map((chord) => chordLabel(chord)),
      label: 'move',
    },
    { keys: [shortcutLabel('paletteOpen')], label: 'open' },
    { keys: [shortcutLabel('paletteKind')], label: 'next kind' },
  ];
}

/** The copy ⌘C makes from an entry, or with Shift the login's user name. */
function copyFor(
  entry: SearchEntry | undefined,
  shift: boolean,
): CopyAction | undefined {
  const copies = entry?.copies ?? [];
  return shift ? copies.find((copy) => copy.field === 'username') : copies[0];
}

/** The keys the highlighted row answers to, for its hints and description. */
function rowKeys(
  entry: SearchEntry,
  favorite: boolean,
  canCopy: boolean,
  canOpenWebsite: boolean,
): { keys: string; label: string }[] {
  const keys = [
    {
      keys: shortcutLabel('paletteOpen'),
      label: entry.target.kind === 'action' ? 'Run' : 'Open',
    },
  ];
  if (canCopy) {
    const copy = copyFor(entry, false);
    if (copy)
      keys.push({ keys: shortcutLabel('paletteCopy'), label: copy.label });
    const user = copyFor(entry, true);
    if (user)
      keys.push({
        keys: shortcutLabel('paletteCopyUsername'),
        label: user.label,
      });
  }
  if (canOpenWebsite && entry.website)
    keys.push({
      keys: shortcutLabel('paletteOpenWebsite'),
      label: 'Open website',
    });
  if (entry.ref)
    keys.push({
      keys: shortcutLabel('paletteFavorite'),
      label: favorite ? 'Remove favorite' : 'Add favorite',
    });
  return keys;
}

function Marked({ text, needle }: { text: string; needle: string }): ReactNode {
  if (!needle) return text;
  const at = text.toLowerCase().indexOf(needle);
  if (at < 0) return text;
  return (
    <>
      {text.slice(0, at)}
      <em>{text.slice(at, at + needle.length)}</em>
      {text.slice(at + needle.length)}
    </>
  );
}

/**
 * Every row's mark is the `md` one a group's mark draws, so an item, a person
 * and a channel sit on the same square as the vault beside them.
 */
function EntryGlyph({ glyph }: { glyph: SearchEntry['glyph'] }): ReactNode {
  switch (glyph.kind) {
    case 'item':
      return (
        <span className={`kico md ${glyph.itemKind}`} aria-hidden="true">
          <Icon name={KINDS[glyph.itemKind].icon} />
        </span>
      );
    case 'folder':
      return (
        <span className="kico md neutral" aria-hidden="true">
          <Icon name="folder" />
        </span>
      );
    case 'store':
      return <GroupMark store={glyph.store} />;
    case 'person':
      return (
        <span
          className="kico md"
          aria-hidden="true"
          style={{ background: hue(glyph.username) }}
        >
          {initials(glyph.username)}
        </span>
      );
    case 'channel':
      return (
        <span className="kico md pal-channel" aria-hidden="true">
          #
        </span>
      );
    case 'action':
      return (
        <span className="kico md neutral" aria-hidden="true">
          <Icon name={glyph.icon} />
        </span>
      );
  }
}

/**
 * The palette's own body. It is mounted only while the palette is open, so the
 * query, the scope and the highlighted row start fresh every time.
 */
function SearchSheet({
  snapshot,
  onClose,
  onNavigate,
  onOpenItem,
  onOpenFolder,
  onCopyItem,
  onOpenWebsite,
  channels = [],
  channelReadiness = 'ready',
  actions = [],
}: Omit<SearchPaletteProps, 'open'>): ReactNode {
  const [query, setQuery] = useState('');
  const [scope, setScope] = useState<SearchScope>('all');
  const [active, setActive] = useState(0);
  const listId = useId();
  const keysId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const lists = useRecentFavorites();
  const inventory = combineReadiness([
    inventoryReadiness(snapshot, 'accounts'),
    inventoryReadiness(snapshot, 'teams'),
  ]);
  const peopleReadiness = combineReadiness([
    inventory,
    ...snapshot.stores
      .filter((store) => store.kind === 'team')
      .map((store) => groupDetailReadiness(snapshot, store.id, 'roster')),
  ]);
  const readiness = combineReadiness([
    ...(scope === 'all' || scope === 'stores' ? [inventory] : []),
    ...(scope === 'all' || scope === 'items' ? [itemsReadiness(snapshot)] : []),
    ...(scope === 'all' || scope === 'people' ? [peopleReadiness] : []),
    ...(scope === 'all' || scope === 'channels'
      ? [combineReadiness([inventory, channelReadiness])]
      : []),
  ]);

  const index = useMemo(
    () => searchIndex(snapshot, channels, actions),
    [snapshot, channels, actions],
  );
  const results = useMemo(
    () => searchResults(index, query, scope, SEARCH_GROUP_LIMIT, lists),
    [index, query, scope, lists],
  );
  const { needle } = parseSearchQuery(query);

  // A shorter result set can strand the highlight past its end.
  const selected = results.length ? Math.min(active, results.length - 1) : -1;
  const optionId = (position: number): string => `${listId}-option-${position}`;
  const highlighted = selected < 0 ? undefined : results[selected];
  const highlightedKeys = highlighted
    ? rowKeys(
        highlighted,
        isFavorite(lists, highlighted.ref ?? null),
        Boolean(onCopyItem),
        Boolean(onOpenWebsite),
      )
    : [];

  const open = (entry: SearchEntry | undefined): void => {
    if (!entry) return;
    onClose();
    const { target } = entry;
    switch (target.kind) {
      case 'item':
        rememberRecent(entry.ref ?? null);
        if (onOpenItem) onOpenItem(target.store, target.path);
        else onNavigate({ kind: 'store', ref: target.store });
        return;
      case 'folder':
        rememberRecent(entry.ref ?? null);
        if (onOpenFolder) onOpenFolder(target.store, target.path);
        else onNavigate({ kind: 'store', ref: target.store });
        return;
      case 'store':
        onNavigate({ kind: 'store', ref: target.ref });
        return;
      case 'person':
        onNavigate({
          kind: 'group-settings',
          ref: target.ref,
          tab: 'people',
        });
        return;
      case 'channel':
        onNavigate({
          kind: 'chat',
          ref: target.ref,
          ...(target.channel ? { channel: target.channel } : {}),
        });
        return;
      case 'action':
        actions.find((action) => action.id === target.id)?.run();
        return;
    }
  };

  const cycleScope = (step: number): void => {
    const at = SEARCH_SCOPES.findIndex((chip) => chip.id === scope);
    const next = (at + step + SEARCH_SCOPES.length) % SEARCH_SCOPES.length;
    setScope(SEARCH_SCOPES[next].id);
    setActive(0);
  };

  const onKeyDown = (event: ReactKeyboardEvent<HTMLElement>): void => {
    const copyUsername = matchesShortcut(event, 'paletteCopyUsername');
    if (copyUsername || matchesShortcut(event, 'paletteCopy')) {
      const copy = copyFor(highlighted, copyUsername);
      const input = inputRef.current;
      // With query text selected, ⌘C copies that text as in any field.
      if (input && input.selectionStart !== input.selectionEnd) return;
      if (!highlighted || !copy || !onCopyItem) return;
      const { target } = highlighted;
      if (target.kind !== 'item') return;
      event.preventDefault();
      onClose();
      onCopyItem(target.store, target.path, copy.field);
      return;
    }
    if (matchesShortcut(event, 'paletteOpenWebsite')) {
      if (!highlighted?.website || !onOpenWebsite) return;
      const { target } = highlighted;
      if (target.kind !== 'item') return;
      event.preventDefault();
      onClose();
      onOpenWebsite(target.store, target.path);
      return;
    }
    if (matchesShortcut(event, 'paletteFavorite')) {
      if (!highlighted?.ref) return;
      event.preventDefault();
      toggleFavorite(highlighted.ref);
      return;
    }
    if (matchesShortcut(event, 'paletteMove')) {
      event.preventDefault();
      setActive(
        event.key === 'ArrowUp'
          ? Math.max(selected - 1, 0)
          : Math.min(selected + 1, results.length - 1),
      );
      return;
    }
    if (matchesShortcut(event, 'paletteOpen')) {
      event.preventDefault();
      open(highlighted);
      return;
    }
    const previousKind = matchesShortcut(event, 'palettePreviousKind');
    if (previousKind || matchesShortcut(event, 'paletteKind')) {
      // The chips are the only thing Tab moves between here: the field keeps
      // focus, so there is nowhere else in the sheet for it to go.
      event.preventDefault();
      cycleScope(previousKind ? -1 : 1);
    }
  };

  // Consecutive rows under one heading, with the flat position each row
  // answers to. Recent and Favorites can hold the same item, so a group is
  // its heading, not the entry's scope.
  const groups: {
    label: string;
    rows: { entry: SearchEntry; position: number }[];
  }[] = [];
  results.forEach((entry, position) => {
    const last = groups.at(-1);
    if (last?.label === entry.group) last.rows.push({ entry, position });
    else groups.push({ label: entry.group, rows: [{ entry, position }] });
  });

  return (
    <DismissibleDialog
      className="backdrop pal-back"
      aria-label="Search"
      onDismiss={onClose}
    >
      <div className="pal" onKeyDown={onKeyDown}>
        <div className="pal-q">
          <Icon name="search" />
          <input
            ref={inputRef}
            type="text"
            value={query}
            data-dialog-autofocus="true"
            autoComplete="off"
            spellCheck={false}
            role="combobox"
            aria-expanded={true}
            aria-controls={listId}
            aria-autocomplete="list"
            aria-activedescendant={
              selected < 0 ? undefined : optionId(selected)
            }
            aria-describedby={keysId}
            aria-label="Search items, vaults, teams, people, channels"
            placeholder="Search items, vaults, teams, people, channels"
            onChange={(event) => {
              setQuery(event.target.value);
              setActive(0);
            }}
          />
          <span className="pal-esc" aria-hidden="true">
            esc
          </span>
          <span id={keysId} className="offscreen">
            {highlightedKeys
              .map((hint) =>
                hint.keys === '↵'
                  ? `Enter ${hint.label}`
                  : `${hint.keys} ${hint.label}`,
              )
              .join(', ')}
          </span>
        </div>

        <div className="pal-scopes" role="group" aria-label="Search scope">
          {SEARCH_SCOPES.map((chip) => (
            <button
              key={chip.id}
              type="button"
              className={chip.id === scope ? 'chip on' : 'chip'}
              aria-pressed={chip.id === scope}
              tabIndex={-1}
              onClick={() => {
                setScope(chip.id);
                setActive(0);
              }}
            >
              {chip.label}
            </button>
          ))}
        </div>

        <CollectionStatus state={readiness} label="search results" />
        <div
          className="pal-results"
          id={listId}
          role="listbox"
          aria-label="Search results"
        >
          {groups.length === 0 &&
          readiness !== 'ready' ? null : groups.length === 0 ? (
            <p className="pal-none">
              {query.trim() ? `No matches for “${query.trim()}”. ` : ''}
              Search item names, paths, usernames, and channels. Narrow with
              in:vault or kind:password.
            </p>
          ) : (
            groups.map((section) => (
              <div
                key={section.label}
                role="group"
                aria-label={section.label}
                className="pal-group"
              >
                <div className="pal-grp">{section.label}</div>
                {section.rows.map(({ entry, position }) => (
                  <div
                    key={entry.id}
                    id={optionId(position)}
                    role="option"
                    aria-selected={position === selected}
                    className={
                      position === selected ? 'pal-hit sel' : 'pal-hit'
                    }
                    onMouseMove={() => {
                      setActive(position);
                    }}
                    onClick={() => {
                      open(entry);
                    }}
                  >
                    <EntryGlyph glyph={entry.glyph} />
                    <span className="t">
                      <b>
                        <Marked text={entry.name} needle={needle} />
                        {isFavorite(lists, entry.ref ?? null) ? (
                          <>
                            <Icon name="star" className="pal-star" />
                            <span className="offscreen">, favorite</span>
                          </>
                        ) : null}
                      </b>
                      <small>{entry.detail}</small>
                    </span>
                    {position === selected ? (
                      <span className="pal-keys" aria-hidden="true">
                        {highlightedKeys.slice(1).map((hint) => (
                          <span key={hint.keys}>
                            <kbd>{hint.keys}</kbd> {hint.label}
                          </span>
                        ))}
                      </span>
                    ) : null}
                    <span className="where">
                      {entry.keys ? (
                        <kbd aria-hidden="true">{entry.keys}</kbd>
                      ) : (
                        entry.where
                      )}
                    </span>
                  </div>
                ))}
              </div>
            ))
          )}
        </div>

        <div className="pal-foot">
          {footKeys().map((hint) => (
            <span className="k" key={hint.label}>
              {hint.keys.map((key) => (
                <kbd key={key}>{key}</kbd>
              ))}{' '}
              {hint.label}
            </span>
          ))}
          <span className="pal-note">
            Names and paths are searched; values are not.
          </span>
          <span className="pal-count" aria-live="polite">
            {results.length === 1 ? '1 result' : `${results.length} results`}
          </span>
        </div>
      </div>
    </DismissibleDialog>
  );
}

/** The palette. Closed, it draws nothing at all. */
export function SearchPalette(props: SearchPaletteProps): ReactNode {
  const { open, ...sheet } = props;
  const blocking = useDismissOnOverlayBlock(sheet.onClose, open);
  if (!open || blocking) return null;
  return <SearchSheet {...sheet} />;
}
