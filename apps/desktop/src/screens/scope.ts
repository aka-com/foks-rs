/**
 * Item filtering and sorting utilities for vault catalog views.
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

/** Returns the store display name for an item. */
export function whereOf(world: World, item: Item): string {
  const store = storeOf(world, item.store);
  return store?.name ?? item.store;
}

/**
 * Computes reader metadata for an item based on the store roster and read role.
 */
export function readableBy(
  world: World,
  item: Item,
): { label: string; title?: string } {
  const readers = readersOf(world, item);
  if (!readers) return { label: 'Only you' };
  return {
    label: peopleLabel(readers.length),
    title: readers.map(partyName).join(', '),
  };
}

const SORTS: Readonly<
  Record<SortKey, (world: World) => (a: Item, b: Item) => number>
> = {
  name: () => (a, b) => nameOf(a.path).localeCompare(nameOf(b.path)),
  kind: () => (a, b) =>
    KIND_LIST.indexOf(kindOf(a) as (typeof KIND_LIST)[number]) -
      KIND_LIST.indexOf(kindOf(b) as (typeof KIND_LIST)[number]) ||
    nameOf(a.path).localeCompare(nameOf(b.path)),
  group: (world) => (a, b) =>
    (storeOf(world, a.store)?.name ?? '').localeCompare(
      storeOf(world, b.store)?.name ?? '',
    ) || nameOf(a.path).localeCompare(nameOf(b.path)),
};

export interface FolderNode {
  name: string;
  path: string;
  folders: FolderNode[];
  items: Item[];
  count: number;
}

interface MutableFolderNode extends Omit<FolderNode, 'folders'> {
  children: Map<string, MutableFolderNode>;
}

/** Build a folders-only tree from item paths. Folder nodes are derived catalog views. */
export function folderTree(items: readonly Item[]): FolderNode {
  const root: MutableFolderNode = {
    name: '',
    path: '/',
    children: new Map(),
    items: [],
    count: 0,
  };
  for (const item of items) {
    const parts = item.path.split('/').filter(Boolean);
    let node = root;
    for (const part of parts.slice(0, -1)) {
      let child = node.children.get(part);
      if (!child) {
        child = {
          name: part,
          path: node.path === '/' ? `/${part}` : `${node.path}/${part}`,
          children: new Map(),
          items: [],
          count: 0,
        };
        node.children.set(part, child);
      }
      node = child;
    }
    node.items.push(item);
  }
  const finish = (node: MutableFolderNode): FolderNode => {
    const folders = [...node.children.values()]
      .sort((a, b) => a.name.localeCompare(b.name))
      .map(finish);
    return {
      name: node.name,
      path: node.path,
      folders,
      items: node.items,
      count:
        node.items.length +
        folders.reduce((sum, child) => sum + child.count, 0),
    };
  };
  return finish(root);
}

/** Find a derived folder by its absolute path. */
export function folderAt(root: FolderNode, path: string): FolderNode | null {
  if (!path || path === '/') return root;
  let node: FolderNode | undefined = root;
  for (const part of path.split('/').filter(Boolean)) {
    node = node.folders.find((folder) => folder.name === part);
    if (!node) return null;
  }
  return node;
}

/** Checks whether the given location displays catalog items. */
export function listsItems(location: Location): boolean {
  return location.kind === 'all' || location.kind === 'store';
}

/**
 * Filters and sorts items based on location, kind filter, search query, and sort key.
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
