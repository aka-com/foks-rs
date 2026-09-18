import { GROUP_SETTINGS_TABS } from './types';
import type {
  DevicesSection,
  GroupSettingsTab,
  Location,
  SettingsSection,
} from './types';

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

/**
 * The tab a `section=` value written before the rail belongs to now. Recovery
 * devices, the backup phrase and security keys are Devices; Groups is Teams;
 * Accounts is People.
 */
export const RETIRED_SETTINGS_SECTIONS: Readonly<
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

export const PUBLIC_LOCATION_ALIASES: Readonly<Record<string, Location>> = {
  all: { kind: 'all' },
  // The Alerts page is the top of People now.
  alerts: { kind: 'people' },
  join: { kind: 'teams' },
  groups: { kind: 'teams' },
  create: { kind: 'teams' },
  // Servers used to be its own page; it is a Settings section now, and every
  // former name for that page maps to this section.
  servers: { kind: 'settings', section: 'servers' },
  settings: { kind: 'settings' },
  'servers-list': { kind: 'settings', section: 'servers' },
  'servers-add': { kind: 'settings', section: 'servers' },
  // Recovery devices and security keys are the Devices tab; the accounts pane
  // is the foot of People.
  'settings-macs': { kind: 'devices', section: 'macs' },
  'settings-phrase': { kind: 'devices', section: 'macs' },
  'settings-keys': { kind: 'devices', section: 'keys' },
  'settings-enrol': { kind: 'devices', section: 'keys' },
  'settings-account': { kind: 'people' },
  // The agent and About content is the This Mac page.
  'settings-agent': { kind: 'settings', section: 'mac' },
  'settings-about': { kind: 'settings', section: 'mac' },
};

export function decodeLegacyLocation(
  params: URLSearchParams,
  alias: Location | undefined,
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
