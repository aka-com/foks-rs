/**
 * Where the shell is, and how that survives a reload.
 *
 * A small external store on `ui/src/ui-store.ts`'s pattern: the state is a
 * plain value, `transition` is a pure function of it, and React subscribes
 * through `useSyncExternalStore`. Nothing here touches the DOM, so every
 * transition and every URL round trip is testable without a renderer.
 *
 * First run is a **mode, not a window** (`Location` kind `first-run`): the
 * same shell, with the sidebar in its step-list form.
 *
 * Every place that names an account or a group names it by **StoreRef** — the
 * exact `Store.id` the agent projects — and never by alias. An alias is a
 * profile-local label, so two profiles may each hold an account called
 * `personal`; a location keyed by one would silently mean whichever the
 * catalog happened to list first.
 */

import { useSyncExternalStore } from 'react';
import type { LeaseState, StoreRef } from './model/types';

/* ------------------------------------------------------------- location -- */

/** Which settings pane is open. */
export type SettingsSection =
  'macs' | 'phrase' | 'keys' | 'account' | 'servers' | 'groups' | 'about';

export type GroupSettingsTab = 'people' | 'settings';

/** A step in the first-run state machine. */
export type FirstRunStep = string;
export type FirstRunPath = 'invited' | 'own';

export type Location =
  | { kind: 'all' }
  | { kind: 'store'; ref: StoreRef }
  | { kind: 'group-settings'; ref: StoreRef; tab?: GroupSettingsTab }
  | { kind: 'alerts' }
  /**
   * `profile` names the server the Servers section is open on; it means
   * nothing on any other section and is dropped when moving between them.
   */
  | {
      kind: 'settings';
      section?: SettingsSection;
      store?: StoreRef;
      profile?: string;
    }
  | { kind: 'first-run'; step: FirstRunStep; path?: FirstRunPath };

/** The item the details panel is showing, or nothing. */
export type Selection = { store: StoreRef; path: string } | null;

/** List or cards. A view preference, not a place. */
export type ViewMode = 'list' | 'grid';

/** The kind segmented control's choice. `All` is not a kind, it is no filter. */
export type KindFilter = 'All' | 'Password' | 'Resource' | 'File' | 'Link';

/** The sort menu's choice. */
export type SortKey = 'name' | 'kind' | 'group' | 'version';

export interface LocationState {
  location: Location;
  selection: Selection;
  /** The search field's contents — client-side search over paths. */
  query: string;
  view: ViewMode;
  /** Whether the details panel is open. Selecting an item opens it. */
  details: boolean;
  kind: KindFilter;
  sort: SortKey;
}

export const INITIAL_STATE: LocationState = {
  location: { kind: 'all' },
  selection: null,
  query: '',
  view: 'list',
  details: false,
  kind: 'All',
  sort: 'name',
};

/* ------------------------------------------------------------ transition -- */

export type LocationAction =
  | { type: 'navigate'; location: Location }
  | { type: 'select'; selection: Selection }
  | { type: 'search'; query: string }
  | { type: 'view'; view: ViewMode }
  | { type: 'details'; open: boolean }
  | { type: 'kind'; kind: KindFilter }
  | { type: 'sort'; sort: SortKey };

/** Two locations are the same place. */
export function sameLocation(a: Location, b: Location): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === 'store' && b.kind === 'store') return a.ref === b.ref;
  if (a.kind === 'group-settings' && b.kind === 'group-settings')
    return a.ref === b.ref && a.tab === b.tab;
  if (a.kind === 'settings' && b.kind === 'settings')
    return (
      a.section === b.section && a.store === b.store && a.profile === b.profile
    );
  if (a.kind === 'first-run' && b.kind === 'first-run')
    return a.step === b.step && a.path === b.path;
  return true;
}

/**
 * The whole navigation model, as one pure function.
 *
 * Moving to a different place drops the selection — the details panel cannot
 * show an item from a store you have left — but re-navigating to where you
 * already are keeps it, so a redundant click does not close the panel. The
 * search box is deliberately *not* cleared: a person filtering "wifi" and
 * switching stores is still looking for the same thing.
 */
