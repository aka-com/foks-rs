import {
  decodeLegacyLocation,
  PUBLIC_LOCATION_ALIASES,
  RETIRED_SETTINGS_SECTIONS,
  SETTINGS_SECTION_ALIASES,
} from './legacy-routes';
import { settingsSectionOf } from './routes';
import { GROUP_SETTINGS_TABS, SETTINGS_SECTIONS } from './types';
import type {
  DevicesSection,
  GroupSettingsTab,
  Location,
  SettingsSection,
} from './types';

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
            settingsSectionOf(location) === 'account'
              ? (location.profile ?? null)
              : null,
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

export function decodeProductionLocation(search: string): Location | null {
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
    // Sections that folded into one of the four pages open that page.
    const resolved =
      section && SETTINGS_SECTION_ALIASES[section]
        ? SETTINGS_SECTION_ALIASES[section]
        : section;
    // A profile opens a server detail at the bottom of Account. The retired
    // Servers section and addresses with no section keep their profile.
    // Legacy `security-keys` URLs identified only a list, so they redirect to
    // the Account root.
    const profile =
      section === null || section === 'account' || section === 'servers'
        ? (params.get('profile') ?? undefined)
        : undefined;
    return resolved &&
      (SETTINGS_SECTIONS as readonly string[]).includes(resolved)
      ? {
          kind: 'settings',
          section: resolved as SettingsSection,
          ...(store ? { store } : {}),
          ...(profile ? { profile } : {}),
        }
      : {
          kind: 'settings',
          ...(store ? { store } : {}),
          ...(profile ? { profile } : {}),
        };
  }
  if (state === 'servers') {
    const profile = params.get('profile') ?? undefined;
    return {
      kind: 'settings',
      section: 'account',
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
  return decodeLegacyLocation(
    params,
    PUBLIC_LOCATION_ALIASES[state],
    state === 'servers-list' || state === 'servers-add',
  );
}

/** The href a location deep-links to, given the address the page is at. */
export function locationHref(href: string, location: Location): string {
  const { state, params } = encodeLocation(location);
  return setUrl(href, state, params);
}
