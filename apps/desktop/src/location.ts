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
 * Which page of Settings' sub-navigation an address points at. Each section is
 * a page of its own, with the sub-navigation staying on screen while any one
 * of them is open. `servers` lists the servers this Mac talks to and holds
 * each server's own page, security keys included. `preferences` contains
 * account passphrases and local desktop alert settings. `mac` is This Mac: the
 * application version and lock,
 * the agent, the local FOKS data operations and the Mac-wide reset.
 *
 * Older addresses name sections that folded into these three; `decodeLocation`
 * maps them (`SETTINGS_SECTION_ALIASES`).
 */
export type SettingsSection = 'servers' | 'preferences' | 'mac';

/** The order the sub-navigation lists Settings' pages in. */
export const SETTINGS_SECTIONS: readonly SettingsSection[] = [
  'servers',
  'preferences',
  'mac',
];

/**
 * Former `section=` values that are pages of one of the three sections now.
 * `credentials` (the passphrase rows) and `notifications` are Preferences;
 * `device`, `about` and the older `agent` are This Mac; `security-keys` was a
 * list of links to each server's page, so it is Servers.
 */
export const SETTINGS_SECTION_ALIASES: Readonly<
  Record<string, SettingsSection>
> = {
  credentials: 'preferences',
  notifications: 'preferences',
  device: 'mac',
  about: 'mac',
  agent: 'mac',
  'security-keys': 'servers',
};

/** The page an address with no `section=` opens: the sub-navigation's first. */
export const DEFAULT_SETTINGS_SECTION: SettingsSection = SETTINGS_SECTIONS[0];

/** The words the sub-navigation and the topbar's crumb use for each page. */
export const SETTINGS_SECTION_LABEL: Readonly<Record<SettingsSection, string>> =
  {
    servers: 'Servers',
    preferences: 'Preferences',
    mac: 'This Mac',
  };

/** Which pane of the Devices tab is open. */
export type DevicesSection = 'macs' | 'keys';

/**
 * Which tab of a group's page an address points at. `people` is the Members
 * roster, `channels` the group's chat channels, `files` the group's vault
 * view, `requests` its pending invitations and join requests, and `settings`
 * the group's settings.
 */
export type GroupSettingsTab =
  'people' | 'channels' | 'files' | 'requests' | 'settings';

