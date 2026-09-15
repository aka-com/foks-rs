/**
 * Navigation state management and URL routing.
 *
 * Provides a subscription-based external store managing application location,
 * view options, selection, and query state without direct DOM dependencies.
 * Stores and accounts are identified by canonical StoreRef identifiers.
 */

import { useSyncExternalStore } from 'react';
import type { AccountStore, LeaseState, Store, StoreRef } from './model/types';

/* ------------------------------------------------------------- location -- */

/**
 * Which section of the Settings page an address points at. Settings is one
 * scrolling page now, so a section is where the page opens, not a pane that
 * hides the rest. `credentials` is the Account section: the passphrase and
 * card credentials People and Devices send the reader here for.
 */
export type SettingsSection =
  'servers' | 'about' | 'credentials' | 'notifications';

/** Which pane of the Devices tab is open. */
export type DevicesSection = 'macs' | 'keys';

/**
 * Which tab of a group's page an address points at. `people` is the Members
 * roster, `channels` the group's chat channels, `files` the group's vault
 * view, and `settings` the group's settings.
 */
export type GroupSettingsTab = 'people' | 'channels' | 'files' | 'settings';

/** The tabs in the order the group page's strip draws them. */
export const GROUP_SETTINGS_TABS: readonly GroupSettingsTab[] = [
  'people',
  'channels',
  'files',
  'settings',
];

/** A step in the first-run state machine. */
export type FirstRunStep = string;
export type FirstRunPath = 'invited' | 'own';

/** The rail tab a location belongs to. First run belongs to none. */
export type RailTab =
  'people' | 'chat' | 'files' | 'teams' | 'devices' | 'settings';

export type Location =
  | { kind: 'all' }
  | { kind: 'store'; ref: StoreRef }
  | { kind: 'group-settings'; ref: StoreRef; tab?: GroupSettingsTab }
  /** People: the attention list, then the accounts on this Mac. */
  | { kind: 'people'; store?: StoreRef }
  /**
   * Chat. `ref` is the team whose inbox is mounted and `channel` the open
   * conversation; with no `ref` the tab picks the first team that has chat.
   */
  | { kind: 'chat'; ref?: StoreRef; channel?: string }
  /** The Files roots page: All items, then vaults, groups and shares. */
  | { kind: 'files' }
  /**
   * The Teams list, then the group settings the Groups pane carries. `store`
   * names the account the create and discovery rows act as.
   */
  | { kind: 'teams'; store?: StoreRef }
  /**
   * Devices. `device` is one row's own page: the key id of a Mac or a paper
   * key, or `yubi:<alias>` for a security-key enrollment, which the agent
   * names by alias and not by an id.
   */
  | {
      kind: 'devices';
      section?: DevicesSection;
      store?: StoreRef;
      device?: string;
    }
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

/** Resolve the account through which a page's object is accessed. */
export function accountAtLocation(
  stores: readonly Store[],
  location: Location,
  fallback?: StoreRef,
): AccountStore | undefined {
  const accounts = stores.filter(
    (store): store is AccountStore => store.kind === 'account',
  );
  if ('store' in location && location.store)
    return accounts.find((account) => account.id === location.store);
  if ('ref' in location && location.ref) {
    const target = stores.find((store) => store.id === location.ref);
    return (
      target &&
      accounts.find(
        (account) =>
          account.server === target.server &&
          account.account === target.account,
      )
    );
  }
  if (location.kind === 'settings' && location.profile)
    return accounts.find((account) => account.server === location.profile);
  return accounts.find((account) => account.id === fallback) ?? accounts[0];
}

/** The rail tab that owns a location, or `null` for first run. */
export function railTabOf(location: Location): RailTab | null {
  switch (location.kind) {
    case 'people':
      return 'people';
    case 'chat':
      return 'chat';
    case 'files':
    case 'all':
    case 'store':
      return 'files';
    case 'teams':
    case 'group-settings':
      return 'teams';
    case 'devices':
      return 'devices';
    case 'settings':
      return 'settings';
    case 'first-run':
      return null;
  }
}

/** The chat location the Chat tab last opened, for the rail's Chat tab. */
let openedChat: Extract<Location, { kind: 'chat' }> | null = null;

