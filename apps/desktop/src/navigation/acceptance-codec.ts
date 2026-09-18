import { decodeLegacyLocation, PUBLIC_LOCATION_ALIASES } from './legacy-routes';
import { decodeProductionLocation } from './production-codec';
import { decodeSceneNavigation, oneOf } from './scene-codec';
import { INITIAL_SCENE } from './types';
import type { FirstRunPath, Location, Scene } from './types';

/**
 * The stable `?state=` names for the places that are locations. States that
 * are a *sheet* over a location — `new`,
 * `manage`, `conflict` — are not locations and are not listed; those belong to
 * the screens that own them.
 */
const STATE_ALIASES: Readonly<Record<string, Location>> = {
  ...PUBLIC_LOCATION_ALIASES,
  personal: { kind: 'store', ref: 'acct:personal' },
  work: { kind: 'store', ref: 'acct:work' },
  household: { kind: 'store', ref: 'team:household' },
  group: { kind: 'store', ref: 'team:household' },
  homelab: { kind: 'store', ref: 'team:homelab' },
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
  'servers-unprobed': {
    kind: 'settings',
    section: 'servers',
    profile: 'partner',
  },
  'servers-check': { kind: 'settings', section: 'servers', profile: 'partner' },
  'settings-macs-work': {
    kind: 'devices',
    section: 'macs',
    store: 'acct:work',
  },
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
  const location = decodeProductionLocation(search);
  if (location) return location;
  const params = new URLSearchParams(search);
  const state = params.get('state');
  if (!state) return null;
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
  return decodeLegacyLocation(params, STATE_ALIASES[state]);
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

/**
 * Decodes a complete Scene configuration from a URL search query, resolving
 * named aliases first and applying explicit parameters as overrides.
 */
export function decodeAcceptanceScene(search: string): Scene {
  const params = new URLSearchParams(search);
  const name = params.get('state') ?? '';
  const alias = SCENE_ALIASES[name] ?? {};
  const location =
    decodeLocation(search) ?? alias.location ?? INITIAL_SCENE.location;
  return {
    location,
    ...decodeSceneNavigation(params, alias),
    lease:
      oneOf(['fresh', 'lapsed'] as const, params.get('lease')) ??
      alias.lease ??
      INITIAL_SCENE.lease,
    reveal: alias.reveal ?? false,
    demo: alias.demo ?? null,
  };
}

export { decodeAcceptanceScene as decodeScene };
