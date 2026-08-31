/**
 * What is listed, in what order — `01-vault.html`'s `scoped()`.
 *
 * Pure functions of a world and a navigation state, so the list a screen
 * draws and the list a test asserts are the same list. Nothing here formats
 * markup; the numbers and strings it returns are the model's.
 */

import {
  KIND_LIST,
  catalog,
  kindOf,
  nameOf,
  readersOf,
  partyName,
  peopleLabel,
  storeOf,
} from '../model';
import type { Item, World } from '../model';
import type { Location, LocationState, SortKey } from '../location';

/** "Personal" — the store caption under a row's name; Server has its own column. */
export function whereOf(world: World, item: Item): string {
  const store = storeOf(world, item.store);
  return store?.name ?? item.store;
}

/**
 * The "Readable by" cell.
 *
 * Always the computation, never prose: the roster filtered by the item's read
 * role. An account store has no roster, so it answers "only you" — sharing
 * anything means putting it in a group.
 */
export function readableBy(
  world: World,
  item: Item,
): { label: string; title?: string } {
  const readers = readersOf(world, item);
  if (!readers) return { label: 'only you' };
  return {
    label: peopleLabel(readers.length),
    title: readers.map(partyName).join(', '),
  };
}

const SORTS: Readonly<Record<SortKey, (world: World) => (a: Item, b: Item) => number>> =
  {
    name: () => (a, b) => nameOf(a.path).localeCompare(nameOf(b.path)),
    kind: () => (a, b) =>
      KIND_LIST.indexOf(kindOf(a) as (typeof KIND_LIST)[number]) -
        KIND_LIST.indexOf(kindOf(b) as (typeof KIND_LIST)[number]) ||
      nameOf(a.path).localeCompare(nameOf(b.path)),
    group: (world) => (a, b) =>
      (storeOf(world, a.store)?.name ?? '').localeCompare(
        storeOf(world, b.store)?.name ?? '',
      ) || nameOf(a.path).localeCompare(nameOf(b.path)),
    version: () => (a, b) =>
      b.version - a.version || nameOf(a.path).localeCompare(nameOf(b.path)),
  };

/** Whether a location lists items at all. */
export function listsItems(location: Location): boolean {
  return location.kind === 'all' || location.kind === 'store';
}

/**
 * The items a location, a kind filter, a query and a sort choose.
 *
 * Search covers paths and store names, as the mock's does — contents stay
 * masked, so they are not searched.
 */
export function scopedItems(world: World, state: LocationState): Item[] {
  const { location, kind, query, sort } = state;
  let items = catalog(world);
  if (location.kind === 'store') {
    items = items.filter((item) => item.store === location.ref);
  }
  if (kind !== 'All') {
    items = items.filter((item) => kindOf(item) === kind);
  }
  if (query) {
    const needle = query.toLowerCase();
    items = items.filter(
      (item) =>
        item.path.toLowerCase().includes(needle) ||
        (storeOf(world, item.store)?.name ?? '').toLowerCase().includes(needle),
    );
  }
  return [...items].sort(SORTS[sort](world));
}