/**
 * Where the rail's Chat tab goes: the team and channel the tab last had open,
 * so returning to Chat does not re-run the first-team fallback. The Chat tab
 * itself is the only writer, and it forgets a team that stopped having chat.
 */
export function chatTabLocation(): Location {
  return openedChat ?? { kind: 'chat' };
}

export function rememberChatLocation(
  location: Extract<Location, { kind: 'chat' }> | null,
): void {
  openedChat = location?.ref ? location : null;
}

/** The remembered team, for the writer deciding whether the memory still holds. */
export function rememberedChatRef(): StoreRef | undefined {
  return openedChat?.ref;
}

/** The item the details panel is showing, or nothing. */
export type Selection = { store: StoreRef; path: string } | null;

/** Layout mode for displaying vault items. */
export type ViewMode = 'list' | 'grid' | 'folders';

/** Kind filter selection, where 'All' disables kind filtering. */
export type KindFilter = 'All' | 'Password' | 'Resource' | 'File' | 'Link';

/** The sort menu's choice. */
export type SortKey = 'name' | 'kind' | 'group';

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
  /** Selected folder in folder view: `store|/path` on All, `/path` in a store. */
  folder: string;
  /** Comma-free tree node keys whose children are folded. */
  closedFolders: readonly string[];
}

export const INITIAL_STATE: LocationState = {
  location: { kind: 'all' },
  selection: null,
  query: '',
  // Item pages open in the folder browser; the toolbar's list/grid/folders
  // toggle still chooses, and the choice survives the next navigation.
  view: 'folders',
  details: false,
  kind: 'All',
  sort: 'name',
  folder: '',
  closedFolders: [],
};

/* ------------------------------------------------------------ transition -- */

/**
 * How a navigation is recorded. A replacement is a move inside the place the
 * reader is already in — a tab of the page they are on — rather than an
 * arrival somewhere new, so it keeps what that place was showing instead of
 * standing a fresh navigation in its stead.
 */
export interface NavigateOptions {
  replace?: boolean;
}

export type LocationAction =
  | { type: 'navigate'; location: Location; replace?: boolean }
  | { type: 'select'; selection: Selection }
  | { type: 'search'; query: string }
  | { type: 'view'; view: ViewMode }
  | { type: 'details'; open: boolean }
  | { type: 'kind'; kind: KindFilter }
  | { type: 'sort'; sort: SortKey }
  | { type: 'folder'; folder: string }
  | { type: 'toggle-folder'; folder: string };

/** Returns whether two locations identify the same navigation target. */
export function sameLocation(a: Location, b: Location): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === 'store' && b.kind === 'store') return a.ref === b.ref;
  if (a.kind === 'chat' && b.kind === 'chat')
    return a.ref === b.ref && a.channel === b.channel;
  if (a.kind === 'group-settings' && b.kind === 'group-settings')
    return a.ref === b.ref && a.tab === b.tab;
  if (a.kind === 'people' && b.kind === 'people') return a.store === b.store;
  if (a.kind === 'teams' && b.kind === 'teams') return a.store === b.store;
  if (a.kind === 'devices' && b.kind === 'devices')
    return (
      a.section === b.section && a.store === b.store && a.device === b.device
    );
  if (a.kind === 'settings' && b.kind === 'settings')
    return (
      a.section === b.section && a.store === b.store && a.profile === b.profile
    );
  if (a.kind === 'first-run' && b.kind === 'first-run')
    return a.step === b.step && a.path === b.path;
  return true;
}

/**
 * Pure transition function for location state actions.
 *
 * Navigating to a new location clears the current item selection while
 * preserving active search queries. Re-navigating to the current location
 * preserves the active selection.
 */
