import { GROUP_SETTINGS_TABS } from './types';
import type {
  DevicesSection,
  GroupSettingsTab,
  Location,
  SettingsSection,
} from './types';

/**
 * Former `section=` values that are pages of one of the three sections now.
 * `credentials` (the passphrase rows) is Account and `notifications` is
 * Preferences;
 * `device`, `about` and the older `agent` are Device; `security-keys` was a
 * list of links to each server's page, now at the bottom of Account. `servers`
 * is the retired id of the same page.
 */
export const SETTINGS_SECTION_ALIASES: Readonly<
  Record<string, SettingsSection>
> = {
  credentials: 'account',
  notifications: 'preferences',
  device: 'mac',
  about: 'mac',
  agent: 'mac',
  servers: 'account',
  'security-keys': 'account',
};

/**
 * Settings › Account, where every former address for an account's own page
 * lands: the Account tab (`state=people`), the Alerts page, and the accounts
 * pane Settings once held (`settings-account`, `section=account`). The last
 * of these needs no alias: `account` is a current section id, which
 * `decodeLocation` reads through the sections list.
 */
export const ACCOUNT_SECTION: Location = {
  kind: 'settings',
  section: 'account',
};

/**
 * The tab a `section=` value written before the rail belongs to now. Recovery
 * devices, the backup phrase and security keys are Devices; Groups is Teams.
 * `account` is a current section and needs no entry here.
 */
export const RETIRED_SETTINGS_SECTIONS: Readonly<
  Record<
    string,
    { kind: 'devices'; section: DevicesSection } | { kind: 'teams' }
  >
> = {
  macs: { kind: 'devices', section: 'macs' },
  phrase: { kind: 'devices', section: 'macs' },
  keys: { kind: 'devices', section: 'keys' },
  groups: { kind: 'teams' },
};

export const PUBLIC_LOCATION_ALIASES: Readonly<Record<string, Location>> = {
  all: { kind: 'all' },
  // The Alerts page is the top of Settings › Account now.
  alerts: ACCOUNT_SECTION,
  // Creating a team and pasting an invitation are sheets over the Teams list,
  // so the names that promise them open that list with the sheet up.
  join: { kind: 'teams', open: 'join' },
  groups: { kind: 'teams' },
  create: { kind: 'teams', open: 'create' },
  // Servers now lives at the bottom of Account. Every former name for that
  // page maps there, while a named profile still opens its server detail.
  servers: ACCOUNT_SECTION,
  settings: { kind: 'settings' },
  'servers-list': ACCOUNT_SECTION,
  'servers-add': ACCOUNT_SECTION,
  // Recovery devices and security keys are the Devices tab; the accounts pane
  // is the Account section.
  'settings-macs': { kind: 'devices', section: 'macs' },
  'settings-phrase': { kind: 'devices', section: 'macs' },
  'settings-keys': { kind: 'devices', section: 'keys' },
  'settings-enrol': { kind: 'devices', section: 'keys' },
  'settings-account': ACCOUNT_SECTION,
  // The Account tab, before it became the first Settings section.
  people: ACCOUNT_SECTION,
  // The agent and About content is the Device page.
  'settings-agent': { kind: 'settings', section: 'mac' },
  'settings-about': { kind: 'settings', section: 'mac' },
};

export function decodeLegacyLocation(
  params: URLSearchParams,
  alias: Location | undefined,
  allowSettingsProfile = false,
): Location | null {
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
      allowSettingsProfile && alias.section === 'account'
        ? (params.get('profile') ?? alias.profile)
        : undefined;
    return {
      ...alias,
      ...(store ? { store } : {}),
      ...(profile ? { profile } : {}),
    };
  }
  if (alias?.kind === 'devices' || alias?.kind === 'teams') {
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
