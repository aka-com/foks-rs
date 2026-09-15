/**
 * The global search palette (⌘K).
 *
 * One field over whatever page is open, reaching every kind the shell knows:
 * item names and paths, vaults, groups and shares, the people on a roster, and
 * the chat channels the caller can name. It replaces the "All items" page and
 * the per-page search field, which only filtered paths inside one store.
 *
 * The palette is a sheet, not a location: it carries no `?state=` and pushes no
 * history entry. Opening a result navigates, which is what the history records.
 *
 * `searchIndex` and `searchResults` are pure so the index and the ranking can
 * be tested without a DOM.
 */

import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from 'react';

import { DismissibleDialog } from '/kit/overlay-primitives';
import { Icon } from '../components/icon';
import { GroupMark } from '../screens/group-mark';
import {
  KINDS,
  hue,
  initials,
  isMachine,
  kindOf,
  nameOf,
  parseRole,
  prefixOf,
  roleName,
  serverName,
  storeDescription,
  storeNavigationOrder,
  storeOf,
} from '../model';
import type { AgentSnapshot, Store, StoreRef } from '../model';
import type { Location } from '../location';
import type { FoksIconName } from '../icons';

import './search-palette.css';

/* ---------------------------------------------------------------- scopes -- */

/** The chip a result is filtered by. `all` filters nothing. */
export type SearchScope = 'all' | 'items' | 'stores' | 'people' | 'channels';

/** The chips in the order the strip draws them, which is the order Tab walks. */
export const SEARCH_SCOPES: readonly {
  id: SearchScope;
  label: string;
}[] = [
  { id: 'all', label: 'All' },
  { id: 'items', label: 'Items' },
  { id: 'stores', label: 'Vaults & groups' },
  { id: 'people', label: 'People' },
  { id: 'channels', label: 'Channels' },
];

/** The heading each scope's results sit under, in group order. */
const GROUP_LABELS: Readonly<Record<Exclude<SearchScope, 'all'>, string>> = {
  items: 'Items',
  stores: 'Vaults, groups and shares',
  people: 'People',
  channels: 'Channels',
};

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
  | { kind: 'store'; ref: StoreRef }
  | { kind: 'person'; ref: StoreRef; username: string }
  | { kind: 'channel'; ref: StoreRef; channel?: string };

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
  /** The tile drawn at the head of the row. */
  glyph:
    | { kind: 'item'; itemKind: keyof typeof KINDS }
    | { kind: 'store'; store: Store }
    | { kind: 'person'; username: string }
    | { kind: 'channel' };
  target: SearchTarget;
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
  return store.team_kind === 'adhoc' ? 'Share' : 'Group';
}

/** A store's own alias, which is a second name people search by. */
function storeAlias(store: Store): string {
  return store.kind === 'account' ? store.account : store.alias;
}