export function transition(
  state: LocationState,
  action: LocationAction,
): LocationState {
  switch (action.type) {
    case 'navigate':
      if (sameLocation(state.location, action.location)) return state;
      // A replacement moves within the place the reader is in, so the item
      // they had selected and the folder they had open are still theirs.
      if (action.replace) return { ...state, location: action.location };
      return {
        ...state,
        location: action.location,
        selection: null,
        folder: '',
        closedFolders: [],
        query:
          action.location.kind === 'chat' || state.location.kind === 'chat'
            ? ''
            : state.query,
      };
    case 'select':
      // Selecting an item automatically opens the details panel; deselecting keeps the panel open.
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
    case 'folder':
      return state.folder === action.folder && state.selection === null
        ? state
        : { ...state, folder: action.folder, selection: null };
    case 'toggle-folder': {
      const closed = new Set(state.closedFolders);
      if (closed.has(action.folder)) closed.delete(action.folder);
      else closed.add(action.folder);
      return { ...state, closedFolders: [...closed].sort() };
    }
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
  // The Alerts page is the top of People now.
  alerts: { kind: 'people' },
  join: { kind: 'teams' },
  groups: { kind: 'teams' },
  // `people` is the People tab, decoded above; the mock's People *tab of a
  // group* is `group-people`, alongside `party` and `federation`.
  'group-people': { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
  // Engineering's server offers no chat, so its Channels tab is the reason
  // rather than the tab: the scene opens on the group that has channels.
  'group-channels': {
    kind: 'group-settings',
    ref: 'team:household',
    tab: 'channels',
  },
  'group-files': { kind: 'group-settings', ref: 'team:eng', tab: 'files' },
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
  create: { kind: 'teams' },
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
  // Recovery devices and security keys are the Devices tab; the accounts pane
  // is the foot of People.
  'settings-macs': { kind: 'devices', section: 'macs' },
  'settings-macs-work': {
    kind: 'devices',
    section: 'macs',
    store: 'acct:work',
  },
  'settings-phrase': { kind: 'devices', section: 'macs' },
  'settings-keys': { kind: 'devices', section: 'keys' },
  'settings-enrol': { kind: 'devices', section: 'keys' },
  'settings-account': { kind: 'people' },
  'settings-agent': { kind: 'settings', section: 'about' },
  'settings-about': { kind: 'settings', section: 'about' },
};

/**
 * Baseline dictionary of route query parameters reset during transitions to
 * prevent parameter leakage across locations.
 */
const CLEARED_PARAMS: Readonly<Record<string, string | null>> = {
  store: null,
  section: null,
  profile: null,
  step: null,
  path: null,
  tab: null,
  account: null,
  channel: null,
  device: null,
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
    case 'chat':
      return {
        state: 'chat',
        params: {
          ...CLEARED_PARAMS,
          store: location.ref ?? null,
          channel: location.channel ?? null,
        },
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
    case 'people':
      return {
        state: 'people',
        params: { ...CLEARED_PARAMS, store: location.store ?? null },
      };
    case 'teams':
      return {
        state: 'teams',
        params: { ...CLEARED_PARAMS, store: location.store ?? null },
      };
    case 'devices':
      return {
        state: 'devices',
        params: {
          ...CLEARED_PARAMS,
          store: location.store ?? null,
          section: location.section ?? null,
          device: location.device ?? null,
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
  'notifications',
  'servers',
  'about',
  'credentials',
];
const DEVICES_SECTIONS: readonly DevicesSection[] = ['macs', 'keys'];

/**
 * The tab a `section=` value written before the rail belongs to now. Recovery
 * devices, the backup phrase and security keys are Devices; Groups is Teams;
 * Accounts is People.
 */
const RETIRED_SETTINGS_SECTIONS: Readonly<
  Record<
    string,
    | { kind: 'devices'; section: DevicesSection }
    | { kind: 'teams' }
    | { kind: 'people' }
  >
> = {
  macs: { kind: 'devices', section: 'macs' },
  phrase: { kind: 'devices', section: 'macs' },
  keys: { kind: 'devices', section: 'keys' },
  groups: { kind: 'teams' },
  account: { kind: 'people' },
};

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
  'local-done',
  'identity-pending',
  'operation-pending',
  'checklist-invited',
  'checklist-own',
] as const;

/**
 * Decodes a Location object from a URL query string, returning `null` if the
 * state parameter is unrecognized or does not represent a standalone location.
 */
export function decodeLocation(search: string): Location | null {
  const params = new URLSearchParams(search);
  const state = params.get('state');
  if (!state) return null;
  if (state === 'store') {
    const ref = params.get('store');
    return ref ? { kind: 'store', ref } : null;
  }
  // `team-chat` was the chat location before the team column; it is the same
  // place, with the team named by `store`.
  if (state === 'chat' || state === 'team-chat') {
    const ref = params.get('store') ?? undefined;
    const channel = params.get('channel');
    if (channel !== null && !/^[0-9a-f]{32}$/.test(channel)) return null;
    if (state === 'team-chat' && !ref) return null;
    return {
      kind: 'chat',
      // A channel belongs to the team that names it: with no team the tab
      // resolves one, and that team's channels are not this identifier.
      ...(ref ? { ref, ...(channel ? { channel } : {}) } : {}),
    };
  }
  if (state === 'group-settings') {
    const ref = params.get('store');
    const tab = params.get('tab');
    const resolvedTab =
      tab && GROUP_SETTINGS_TABS.includes(tab as GroupSettingsTab)
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
  if (state === 'people') {
    const store = params.get('store') ?? undefined;
    return { kind: 'people', ...(store ? { store } : {}) };
  }
  if (state === 'files') return { kind: 'files' };
  if (state === 'teams') {
    const store = params.get('store') ?? undefined;
    return { kind: 'teams', ...(store ? { store } : {}) };
  }
  if (state === 'devices') {
    const section = params.get('section');
    const store = params.get('store') ?? undefined;
    const device = params.get('device') ?? undefined;
    const resolved =
      section && (DEVICES_SECTIONS as readonly string[]).includes(section)
        ? (section as DevicesSection)
        : section === 'phrase'
          ? 'macs'
          : undefined;
    return {
      kind: 'devices',
      ...(resolved ? { section: resolved } : {}),
      ...(store ? { store } : {}),
      ...(device ? { device } : {}),
    };
  }
  if (state === 'settings') {
    const section = params.get('section');
    // `account=` was the alias-keyed predecessor of `store=`. It is not read:
    // an alias is ambiguous across profiles, and this project ships nothing to
    // be compatible with.
    const store = params.get('store') ?? undefined;
    // Sections that moved to a tab of their own keep working as deep links.
    const moved = section ? RETIRED_SETTINGS_SECTIONS[section] : undefined;
    // Every tab a section moved to acts on one account, so the account the
    // address named comes with it.
    if (moved) return { ...moved, ...(store ? { store } : {}) };
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
      tab && GROUP_SETTINGS_TABS.includes(tab as GroupSettingsTab)
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
  if (
    alias?.kind === 'devices' ||
    alias?.kind === 'people' ||
    alias?.kind === 'teams'
  ) {
    const store = params.get('store') ?? alias.store;
    // A device's own page is addressable under the old Devices names too.
    const device = alias.kind === 'devices' ? params.get('device') : null;
    return {
      ...alias,
      ...(store ? { store } : {}),
      ...(device ? { device } : {}),
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
 * `lease` is a **snapshot**, not a place in it. `decodeLocation` answers `null`
 * for all of them on purpose (`tests/location.test.ts` pins that), so the
 * scene layer interprets the remaining state encoded by the name. The lease is
 * carried separately because it is a property of the snapshot the shell is
 * handed: the shell applies it with `applyLease`, it does not store it.
 */
export interface Scene {
  location: Location;
  selection: Selection;
  view: ViewMode;
  kind: KindFilter;
  sort: SortKey;
  folder: string;
  closedFolders: readonly string[];
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
  folders: { view: 'folders' },
  // Lapsed-lease fixture, with the lapsed store selected.
  lease: { location: { kind: 'store', ref: 'acct:work' }, lease: 'lapsed' },
  // The group whose summary reports inactive.
  inactive: { location: { kind: 'store', ref: 'team:homelab' } },
  // People's attention list, with the lapsed-lease fixture that populates it.
  alerts: { lease: 'lapsed' },
};

const VIEWS: readonly ViewMode[] = ['list', 'grid', 'folders'];
const KIND_FILTERS: readonly KindFilter[] = [
  'All',
  'Password',
  'Resource',
  'File',
  'Link',
];
const SORTS: readonly SortKey[] = ['name', 'kind', 'group'];

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
  view: 'folders',
  kind: 'All',
  sort: 'name',
  folder: '',
  closedFolders: [],
  lease: 'fresh',
  reveal: false,
  demo: null,
};

/**
 * Decodes a complete Scene configuration from a URL search query, resolving
 * named aliases first and applying explicit parameters as overrides.
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
    folder: params.get('folder') ?? alias.folder ?? INITIAL_SCENE.folder,
    closedFolders: (params.get('closed') ?? '').split(',').filter(Boolean),
    lease:
      oneOf(['fresh', 'lapsed'] as const, params.get('lease')) ??
      alias.lease ??
      INITIAL_SCENE.lease,
    reveal: alias.reveal ?? false,
    demo: alias.demo ?? null,
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

/* ----------------------------------------------------------------- store -- */

/**
 * External navigation store satisfying React's `useSyncExternalStore` contract.
 */
export class LocationStore {
  private current: LocationState;
  private stores: readonly Store[] = [];
  private actingAccount?: StoreRef;
  private hasInventory = false;
  private readonly tabs = new Map<RailTab, LocationState>();

  clearTabMemory(): void {
    this.tabs.clear();
  }

  /** Rail tabs resume their last page; explicit home links still open roots. */
  navigateTab(tab: RailTab): void {
    if (railTabOf(this.current.location) === tab) return;
    const defaults: Record<RailTab, Location> = {
      people: { kind: 'people' },
      chat: chatTabLocation(),
      files: { kind: 'files' },
      teams: { kind: 'teams' },
      devices: { kind: 'devices' },
      settings: { kind: 'settings' },
    };
    let saved = this.tabs.get(tab);
    let location = saved?.location ?? defaults[tab];
    const target = 'ref' in location ? location.ref : undefined;
    if (
      this.hasInventory &&
      target &&
      !this.stores.some((store) => store.id === target)
    ) {
      saved = undefined;
      location = tab === 'chat' ? { kind: 'chat' } : defaults[tab];
    }
    if (
      location.kind === 'people' ||
      location.kind === 'teams' ||
      location.kind === 'devices' ||
      location.kind === 'settings'
    ) {
      const account = this.getAccount();
      const changed = location.store !== account;
      location = { ...location, store: account };
      if (changed && location.kind === 'devices') delete location.device;
      if (changed && location.kind === 'settings') delete location.profile;
    }
    location = this.accountLocation(location);
    const next = transition(this.current, { type: 'navigate', location });
    this.publish(
      saved
        ? { ...saved, location, selection: null, details: false }
        : { ...next, query: '', details: false },
    );
  }

  /** Inventory is refreshed by the shell; a removed account is never reused. */
  setAccountStores(stores: readonly Store[]): void {
    this.hasInventory = true;
    this.stores = stores;
    this.actingAccount = accountAtLocation(
      stores,
      this.current.location,
      this.actingAccount,
    )?.id;
  }

  getAccount(): StoreRef | undefined {
    return accountAtLocation(
      this.stores,
      this.current.location,
      this.actingAccount,
    )?.id;
  }

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
    return this.publish(next);
  }

  private publish(next: LocationState): LocationState {
    if (next === this.current) return next;
    const tab = railTabOf(this.current.location);
    if (tab) this.tabs.set(tab, this.current);
    this.current = next;
    for (const listener of this.listeners) listener();
    return next;
  }

  private accountLocation(location: Location): Location {
    const account = accountAtLocation(this.stores, location, this.getAccount());
    if (account) {
      this.actingAccount = account.id;
      if (
        (location.kind === 'people' ||
          location.kind === 'devices' ||
          location.kind === 'settings' ||
          location.kind === 'teams') &&
        !location.store
      )
        location = { ...location, store: account.id };
    }
    return location;
  }

  navigate(location: Location, options: NavigateOptions = {}): void {
    this.dispatch({
      type: 'navigate',
      location: this.accountLocation(location),
      ...(options.replace ? { replace: true } : {}),
    });
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

  setFolder(folder: string): void {
    this.dispatch({ type: 'folder', folder });
  }

  toggleFolder(folder: string): void {
    this.dispatch({ type: 'toggle-folder', folder });
  }
}

/** Initializes a LocationStore populated with state from a decoded Scene. */
export function storeAtScene(scene: Scene): LocationStore {
  return new LocationStore({
    ...INITIAL_STATE,
    location: scene.location,
    selection: scene.selection,
    details: scene.selection !== null,
    view: scene.view,
    kind: scene.kind,
    sort: scene.sort,
    folder: scene.folder,
    closedFolders: scene.closedFolders,
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
