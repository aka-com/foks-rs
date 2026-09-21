import type { LeaseState, StoreRef } from '../model/types';

/* ------------------------------------------------------------- location -- */

/**
 * Which page of Settings' sub-navigation an address points at. Each section is
 * a page of its own, with the sub-navigation staying on screen while any one
 * of them is open. `account` is one account's profile, the account the address
 * `store` names: its username, local alias, server, and the counts that link
 * to Devices and Teams. It also lists the servers this Mac talks to and holds
 * each server's own page, security keys included, and manages the selected
 * account's passphrase. `preferences` contains local desktop alert and
 * appearance settings. `mac` is Device: the
 * application version and lock, the agent, the local FOKS data operations and
 * the Mac-wide reset. The section keeps its `mac` id, so older addresses that
 * name it still resolve.
 *
 * Older addresses name sections that folded into these four; `decodeLocation`
 * maps them (`SETTINGS_SECTION_ALIASES`). The former Account tab
 * (`state=people`) decodes to the `account` section.
 */
export type SettingsSection =
  | 'account'
  /** @deprecated Decoded and rendered as Account. */
  | 'servers'
  | 'preferences'
  | 'mac';

/** The order the sub-navigation lists Settings' pages in. */
export const SETTINGS_SECTIONS: readonly SettingsSection[] = [
  'account',
  'preferences',
  'mac',
];

/** The page an address with no `section=` opens: the sub-navigation's first. */
export const DEFAULT_SETTINGS_SECTION: SettingsSection = SETTINGS_SECTIONS[0];

/** The words the sub-navigation and the topbar's crumb use for each page. */
export const SETTINGS_SECTION_LABEL: Readonly<Record<SettingsSection, string>> =
  {
    account: 'Account',
    servers: 'Account',
    preferences: 'Preferences',
    mac: 'Storage',
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

/**
 * The sheet an address asks the Teams page to open when it arrives. Creating
 * a team and pasting an invitation are sheets over the Teams list rather than
 * pages of their own, so this is the only way another screen can send a
 * reader straight into one.
 */
export type TeamsSheetIntent = 'create' | 'join';

/** The intents a `?open=` value may name, for decoding an address. */
export const TEAMS_SHEET_INTENTS: readonly TeamsSheetIntent[] = [
  'create',
  'join',
];

/** A step in the first-run state machine. */
export type FirstRunStep = string;
export type FirstRunPath = 'invited' | 'own';

/** The rail tab a location belongs to. First run belongs to none. */
export type RailTab = 'chat' | 'files' | 'teams' | 'devices' | 'settings';

export type Location =
  | { kind: 'all' }
  | { kind: 'store'; ref: StoreRef }
  | { kind: 'group-settings'; ref: StoreRef; tab?: GroupSettingsTab }
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
  | { kind: 'teams'; store?: StoreRef; open?: TeamsSheetIntent }
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
   * `store` names the account the page acts on, which the Account section
   * shows and the other sections read through. `profile` names the server
   * detail open at the bottom of Account; it means nothing on other sections
   * and is dropped when moving between them.
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
export type SortDirection = 'asc' | 'desc';

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
  sortDirection: SortDirection;
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
  sortDirection: 'asc',
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
  | { type: 'select'; selection: Selection; closeDetails?: boolean }
  | { type: 'search'; query: string }
  | { type: 'view'; view: ViewMode }
  | { type: 'details'; open: boolean }
  | { type: 'kind'; kind: KindFilter }
  | { type: 'sort'; sort: SortKey; direction?: SortDirection }
  | { type: 'folder'; folder: string }
  | { type: 'toggle-folder'; folder: string };

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
  sortDirection: SortDirection;
  folder: string;
  closedFolders: readonly string[];
  lease: LeaseState;
  /** One-shot Show intent carried only by the named acceptance scene. */
  reveal: boolean;
  /** Fixture review selection resolved from live model facts, never a baked path. */
  demo: 'password' | 'resource' | 'file' | 'link' | 'group' | null;
}

export const INITIAL_SCENE: Scene = {
  location: { kind: 'all' },
  selection: null,
  view: 'list',
  kind: 'All',
  sort: 'name',
  sortDirection: 'asc',
  folder: '',
  closedFolders: [],
  lease: 'fresh',
  reveal: false,
  demo: null,
};