function itemEntries(snapshot: AgentSnapshot): SearchEntry[] {
  const entries: SearchEntry[] = [];
  for (const item of snapshot.items) {
    const kind = kindOf(item);
    // Folder results are excluded because selecting a folder requires
    // navigating to its path rather than opening an item.
    if (kind === 'Folder') continue;
    const store = storeOf(snapshot, item.store);
    if (!store) continue;
    const folder = prefixOf(item.path);
    entries.push({
      id: `item:${item.store}|${item.path}`,
      scope: 'items',
      group: GROUP_LABELS.items,
      name: nameOf(item.path),
      detail: [KINDS[kind].label, folder, store.name]
        .filter(Boolean)
        .join(' · '),
      where: serverName(snapshot, store),
      terms: [nameOf(item.path), item.path],
      glyph: { kind: 'item', itemKind: kind },
      target: { kind: 'item', store: item.store, path: item.path },
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
    where: serverName(snapshot, store),
    terms: [store.name, storeAlias(store)],
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
    { store: Store; username: string; groups: string[]; role: string }
  >();
  for (const party of snapshot.parties) {
    if (party.party_kind !== 'user' || !party.username) continue;
    const store = storeOf(snapshot, party.store);
    if (!store) continue;
    const key = `${store.server}|${party.username}`;
    const role = parseRole(party.destination_role);
    const found = seen.get(key);
    if (found) {
      if (!found.groups.includes(store.name)) found.groups.push(store.name);
      continue;
    }
    order.push(key);
    seen.set(key, {
      store,
      username: party.username,
      groups: [store.name],
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
      where: serverName(snapshot, person.store),
      terms: [person.username],
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
      where: serverName(snapshot, store),
      // `#deploys` first, so typing the hash reaches the channel and typing
      // the group name reaches every channel in it.
      terms: [`#${channel.name}`, channel.name, store.name],
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

/**
 * Everything the palette can find, in group order. Channels come from the
 * caller because the palette does not mount the chat inbox; with none, the
 * other three kinds still search.
 */
export function searchIndex(
  snapshot: AgentSnapshot,
  channels: readonly SearchChannel[] = [],
): readonly SearchEntry[] {
  return [
    ...itemEntries(snapshot),
    ...storeEntries(snapshot),
    ...peopleEntries(snapshot),
    ...channelEntries(snapshot, channels),
  ];
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

/**
 * The results for a query, ranked and capped per group. An empty query is not
 * a filter: it lists the head of every group the scope admits, which is what
 * the palette shows the moment it opens.
 */
export function searchResults(
  index: readonly SearchEntry[],
  query: string,
  scope: SearchScope = 'all',
  limit: number = SEARCH_GROUP_LIMIT,
): readonly SearchEntry[] {
  const needle = query.trim().toLowerCase();
  const ranked: { entry: SearchEntry; rank: number; order: number }[] = [];
  index.forEach((entry, order) => {
    if (scope !== 'all' && entry.scope !== scope) return;
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
      if (!(event.metaKey || event.ctrlKey) || event.altKey) return;
      if (event.key.toLowerCase() !== 'k') return;
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
   * Selects the item a result names, after the navigation to its store page.
   * Selection is not part of `Location`, so without this an item result opens
   * its store and nothing is selected.
   */
  onOpenItem?: (store: StoreRef, path: string) => void;
  /** The channels the caller knows; the palette loads none of its own. */
  channels?: readonly SearchChannel[];
}

/** The keyboard hints along the foot, which never change. */
const FOOT_KEYS: readonly { keys: readonly string[]; label: string }[] = [
  { keys: ['↑', '↓'], label: 'move' },
  { keys: ['↵'], label: 'open' },
  { keys: ['tab'], label: 'next kind' },
];

function Marked({ text, query }: { text: string; query: string }): ReactNode {
  const needle = query.trim().toLowerCase();
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

function EntryGlyph({ glyph }: { glyph: SearchEntry['glyph'] }): ReactNode {
  switch (glyph.kind) {
    case 'item':
      return (
        <span className={`kico ${glyph.itemKind}`} aria-hidden="true">
          <Icon name={KINDS[glyph.itemKind].icon as FoksIconName} />
        </span>
      );
    case 'store':
      return <GroupMark store={glyph.store} />;
    case 'person':
      return (
        <span
          className="kico"
          aria-hidden="true"
          style={{ background: hue(glyph.username) }}
        >
          {initials(glyph.username)}
        </span>
      );
    case 'channel':
      return (
        <span className="kico pal-channel" aria-hidden="true">
          #
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
  channels = [],
}: Omit<SearchPaletteProps, 'open'>): ReactNode {
  const [query, setQuery] = useState('');
  const [scope, setScope] = useState<SearchScope>('all');
  const [active, setActive] = useState(0);
  const listId = useId();

  const index = useMemo(
    () => searchIndex(snapshot, channels),
    [snapshot, channels],
  );
  const results = useMemo(
    () => searchResults(index, query, scope),
    [index, query, scope],
  );

  // A shorter result set can strand the highlight past its end.
  const selected = results.length ? Math.min(active, results.length - 1) : -1;
  const optionId = (position: number): string => `${listId}-option-${position}`;

  const open = (entry: SearchEntry | undefined): void => {
    if (!entry) return;
    onClose();
    const { target } = entry;
    switch (target.kind) {
      case 'item':
        onNavigate({ kind: 'store', ref: target.store });
        onOpenItem?.(target.store, target.path);
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
    }
  };

  const cycleScope = (step: number): void => {
    const at = SEARCH_SCOPES.findIndex((chip) => chip.id === scope);
    const next = (at + step + SEARCH_SCOPES.length) % SEARCH_SCOPES.length;
    setScope(SEARCH_SCOPES[next].id);
    setActive(0);
  };

  const onKeyDown = (event: ReactKeyboardEvent<HTMLElement>): void => {
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setActive(Math.min(selected + 1, results.length - 1));
      return;
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault();
      setActive(Math.max(selected - 1, 0));
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      open(results[selected]);
      return;
    }
    if (event.key === 'Tab') {
      // The chips are the only thing Tab moves between here: the field keeps
      // focus, so there is nowhere else in the sheet for it to go.
      event.preventDefault();
      cycleScope(event.shiftKey ? -1 : 1);
    }
  };

  // The groups, in group order, with the flat position each row answers to.
  const groups = GROUP_ORDER.map((group) => ({
    group,
    label: GROUP_LABELS[group],
    rows: results
      .map((entry, position) => ({ entry, position }))
      .filter((row) => row.entry.scope === group),
  })).filter((section) => section.rows.length > 0);

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
            aria-label="Search items, vaults, groups, people, channels"
            placeholder="Search items, vaults, groups, people, channels"
            onChange={(event) => {
              setQuery(event.target.value);
              setActive(0);
            }}
          />
          <span className="pal-esc" aria-hidden="true">
            esc
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

        <div
          className="pal-results"
          id={listId}
          role="listbox"
          aria-label="Search results"
        >
          {groups.length === 0 ? (
            <p className="pal-none">
              {query.trim()
                ? `No matches for “${query.trim()}”.`
                : 'Nothing to search yet.'}{' '}
              Search covers item names and paths, store names, usernames and
              channels — not item contents.
            </p>
          ) : (
            groups.map((section) => (
              <div
                key={section.group}
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
                        <Marked text={entry.name} query={query} />
                      </b>
                      <small>{entry.detail}</small>
                    </span>
                    <span className="where">{entry.where}</span>
                  </div>
                ))}
              </div>
            ))
          )}
        </div>

        <div className="pal-foot">
          {FOOT_KEYS.map((hint) => (
            <span className="k" key={hint.label}>
              {hint.keys.map((key) => (
                <kbd key={key}>{key}</kbd>
              ))}{' '}
              {hint.label}
            </span>
          ))}
          <span className="grow" />
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
  if (!open) return null;
  return <SearchSheet {...sheet} />;
}