export function transition(
  state: LocationState,
  action: LocationAction,
): LocationState {
  switch (action.type) {
    case 'navigate':
      return sameLocation(state.location, action.location)
        ? state
        : { ...state, location: action.location, selection: null };
    case 'select':
      // Selecting an item opens the panel that describes it; deselecting
      // leaves the panel where the person put it, empty, rather than closing
      // a pane they opened on purpose.
      return {
        ...state,
        selection: action.selection,
        details: action.selection ? true : state.details,
      };
    case 'search':
      return state.query === action.query
        ? state
        : { ...state, query: action.query };
    case 'view':
      return state.view === action.view
        ? state
        : { ...state, view: action.view };
    case 'details':
      return state.details === action.open
        ? state
        : { ...state, details: action.open };
    case 'kind':
      return state.kind === action.kind
        ? state
        : { ...state, kind: action.kind };
    case 'sort':
      return state.sort === action.sort
        ? state
        : { ...state, sort: action.sort };
  }
}

/* ------------------------------------------------------------- URL codec -- */

/**
 * Read one state value from a deep link.
 */
export function getState(
  search: string,
  fallback: string,
  param = 'state',
): string {
  return new URLSearchParams(search).get(param) ?? fallback;
}

/**
 * Write state values as a pure function of a URL rather than a
 * `history.replaceState` side effect.
 */
export function setUrl(
  href: string,
  state: string,
  params: Readonly<Record<string, string | null>> = {},
): string {
  const url = new URL(href);
  url.searchParams.set('state', state);
  for (const [key, value] of Object.entries(params)) {
    if (value == null) url.searchParams.delete(key);
    else url.searchParams.set(key, value);
  }
  return url.toString();
}

/**
 * The stable `?state=` names for the places that are locations. States that
 * are a *sheet* over a location — `new`,
 * `manage`, `conflict` — are not locations and are not listed; those belong to
 * the screens that own them.
 */
