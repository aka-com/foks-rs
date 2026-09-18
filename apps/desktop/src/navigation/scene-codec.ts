import type { LeaseState } from '../model/types';
import {
  decodeProductionLocation,
  encodeLocation,
  setUrl,
} from './production-codec';
import { INITIAL_SCENE } from './types';
import type {
  KindFilter,
  LocationState,
  Scene,
  Selection,
  SortKey,
  ViewMode,
} from './types';

const VIEWS: readonly ViewMode[] = ['list'];
/** View values older URLs carried, before the tree replaced the view toggle. */
const LEGACY_VIEWS: Readonly<Record<string, ViewMode>> = {
  grid: 'list',
  folders: 'list',
};
const KIND_FILTERS: readonly KindFilter[] = ['All', 'Password', 'Document'];
/** Kind values older URLs carried, before Notes, Files and Links became Documents. */
const LEGACY_KINDS: Readonly<Record<string, KindFilter>> = {
  Resource: 'Document',
  File: 'Document',
  Link: 'Document',
};
const SORTS: readonly SortKey[] = ['name', 'kind', 'group'];

export function oneOf<T extends string>(
  values: readonly T[],
  candidate: string | null,
): T | null {
  return candidate && (values as readonly string[]).includes(candidate)
    ? (candidate as T)
    : null;
}

/** `store|path`, the key the fixture's `plaintext` map is written with. */
function decodeSelection(value: string | null): Selection {
  if (!value) return null;
  const cut = value.indexOf('|');
  if (cut <= 0) return null;
  return { store: value.slice(0, cut), path: value.slice(cut + 1) };
}

type SceneNavigation = Pick<
  Scene,
  'selection' | 'view' | 'kind' | 'sort' | 'folder' | 'closedFolders'
>;

export function decodeSceneNavigation(
  params: URLSearchParams,
  alias: Partial<SceneNavigation> = {},
): SceneNavigation {
  return {
    selection: decodeSelection(params.get('sel')) ?? alias.selection ?? null,
    view:
      oneOf(VIEWS, params.get('view')) ??
      LEGACY_VIEWS[params.get('view') ?? ''] ??
      alias.view ??
      INITIAL_SCENE.view,
    kind:
      oneOf(KIND_FILTERS, params.get('kind')) ??
      LEGACY_KINDS[params.get('kind') ?? ''] ??
      alias.kind ??
      INITIAL_SCENE.kind,
    sort: oneOf(SORTS, params.get('sort')) ?? alias.sort ?? INITIAL_SCENE.sort,
    folder: params.get('folder') ?? alias.folder ?? INITIAL_SCENE.folder,
    closedFolders: (params.get('closed') ?? '').split(',').filter(Boolean),
  };
}

export function decodeProductionScene(search: string): Scene {
  return {
    ...INITIAL_SCENE,
    location: decodeProductionLocation(search) ?? INITIAL_SCENE.location,
    ...decodeSceneNavigation(new URLSearchParams(search)),
  };
}

/**
 * Encodes a scene into a URL string, omitting parameters that match default values.
 */
export function sceneHref(href: string, scene: Scene): string {
  const { state, params } = encodeLocation(scene.location);
  return setUrl(href, state, {
    ...params,
    sel: scene.selection
      ? `${scene.selection.store}|${scene.selection.path}`
      : null,
    view: scene.view === INITIAL_SCENE.view ? null : scene.view,
    kind: scene.kind === INITIAL_SCENE.kind ? null : scene.kind,
    sort: scene.sort === INITIAL_SCENE.sort ? null : scene.sort,
    folder: scene.folder || null,
    closed: scene.closedFolders.length ? scene.closedFolders.join(',') : null,
    lease: scene.lease === INITIAL_SCENE.lease ? null : scene.lease,
  });
}

/** Constructs a Scene from location state and active lease status. */
export function sceneOf(state: LocationState, lease: LeaseState): Scene {
  return {
    location: state.location,
    selection: state.selection,
    view: state.view,
    kind: state.kind,
    sort: state.sort,
    folder: state.folder,
    closedFolders: state.closedFolders,
    lease,
    reveal: false,
    demo: null,
  };
}
