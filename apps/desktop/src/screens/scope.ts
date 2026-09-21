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
import type { Item, AgentSnapshot } from '../model';
import type { Location, LocationState, SortKey } from '../location';

/** Returns the store display name for an item. */
export function whereOf(snapshot: AgentSnapshot, item: Item): string {
  const store = storeOf(snapshot, item.store);
  return store?.name ?? item.store;
}

/**
 * Computes reader metadata for an item based on the store roster and read role.
 */
export function readableBy(
  snapshot: AgentSnapshot,
  item: Item,
): { label: string; title?: string } {
  const readers = readersOf(snapshot, item);
  if (!readers) return { label: 'Only you' };
  return {
    label: peopleLabel(readers.length),
    title: readers.map(partyName).join(', '),
  };
}

const SORTS: Readonly<
  Record<SortKey, (snapshot: AgentSnapshot) => (a: Item, b: Item) => number>
> = {
  name: () => (a, b) => nameOf(a.path).localeCompare(nameOf(b.path)),
  kind: () => (a, b) =>
    KIND_LIST.indexOf(kindOf(a) as (typeof KIND_LIST)[number]) -
      KIND_LIST.indexOf(kindOf(b) as (typeof KIND_LIST)[number]) ||
    nameOf(a.path).localeCompare(nameOf(b.path)),
  group: (snapshot) => (a, b) =>
    (storeOf(snapshot, a.store)?.name ?? '').localeCompare(
      storeOf(snapshot, b.store)?.name ?? '',
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

/**
 * Checks whether the given location displays catalog items. `files` is the
 * Files tab's own root; it draws the same folder browser as `all` does, since
 * there is no separate landing page for it to fall through to any more.
 */
export function listsItems(location: Location): boolean {
  return (
    location.kind === 'all' ||
    location.kind === 'store' ||
    location.kind === 'files'
  );
}

/**
 * Sentinel store identifier representing all items across all stores. It is not
 * a valid store ID and cannot collide with one.
 */
export const ALL_ITEMS = '*';

/**
 * The folder value the tree and the list read and write: `''` in a `store`
 * location (nothing narrower than the store's own root selected), `path`
 * otherwise, `''` again once the path itself is the root.
 */
export function folderKey(
  storePage: boolean,
  store: string,
  path: string,
): string {
  if (storePage) return path === '/' ? '' : path;
  return `${store}|${path}`;
}

/**
 * Parses the tree/list's selected-folder value into a store id and path:
 * `store|/path` outside a `store` location, `/path` (or `''`, meaning root)
 * inside one, and the `ALL_ITEMS` sentinel for the "All items" view. Outside
 * a `store` location, an empty or malformed value defaults to `ALL_ITEMS`.
 */
export function folderSelection(
  location: Location,
  value: string,
): { store: string; path: string } {
  if (location.kind === 'store') {
    return { store: location.ref, path: value || '/' };
  }
  if (!value || value === ALL_ITEMS) return { store: ALL_ITEMS, path: '/' };
  const cut = value.indexOf('|');
  return cut > 0
    ? { store: value.slice(0, cut), path: value.slice(cut + 1) || '/' }
    : { store: ALL_ITEMS, path: '/' };
}

/**
 * The Files tab's crumb trail beyond "Files" — the store, then each folder
 * down to the one the tree has selected — and the folder value one step
 * back from it. Reads the exact selection `ItemsScreen` reads, so the
 * topbar's crumb and the rail's Back chevron never disagree with the page's
 * own breadcrumb; a stale deep link to a folder that no longer exists falls
 * back to the store's root the same way `ItemsScreen` does, rather than
 * naming a folder that is not there.
 */
export function filesFolderCrumb(
  snapshot: AgentSnapshot | undefined,
  location: Location,
  folder: string,
): { labels: readonly string[]; back: string | null } {
  if (!listsItems(location)) return { labels: [], back: null };
  const selected = folderSelection(location, folder);
  // The "All items" leaf is the tab's own root: nothing to step back to.
  if (selected.store === ALL_ITEMS)
    return { labels: ['All items'], back: null };
  if (!snapshot) return { labels: [], back: null };
  const store = storeOf(snapshot, selected.store);
  if (!store) return { labels: [], back: null };
  const storePage = location.kind === 'store';
  const root = folderTree(
    catalog(snapshot).filter((item) => item.store === store.id),
  );
  const node = folderAt(root, selected.path) ?? root;
  const parts = node.path === '/' ? [] : node.path.split('/').filter(Boolean);
  const back =
    parts.length > 0
      ? folderKey(
          storePage,
          store.id,
          parts.length > 1 ? `/${parts.slice(0, -1).join('/')}` : '/',
        )
      : storePage
        ? null
        : '';
  return { labels: [store.name, ...parts], back };
}

/**
 * Filters and sorts items based on location, kind filter, search query, and sort key.
 */
export function scopedItems(
  snapshot: AgentSnapshot,
  state: LocationState,
): Item[] {
  const { location, kind, query, sort } = state;
  let items: readonly Item[] = catalog(snapshot);
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
        (storeOf(snapshot, item.store)?.name ?? '')
          .toLowerCase()
          .includes(needle),
    );
  }
  return [...items].sort(SORTS[sort](snapshot));
}