const STATE_ALIASES: Readonly<Record<string, Location>> = {
  all: { kind: 'all' },
  personal: { kind: 'store', ref: 'acct:personal' },
  work: { kind: 'store', ref: 'acct:work' },
  household: { kind: 'store', ref: 'team:household' },
  group: { kind: 'store', ref: 'team:household' },
  homelab: { kind: 'store', ref: 'team:homelab' },
  alerts: { kind: 'alerts' },
  join: { kind: 'settings', section: 'groups' },
  'join-invite': { kind: 'settings', section: 'groups' },
  groups: { kind: 'settings', section: 'groups' },
  people: { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
  party: { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
  federation: { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
  items: { kind: 'store', ref: 'team:eng' },
  danger: { kind: 'group-settings', ref: 'team:eng', tab: 'settings' },
  'rekey-menu': { kind: 'group-settings', ref: 'team:eng', tab: 'settings' },
  invite: { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
  add: { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
  demote: { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
  remove: { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
  admit: { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
  create: { kind: 'settings', section: 'groups' },
  // Servers used to be its own page; it is a Settings section now, and every
  // former name for that page maps to this section.
  servers: { kind: 'settings', section: 'servers' },
  settings: { kind: 'settings' },
  'servers-list': { kind: 'settings', section: 'servers' },
  'servers-server': {
    kind: 'settings',
    section: 'servers',
    profile: 'personal',
  },
  'servers-lapsed': { kind: 'settings', section: 'servers', profile: 'acme' },
  'servers-rollback': {
    kind: 'settings',
    section: 'servers',
    profile: 'personal',
  },
  'servers-reset': {
    kind: 'settings',
    section: 'servers',
    profile: 'personal',
  },
  'servers-add': { kind: 'settings', section: 'servers' },
  'servers-unprobed': {
    kind: 'settings',
    section: 'servers',
    profile: 'partner',
  },
  'servers-check': { kind: 'settings', section: 'servers', profile: 'partner' },
  'settings-macs': { kind: 'settings', section: 'macs' },
  'settings-macs-work': {
    kind: 'settings',
    section: 'macs',
    store: 'acct:work',
  },
  'settings-phrase': { kind: 'settings', section: 'phrase' },
  'settings-keys': { kind: 'settings', section: 'keys' },
  'settings-enrol': { kind: 'settings', section: 'keys' },
  'settings-account': { kind: 'settings', section: 'account' },
  'settings-agent': { kind: 'settings', section: 'about' },
  'settings-about': { kind: 'settings', section: 'about' },
};

/**
 * Every parameter any location owns, all cleared.
 *
 * `setUrl` leaves a parameter it is not told about alone, so a location that
 * names only its own parameters leaves the previous one's behind: a store
 * would inherit `section=` from Settings, and a `?state=all` reload would carry
 * a `store=` that means nothing there. Each encoder therefore spreads this and
 * overrides only what it owns. `account` is not a parameter of any location
 * any more — Settings is keyed by StoreRef — and is listed so that an address
 * saved before that change loses it rather than carrying a stale alias.
 */
const CLEARED_PARAMS: Readonly<Record<string, string | null>> = {
  store: null,
  section: null,
  profile: null,
  step: null,
  path: null,
  tab: null,
  account: null,
};

/** The `?state=` value and extra parameters a location deep-links as. */
export function encodeLocation(location: Location): {
  state: string;
  params: Record<string, string | null>;
} {
  switch (location.kind) {
    case 'store':
      return {
        state: 'store',
        params: { ...CLEARED_PARAMS, store: location.ref },
      };
    case 'group-settings':
      return {
        state: 'group-settings',
        params: {
          ...CLEARED_PARAMS,
          store: location.ref,
          tab: location.tab ?? null,
        },
      };
    case 'settings':
      // The exact account this Settings page acts on, not its alias: two
      // profiles may both hold an account called `personal`.
      return {
        state: 'settings',
        params: {
          ...CLEARED_PARAMS,
          store: location.store ?? null,
          section: location.section ?? null,
          profile:
            location.section === 'servers' ? (location.profile ?? null) : null,
        },
      };
    case 'first-run':
      return {
        state: 'first-run',
        params: {
          ...CLEARED_PARAMS,
          step: location.step,
          path: location.path ?? null,
        },
      };
    default:
      return { state: location.kind, params: { ...CLEARED_PARAMS } };
  }
}

const SETTINGS_SECTIONS: readonly SettingsSection[] = [
  'macs',
  'phrase',
  'keys',
  'account',
  'servers',
  'groups',
  'about',
];

const FIRST_RUN_STATE_NAMES = [
  'boot',
  'who',
  'local',
  'address',
  'no-address',
  'checked',
  'compare',
  'error',
  'account',
  'existing',
  'protect',
  'phrase',
  'waiting',
  'added',
  'create-group',
  'done',
  'local-done',
  'checklist-invited',
  'checklist-own',
] as const;

/**
 * Read a location out of a query string.
 *
 * `null` when the state names something that is not a location — an unknown
 * name, or one of the mock's sheet states. The caller decides the fallback;
 * this does not guess.
 */
export function decodeLocation(search: string): Location | null {
  const params = new URLSearchParams(search);
  const state = params.get('state');
  if (!state) return null;
  if (state === 'store') {
    const ref = params.get('store');
    return ref ? { kind: 'store', ref } : null;
  }
  if (state === 'group-settings') {
    const ref = params.get('store');
    const tab = params.get('tab');
    const resolvedTab =
      tab && (['people', 'settings'] as const).includes(tab as GroupSettingsTab)
        ? (tab as GroupSettingsTab)
        : undefined;
    return ref
      ? {
          kind: 'group-settings',
          ref,
          ...(resolvedTab ? { tab: resolvedTab } : {}),
        }
      : null;
  }
  if (state === 'settings') {
    const section = params.get('section');
    // `account=` was the alias-keyed predecessor of `store=`. It is not read:
    // an alias is ambiguous across profiles, and this project ships nothing to
    // be compatible with.
    const store = params.get('store') ?? undefined;
    // `agent` used to be its own pane; that content now lives on About.
    const resolved = section === 'agent' ? 'about' : section;
    // `profile` is only the Servers section's; anywhere else it is stale.
    const profile =
      resolved === 'servers' ? (params.get('profile') ?? undefined) : undefined;
    return resolved &&
      (SETTINGS_SECTIONS as readonly string[]).includes(resolved)
      ? {
          kind: 'settings',
          section: resolved as SettingsSection,
          ...(store ? { store } : {}),
          ...(profile ? { profile } : {}),
        }
      : { kind: 'settings', ...(store ? { store } : {}) };
  }
  if (state === 'servers') {
    const profile = params.get('profile') ?? undefined;
    return {
      kind: 'settings',
      section: 'servers',
      ...(profile ? { profile } : {}),
    };
  }
  if (state === 'first-run') {
    const path = params.get('path');
    return {
      kind: 'first-run',
      step: params.get('step') ?? 'who',
      ...(path === 'invited' || path === 'own' ? { path } : {}),
    };
  }
  if ((FIRST_RUN_STATE_NAMES as readonly string[]).includes(state)) {
    const path = params.get('path');
    const fixedPath: FirstRunPath | undefined =
      state === 'waiting' || state === 'added' || state === 'checklist-invited'
        ? 'invited'
        : state === 'local' ||
            state === 'local-done' ||
            state === 'create-group' ||
            state === 'done' ||
            state === 'checklist-own'
          ? 'own'
          : undefined;
    return {
      kind: 'first-run',
      step: state,
      ...(fixedPath
        ? { path: fixedPath }
        : path === 'invited' || path === 'own'
          ? { path }
          : {}),
    };
  }
  const alias = STATE_ALIASES[state];
  if (alias?.kind === 'group-settings') {
    const tab = params.get('tab');
    const resolvedTab =
      tab && (['people', 'settings'] as const).includes(tab as GroupSettingsTab)
        ? (tab as GroupSettingsTab)
        : alias.tab;
    return { ...alias, ...(resolvedTab ? { tab: resolvedTab } : {}) };
  }
  if (alias?.kind === 'settings') {
    const store = params.get('store') ?? alias.store;
    const profile =
      alias.section === 'servers'
        ? (params.get('profile') ?? alias.profile)
        : undefined;
    return {
      ...alias,
      ...(store ? { store } : {}),
      ...(profile ? { profile } : {}),
    };
  }
  return alias ?? null;
}

/** The href a location deep-links to, given the address the page is at. */
export function locationHref(href: string, location: Location): string {
  const { state, params } = encodeLocation(location);
  return setUrl(href, state, params);
}

/* ---------------------------------------------------------------- scenes -- */

/**
 * Everything a deep link fixes — the place, and the things that are not a
 * place.
 *
 * `01-vault.html` has one flat `?state=` per scene, and several of its names
 * are *not* locations: `grid` is a view preference, `show` is a selection,
 * `lease` is a **world**, not a place in it. `decodeLocation` answers `null`
 * for all of them on purpose (`tests/location.test.ts` pins that), so the
 * scene layer interprets the remaining state encoded by the name. The lease is
 * carried separately because it is a property of the world the shell is
 * handed: the shell applies it with `applyLease`, it does not store it.
 */
export interface Scene {
  location: Location;
  selection: Selection;
  view: ViewMode;
  kind: KindFilter;
  sort: SortKey;
  lease: LeaseState;
  /** One-shot Show intent carried only by the named acceptance scene. */
  reveal: boolean;
  /** Fixture review selection resolved from live model facts, never a baked path. */
  demo: 'password' | 'resource' | 'file' | 'link' | 'group' | null;
}

/** The scene a `?state=` name means, beyond the location it decodes to. */
const SCENE_ALIASES: Readonly<Record<string, Partial<Scene>>> = {
  password: {
    demo: 'password',
  },
  show: {
    demo: 'password',
    reveal: true,
  },
  resource: {
    demo: 'resource',
  },
  file: {
    demo: 'file',
  },
  link: {
    demo: 'link',
  },
  group: {
    demo: 'group',
  },
  conflict: {
    demo: 'password',
  },
  store: {
    location: { kind: 'store', ref: 'team:eng' },
  },
  items: {
    location: { kind: 'store', ref: 'team:eng' },
  },
  'rekey-menu': {
    location: { kind: 'group-settings', ref: 'team:eng', tab: 'settings' },
  },
  manage: {
    location: { kind: 'group-settings', ref: 'team:household', tab: 'people' },
  },
  'join-invite': { location: { kind: 'settings', section: 'groups' } },
  'party-remove': {
    location: { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
  },
  'groups-lease': {
    location: { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
    lease: 'lapsed',
  },
  'groups-inactive': { location: { kind: 'store', ref: 'team:homelab' } },
  'group-new-text': { location: { kind: 'store', ref: 'team:eng' } },
  'group-new-link': { location: { kind: 'store', ref: 'team:eng' } },
  'group-new-file': { location: { kind: 'store', ref: 'team:eng' } },
  // `grid` is `all` seen as cards.
  grid: { view: 'grid' },
  // Lapsed-lease fixture, with the lapsed store selected.
  lease: { location: { kind: 'store', ref: 'acct:work' }, lease: 'lapsed' },
  // The group whose summary reports inactive.
  inactive: { location: { kind: 'store', ref: 'team:homelab' } },
  // Alerts with the lapsed-lease fixture that populates it.
  alerts: { lease: 'lapsed' },
};

const VIEWS: readonly ViewMode[] = ['list', 'grid'];
const KIND_FILTERS: readonly KindFilter[] = [
  'All',
  'Password',
  'Resource',
  'File',
  'Link',
];
const SORTS: readonly SortKey[] = ['name', 'kind', 'group', 'version'];

function oneOf<T extends string>(
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

export const INITIAL_SCENE: Scene = {
  location: { kind: 'all' },
  selection: null,
  view: 'list',
  kind: 'All',
  sort: 'name',
  lease: 'fresh',
  reveal: false,
  demo: null,
};

/**
 * Read a whole scene out of a query string.
 *
 * The named alias comes first, then explicit parameters, so
 * `?state=grid&view=list` means what it says and a reload of an encoded
 * address lands exactly where it left.
 */
export function decodeScene(search: string): Scene {
  const params = new URLSearchParams(search);
  const name = params.get('state') ?? '';
  const alias = SCENE_ALIASES[name] ?? {};
  const location =
    decodeLocation(search) ?? alias.location ?? INITIAL_SCENE.location;
  return {
    location,
    selection: decodeSelection(params.get('sel')) ?? alias.selection ?? null,
    view: oneOf(VIEWS, params.get('view')) ?? alias.view ?? INITIAL_SCENE.view,
    kind:
      oneOf(KIND_FILTERS, params.get('kind')) ??
      alias.kind ??
      INITIAL_SCENE.kind,
    sort: oneOf(SORTS, params.get('sort')) ?? alias.sort ?? INITIAL_SCENE.sort,
    lease:
      oneOf(['fresh', 'lapsed'] as const, params.get('lease')) ??
      alias.lease ??
      INITIAL_SCENE.lease,
    reveal: alias.reveal ?? false,
    demo: alias.demo ?? null,
  };
}

/**
 * The address a scene deep-links to.
 *
 * Everything at its default is left out, so an ordinary `?state=all` stays
 * `?state=all` and only what a person actually changed shows up.
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
    lease: scene.lease === INITIAL_SCENE.lease ? null : scene.lease,
  });
}

/** The scene a navigation state and a lease world make. */
export function sceneOf(state: LocationState, lease: LeaseState): Scene {
  return {
    location: state.location,
    selection: state.selection,
    view: state.view,
    kind: state.kind,
    sort: state.sort,
    lease,
    reveal: false,
    demo: null,
  };
}

/* ----------------------------------------------------------------- store -- */

/**
 * The shell's navigation store.
 *
 * Modelled on `ui/src/ui-store.ts`: one mutable field, a revision counter and
 * a listener set, so React's external-store contract is satisfied without a
 * reducer in a context. It is deliberately not shared with AKA — the state
 * shape is FOKS's, only the pattern is borrowed.
 */
export class LocationStore {
  private current: LocationState;
  private readonly listeners = new Set<() => void>();

  constructor(initial: LocationState = INITIAL_STATE) {
    this.current = initial;
  }

  readonly getSnapshot = (): LocationState => this.current;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /** Apply an action. Publishes only when the state actually changed. */
  dispatch(action: LocationAction): LocationState {
    const next = transition(this.current, action);
    if (next === this.current) return next;
    this.current = next;
    for (const listener of this.listeners) listener();
    return next;
  }

  navigate(location: Location): void {
    this.dispatch({ type: 'navigate', location });
  }

  select(selection: Selection): void {
    this.dispatch({ type: 'select', selection });
  }

  search(query: string): void {
    this.dispatch({ type: 'search', query });
  }

  setView(view: ViewMode): void {
    this.dispatch({ type: 'view', view });
  }

  setDetails(open: boolean): void {
    this.dispatch({ type: 'details', open });
  }

  setKind(kind: KindFilter): void {
    this.dispatch({ type: 'kind', kind });
  }

  setSort(sort: SortKey): void {
    this.dispatch({ type: 'sort', sort });
  }
}

/** A store standing at a scene — how a deep link becomes navigation state. */
export function storeAtScene(scene: Scene): LocationStore {
  return new LocationStore({
    ...INITIAL_STATE,
    location: scene.location,
    selection: scene.selection,
    details: scene.selection !== null,
    view: scene.view,
    kind: scene.kind,
    sort: scene.sort,
  });
}

/** Subscribe a component to the navigation state. */
export function useLocationState(store: LocationStore): LocationState {
  return useSyncExternalStore(
    store.subscribe,
    store.getSnapshot,
    store.getSnapshot,
  );
}
