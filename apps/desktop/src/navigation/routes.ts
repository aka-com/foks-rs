import type { AccountStore, Store, StoreRef } from '../model/types';
import { DEFAULT_SETTINGS_SECTION } from './types';
import type { Location, RailTab, SettingsSection } from './types';

/**
 * The Settings page an address opens. An address that names a section opens
 * it; a server profile is its own page under Account; any other address
 * opens the sub-navigation's first page.
 */
export function settingsSectionOf(
  location: Extract<Location, { kind: 'settings' }>,
): SettingsSection {
  if (location.section === 'servers') return 'account';
  if (location.section) return location.section;
  return DEFAULT_SETTINGS_SECTION;
}

/** Older Teams URLs used `store` for both account and team identities. */
export function normalizeTeamsLocation(
  stores: readonly Store[],
  location: Extract<Location, { kind: 'teams' }>,
): Extract<Location, { kind: 'teams' }> {
  if (location.ref || !location.store) return location;
  const team = stores.find(
    (store) => store.id === location.store && store.kind === 'team',
  );
  if (!team) return location;
  const owner = stores.find(
    (store) =>
      store.kind === 'account' &&
      store.server === team.server &&
      store.account === team.account,
  );
  const normalized = { ...location, ref: team.id };
  if (owner) normalized.store = owner.id;
  else delete normalized.store;
  return normalized;
}

/** Resolve the account through which a page's object is accessed. */
export function accountAtLocation(
  stores: readonly Store[],
  location: Location,
  fallback?: StoreRef,
): AccountStore | undefined {
  if (location.kind === 'teams')
    location = normalizeTeamsLocation(stores, location);
  const accounts = stores.filter(
    (store): store is AccountStore => store.kind === 'account',
  );
  const requested = 'store' in location ? location.store : undefined;
  if ('ref' in location && location.ref) {
    const target = stores.find((store) => store.id === location.ref);
    if (!target || (location.kind === 'teams' && target.kind !== 'team'))
      return undefined;
    return accounts.find(
      (account) =>
        (!requested || account.id === requested) &&
        account.server === target.server &&
        account.account === target.account,
    );
  }
  if (requested) return accounts.find((account) => account.id === requested);
  if (location.kind === 'settings' && location.profile)
    return accounts.find((account) => account.server === location.profile);
  return accounts.find((account) => account.id === fallback) ?? accounts[0];
}

/** The rail tab that owns a location, or `null` for first run. */
export function railTabOf(location: Location): RailTab | null {
  switch (location.kind) {
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
 * Settings and Devices. The `store` parameter that Teams, Devices and
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

/** Returns whether two locations identify the same navigation target. */
export function sameLocation(a: Location, b: Location): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === 'store' && b.kind === 'store') return a.ref === b.ref;
  if (a.kind === 'chat' && b.kind === 'chat')
    return a.ref === b.ref && a.channel === b.channel;
  if (a.kind === 'group-settings' && b.kind === 'group-settings')
    return a.ref === b.ref && a.tab === b.tab;
  // The sheet intent is part of the address until the page consumes it:
  // asking the list for a sheet it is not showing is a move, not a no-op.
  if (a.kind === 'teams' && b.kind === 'teams')
    return a.store === b.store && a.ref === b.ref && a.open === b.open;
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