/** The tabs in the order the group page's strip draws them. */
export const GROUP_SETTINGS_TABS: readonly GroupSettingsTab[] = [
  'people',
  'channels',
  'files',
  'requests',
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
  /** People: the Account tab, one account's profile at a time. */
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

/**
 * The page the topbar's back chevron returns to, or `null` at a tab's root.
 *
 * Only an address that opens something *inside* a tab has a parent: a store
 * page under Files, a group page under Teams, a section or a device page under
 * Settings and Devices. The `store` parameter that People, Teams, Devices and
 * Settings carry names the account the page acts as — every address of those
 * tabs carries one — so it never makes a page below the tab's root. A chat
 * channel returns to its team's inbox, which has no parent.
 */
export function parentLocation(location: Location): Location | null {
  switch (location.kind) {
    // `all` and `files` both open the "All items" leaf, so neither is inside
    // the other.
    case 'all':
      return null;
    case 'store':
      return { kind: 'files' };
    case 'group-settings':
      return { kind: 'teams' };
    case 'chat':
      return location.ref && location.channel
        ? { kind: 'chat', ref: location.ref }
        : null;
    case 'devices':
      if (location.device)
        return {
          kind: 'devices',
          ...(location.section ? { section: location.section } : {}),
          ...(location.store ? { store: location.store } : {}),
        };
      if (location.section)
        return {
          kind: 'devices',
          ...(location.store ? { store: location.store } : {}),
        };
      return null;
    case 'settings':
      if (location.section || location.profile)
        return {
          kind: 'settings',
          ...(location.store ? { store: location.store } : {}),
        };
      return null;
    default:
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

/**
 * Layout mode for displaying vault items. The folder tree is permanent
 * navigation now, not a mode, so `list` is the only rendering left; the type
 * stays so a saved `view=` link still decodes to something.
 */
export type ViewMode = 'list';

/** Kind filter selection, where 'All' disables kind filtering. */
export type KindFilter = 'All' | 'Password' | 'Document';

/** The sort menu's choice. */
export type SortKey = 'name' | 'kind' | 'group';

export interface LocationState {
  /** Front-end sheet drafts only; never part of Location or URL encoding. */
  sheet?: Readonly<Record<string, unknown>>;
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
  view: 'list',
  details: false,
  kind: 'All',
  sort: 'name',
  folder: '',
  closedFolders: [],
};

/* ------------------------------------------------------------ transition -- */

/**
 * Options common to the operations the guards see. `force` skips them. It
 * marks a move the shell makes rather than one the reader asked for — a
 * redirect out of a place that is no longer readable, a replacement that
 * canonicalizes the address of the page already open, or the navigation a
 * confirmed prompt was raised for — which no screen may refuse.
 */
export interface GuardedOptions {
  force?: boolean;
}

/**
 * How a navigation is recorded. A replacement is a move inside the place the
 * reader is already in — a tab of the page they are on — rather than an
 * arrival somewhere new, so it keeps what that place was showing instead of
 * standing a fresh navigation in its stead.
 */
export interface NavigateOptions extends GuardedOptions {
  replace?: boolean;
}

/* --------------------------------------------------------------- guards -- */

/**
 * What a guard is asked about: the address a navigation is headed for, or the
 * item a selection is about to open. Both unmount whatever the current page
 * has on screen, so both are offered.
 */
export type NavigationIntent =
  | { kind: 'navigate'; location: Location; tab?: boolean }
  | { kind: 'select'; selection: Selection };

/**
 * Navigation guard result. `null` allows navigation, `prompt` requests user
 * confirmation, and `refuse` blocks navigation with an explanatory reason.
 * `onConfirm` runs once before confirmed navigation.
 */
export type GuardVerdict =
  | null
  | {
      verdict: 'prompt';
      title: string;
      body: string;
      confirm: string;
      onConfirm?: () => void;
    }
  | { verdict: 'refuse'; reason: string };

/** A screen's answer for one intent. Registered with `registerGuard`. */
export type NavigationGuard = (intent: NavigationIntent) => GuardVerdict;

/**
 * Displays a confirmation dialog for a `prompt` verdict. Resolves true to
 * continue navigation and false to cancel. Without an installed prompter,
 * prompt verdicts allow navigation.
 */
export interface NavigationPrompter {
  (verdict: Extract<GuardVerdict, { verdict: 'prompt' }>): Promise<boolean>;
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
        ...(state.sheet ? { sheet: undefined } : {}),
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
  // The agent and About content is the This Mac page.
  'settings-agent': { kind: 'settings', section: 'mac' },
  'settings-about': { kind: 'settings', section: 'mac' },
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
    // Sections that folded into one of the three pages open that page.
    const resolved =
      section && SETTINGS_SECTION_ALIASES[section]
        ? SETTINGS_SECTION_ALIASES[section]
        : section;
    // The `profile` parameter applies only to the Servers section. Legacy
    // `security-keys` URLs identified a server row, so they redirect to the
    // root Servers list.
    const profile =
      section === 'servers' ? (params.get('profile') ?? undefined) : undefined;
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
  'group-new-document': { location: { kind: 'store', ref: 'team:eng' } },
  // `grid` and `folders` were view modes before the tree replaced the
  // toggle; both now just mean the one view that is left.
  grid: { view: 'list' },
  folders: { view: 'list' },
  // Lapsed-lease fixture, with the lapsed store selected.
  lease: { location: { kind: 'store', ref: 'acct:work' }, lease: 'lapsed' },
  // The group whose summary reports inactive.
  inactive: { location: { kind: 'store', ref: 'team:homelab' } },
  // People's attention list, with the lapsed-lease fixture that populates it.
  alerts: { lease: 'lapsed' },
};

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
    this.clearSheet();
    this.tabs.clear();
  }

  private readonly sheetRestoration = new Map<symbol, boolean>();
  setSheetRestorable(id: symbol, restorable: boolean | undefined): void {
    if (restorable === undefined) this.sheetRestoration.delete(id);
    else this.sheetRestoration.set(id, restorable);
  }

  setSheetField(key: string, value: unknown): void {
    if (Object.is(this.current.sheet?.[key], value)) return;
    this.publish({
      ...this.current,
      sheet: { ...this.current.sheet, [key]: value },
    });
  }

  clearSheet(): void {
    if (this.current.sheet) this.publish({ ...this.current, sheet: undefined });
  }

  /**
   * Where a rail tab goes, and the state it resumes there, without moving.
   * `null` when the tab already owns the current page. Pure: the acting
   * account is recorded by the navigation itself, not by working out its
   * destination, so a guard can be asked before anything changes.
   */
  private tabTarget(
    tab: RailTab,
  ): { location: Location; saved?: LocationState } | null {
    if (railTabOf(this.current.location) === tab) return null;
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
      if (changed && saved) saved = { ...saved, sheet: undefined };
      location = { ...location, store: account };
      if (changed && location.kind === 'devices') delete location.device;
      if (changed && location.kind === 'settings') delete location.profile;
    }
    return {
      location: this.resolvedLocation(location),
      ...(saved ? { saved } : {}),
    };
  }

  /** Rail tabs resume their last page; explicit home links still open roots. */
  navigateTab(tab: RailTab, options: GuardedOptions = {}): void {
    const target = this.tabTarget(tab);
    if (!target) return;
    const apply = (): void => {
      const location = this.accountLocation(target.location);
      const next = transition(this.current, { type: 'navigate', location });
      this.publish(
        target.saved
          ? { ...target.saved, location, selection: null, details: false }
          : { ...next, query: '', details: false },
      );
    };
    this.guarded(
      { kind: 'navigate', location: target.location, tab: true },
      apply,
      options,
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

  /**
   * The address a navigation lands on, with the account it is read through
   * filled in. Pure — `accountLocation` is the same resolution, and also
   * records that account as the acting one.
   */
  private resolvedLocation(location: Location): Location {
    const account = accountAtLocation(this.stores, location, this.getAccount());
    if (
      account &&
      (location.kind === 'people' ||
        location.kind === 'devices' ||
        location.kind === 'settings' ||
        location.kind === 'teams') &&
      !location.store
    )
      return { ...location, store: account.id };
    return location;
  }

  private accountLocation(location: Location): Location {
    const account = accountAtLocation(this.stores, location, this.getAccount());
    if (account) this.actingAccount = account.id;
    return this.resolvedLocation(location);
  }

  /* ------------------------------------------------------------- guards -- */

  private readonly guards: NavigationGuard[] = [];
  private prompter: NavigationPrompter | null = null;
  private refusalHandler: ((reason: string) => void) | null = null;
  /**
   * The prompt on screen, if any. Every guarded operation replaces it: the
   * newer intent is the one the reader is asking for, so the older prompt's
   * answer, whenever it arrives, is discarded.
   */
  private pendingPrompt: object | null = null;

  /**
   * Registers a guard, returning the function that takes it off again. Guards
   * are asked in registration order and the first non-null verdict decides.
   */
  registerGuard(guard: NavigationGuard): () => void {
    this.guards.push(guard);
    return () => {
      const at = this.guards.indexOf(guard);
      if (at >= 0) this.guards.splice(at, 1);
    };
  }

  /**
   * What the guards say about an intent, without acting on it. A caller that
   * must stay inert rather than raise a prompt — the trackpad's back swipe —
   * asks this first.
   */
  navigationVerdict(intent: NavigationIntent): GuardVerdict {
    // A refusal terminates validation immediately. Prompts remain provisional
    // until every guard has been evaluated because a subsequent guard may
    // still reject navigation.
    let prompt: GuardVerdict = null;
    for (const guard of [...this.guards]) {
      const verdict = guard(intent);
      if (verdict?.verdict === 'refuse') return verdict;
      if (verdict && !prompt) prompt = verdict;
    }
    return prompt;
  }

  /** The dialog a `prompt` verdict is put to the reader through. */
  setPrompter(prompter: NavigationPrompter | null): void {
    this.prompter = prompter;
  }

  /** Where a `refuse` verdict's reason is shown. */
  setRefusalHandler(handler: ((reason: string) => void) | null): void {
    this.refusalHandler = handler;
  }

  /**
   * Runs the guards over `intent` and applies `apply` once they are satisfied:
   * now for an allowed or forced move, and when the prompt is confirmed for a
   * prompted one. The caller's signature stays synchronous either way.
   */
  private guarded(
    intent: NavigationIntent,
    applyRequested: () => void,
    options: GuardedOptions,
  ): void {
    const apply = (): void => {
      if ([...this.sheetRestoration.values()].includes(false))
        this.clearSheet();
      applyRequested();
    };
    const verdict = options.force ? null : this.navigationVerdict(intent);
    // A refusal changes nothing, including an open prompt about another move.
    if (verdict?.verdict === 'refuse') {
      this.refusalHandler?.(verdict.reason);
      return;
    }
    // Anything else supersedes that prompt: its answer no longer applies.
    this.pendingPrompt = null;
    if (!verdict) {
      apply();
      return;
    }
    const prompter = this.prompter;
    if (!prompter) {
      apply();
      return;
    }
    const token = {};
    this.pendingPrompt = token;
    void prompter(verdict).then(
      (confirmed) => {
        if (this.pendingPrompt !== token) return;
        this.pendingPrompt = null;
        if (!confirmed) return;
        verdict.onConfirm?.();
        this.clearSheet();
        apply();
      },
      () => {
        if (this.pendingPrompt === token) this.pendingPrompt = null;
      },
    );
  }

  navigate(location: Location, options: NavigateOptions = {}): void {
    const apply = (): void => {
      this.dispatch({
        type: 'navigate',
        location: this.accountLocation(location),
        ...(options.replace ? { replace: true } : {}),
      });
    };
    this.guarded(
      { kind: 'navigate', location: this.resolvedLocation(location) },
      apply,
      options,
    );
  }

  /**
   * Opens `location` with `selection` showing on it, as one guarded move. The
   * ⌘K palette's item results go through this: a guard that holds the
   * navigation back must hold the selection with it, or the reader would be
   * left on the page they were on with another page's item in the details
   * panel. The guards see the navigation; the selection arrives with it.
   */
  navigateAndSelect(
    location: Location,
    selection: Selection,
    options: NavigateOptions = {},
  ): void {
    const apply = (): void => {
      this.dispatch({
        type: 'navigate',
        location: this.accountLocation(location),
        ...(options.replace ? { replace: true } : {}),
      });
      this.dispatch({ type: 'select', selection });
    };
    this.guarded(
      { kind: 'navigate', location: this.resolvedLocation(location) },
      apply,
      options,
    );
  }

  select(selection: Selection, options: GuardedOptions = {}): void {
    this.guarded(
      { kind: 'select', selection },
      () => {
        this.dispatch({ type: 'select', selection });
      },
      options,
    );
  }

  search(query: string): void {
    this.dispatch({ type: 'search', query });
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
