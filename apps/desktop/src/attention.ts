/**
 * The attention centre's model: what needs the reader, the setup steps left
 * after first run, from facts the shell already holds.
 *
 * Nothing here reads the agent. The items restate, one entity at a time, the
 * same signals the rail's counts and dots summarise: a team's join requests,
 * the Devices reasons in `device-alert.ts`, the servers the Settings dot
 * answers for, and the notices only Settings › Account lists. They add the
 * interrupted membership changes a team's own page bands. Each item names
 * the page that resolves it, so the list is a set of links and not a second
 * place to act. The checklist reads the device lists the shell preloads, the
 * app lock state the host reports and the catalog; a step whose fact has not
 * been read this session is left out rather than guessed.
 */

import type { DevicesSheetIntent, Location } from './location';
import type {
  AccountStore,
  AgentSnapshot,
  Notification,
  StoreRef,
} from './model';
import {
  accountStores,
  inventoryReadiness,
  isTrashPath,
  kindOf,
  plural,
  serverDisplayLabel,
  settingsServerFlag,
  usernameOf,
} from './model';
import { teamRequestTotals } from './operation-queries';
import type { TeamRequestCount } from './operation-queries';
import type { recoveryPhraseTests } from './recovery-phrase-tests';
import type { DeviceAlertSnapshot } from './screens/device-alert';
import { deviceAlertReasons } from './screens/device-alert';

/** Which of the shell's signals an item restates. */
export type AttentionSource = 'servers' | 'devices' | 'teams' | 'notices';

export interface AttentionItem {
  /** Stable across renders while the fact holds. */
  id: string;
  source: AttentionSource;
  /** `warning` for a state that stops work or risks access; `info` otherwise. */
  severity: 'warning' | 'info';
  title: string;
  detail?: string;
  /** The page where the item is resolved. */
  location: Location;
}

export interface AttentionFacts {
  snapshot: AgentSnapshot;
  /** The shared per-team request counts the Teams badge reads. */
  teamRequests: ReadonlyMap<StoreRef, TeamRequestCount>;
  /**
   * Each team's interrupted membership changes, from the journal a team's
   * page reads (`usePendingMembershipCounts`).
   */
  pendingMembership: ReadonlyMap<StoreRef, number>;
  devices: DeviceAlertSnapshot;
  phraseTests: ReturnType<typeof recoveryPhraseTests>;
  /** The notices only Settings › Account lists (`unroutedNotices`). */
  notices: readonly Notification[];
  now?: number;
}

/** "alice", or "alice on Acme" where more than one account is on this Mac. */
function accountLabel(snapshot: AgentSnapshot, store: AccountStore): string {
  const name = usernameOf(snapshot, store) ?? store.account;
  if (accountStores(snapshot).length < 2) return name;
  const server = snapshot.servers.find(
    (entry) => entry.profileName === store.server,
  );
  return server ? `${name} on ${serverDisplayLabel(server)}` : name;
}

function accountOf(
  snapshot: AgentSnapshot,
  ref: StoreRef,
): AccountStore | undefined {
  return accountStores(snapshot).find((store) => store.id === ref);
}

/**
 * Every item that needs the reader, warnings first and otherwise in the
 * order servers, devices, teams, notices.
 */
export function attentionItems({
  snapshot,
  teamRequests,
  pendingMembership,
  devices,
  phraseTests,
  notices,
  now = Date.now(),
}: AttentionFacts): AttentionItem[] {
  const items: AttentionItem[] = [];
  for (const server of snapshot.servers) {
    const flag = settingsServerFlag(snapshot, server);
    if (!flag) continue;
    items.push({
      id: `server:${server.profileName}`,
      source: 'servers',
      severity: 'warning',
      title:
        flag === 'check-in-expired'
          ? 'Check-in expired'
          : 'Server not verified',
      detail: serverDisplayLabel(server),
      location: {
        kind: 'settings',
        section: 'account',
        profile: server.profileName,
      },
    });
  }
  for (const reason of deviceAlertReasons(
    snapshot,
    devices,
    phraseTests,
    now,
  )) {
    const account = accountOf(snapshot, reason.store);
    if (!account) continue;
    const location: Location = { kind: 'devices', store: account.id };
    const detail = accountLabel(snapshot, account);
    switch (reason.kind) {
      case 'pairing-offer':
        items.push({
          id: `pairing-offer:${account.id}`,
          source: 'devices',
          severity: 'info',
          title: 'A pairing offer is open',
          detail,
          location,
        });
        break;
      case 'no-recovery-phrase':
        items.push({
          id: `no-recovery-phrase:${account.id}`,
          source: 'devices',
          severity: 'warning',
          title: 'No recovery phrase',
          detail,
          location,
        });
        break;
      case 'recovery-phrase-test':
        items.push({
          id: `recovery-phrase-test:${reason.backup.backupId}`,
          source: 'devices',
          severity: 'info',
          title: reason.never
            ? `Recovery phrase ${reason.backup.backupAlias} has never been tested`
            : `Recovery phrase ${reason.backup.backupAlias} is due for a test`,
          detail,
          location,
        });
        break;
    }
  }
  for (const store of snapshot.stores) {
    if (store.kind !== 'team') continue;
    const count = pendingMembership.get(store.id);
    if (!count) continue;
    items.push({
      id: `team-membership:${store.id}`,
      source: 'teams',
      // An interrupted change blocks every later membership change on the
      // team until it is resumed.
      severity: 'warning',
      title: `${plural(count, 'incomplete membership change')} in ${store.name}`,
      detail:
        count === 1
          ? 'It blocks other membership changes on this team.'
          : 'They block other membership changes on this team.',
      location: { kind: 'group-settings', ref: store.id },
    });
  }
  for (const { store, total, truncated } of teamRequestTotals(
    snapshot,
    teamRequests,
  )) {
    if (!total) continue;
    items.push({
      id: `team-requests:${store.id}`,
      source: 'teams',
      severity: 'info',
      title: `${truncated ? 'At least ' : ''}${plural(total, 'request')} to join ${store.name}`,
      location: { kind: 'group-settings', ref: store.id, tab: 'requests' },
    });
  }
  for (const note of notices) {
    items.push({
      id: `notice:${note.id}`,
      source: 'notices',
      severity: note.severity === 'info' ? 'info' : 'warning',
      title: note.title,
      detail: note.detail,
      location: { kind: 'settings', section: 'account' },
    });
  }
  // A stable sort: warnings rise, and each group keeps the source order.
  return [
    ...items.filter((item) => item.severity === 'warning'),
    ...items.filter((item) => item.severity === 'info'),
  ];
}

/** The steps the "Get set up" checklist can list. */
export type SetupStepId =
  'recovery-phrase' | 'second-key' | 'app-lock' | 'first-password' | 'team';

export interface SetupStep {
  id: SetupStepId;
  title: string;
  /** What the step protects or enables, or why it cannot be done here. */
  detail: string;
  done: boolean;
  /**
   * The step cannot be done on this computer. It is listed for its detail,
   * and is not counted toward the checklist's progress or completion.
   */
  unavailable?: true;
  /** Where the step is done. Absent when nothing in the app can do it. */
  location?: Location;
}

export interface SetupFacts {
  snapshot: AgentSnapshot;
  devices: DeviceAlertSnapshot;
  /** What the host reported about the app lock; absent until it answers. */
  appLock?: { available: boolean; unavailableReason?: string };
  /** Whether the vault catalog has been read, so an empty one is known empty. */
  catalogReady: boolean;
}

/**
 * The "Get set up" steps, in the order they are worth doing. A step is left
 * out while its fact is unknown: an account whose keys this session has not
 * read says nothing about its recovery phrase or its second key, the same
 * rule the Devices dot follows. A per-account step is listed as open once one
 * read account lacks it, and as done only once every account has been read.
 */
export function setupChecklist({
  snapshot,
  devices,
  appLock,
  catalogReady,
}: SetupFacts): SetupStep[] {
  const steps: SetupStep[] = [];
  const accounts = accountStores(snapshot);
  const devicesSheet = (
    store: AccountStore,
    open: DevicesSheetIntent,
  ): Location => ({ kind: 'devices', store: store.id, open });

  const missingPhrase = accounts.find(
    (store) => devices.phraseLists.get(store.id)?.length === 0,
  );
  if (
    missingPhrase ||
    (accounts.length &&
      accounts.every((store) => devices.phraseLists.has(store.id)))
  ) {
    steps.push({
      id: 'recovery-phrase',
      title: 'Create a recovery phrase',
      detail: missingPhrase
        ? `Restores ${accountLabel(snapshot, missingPhrase)} if every device is lost.`
        : 'Every account on this device has one.',
      done: !missingPhrase,
      ...(missingPhrase
        ? { location: devicesSheet(missingPhrase, 'phrase') }
        : {}),
    });
  }

  // A hardware-card key is one of the account's devices, so a single key is
  // one whether it is this computer's or a card's.
  const singleKey = accounts.find((store) => {
    const count = devices.keyCounts.get(store.id);
    return count !== undefined && count.devices < 2;
  });
  if (
    singleKey ||
    (accounts.length &&
      accounts.every((store) => devices.keyCounts.has(store.id)))
  ) {
    steps.push({
      id: 'second-key',
      title: 'Add a second device or a hardware key',
      detail: singleKey
        ? `${accountLabel(snapshot, singleKey)} has one key. Another device or a hardware key keeps the account usable if that key is lost.`
        : 'Every account on this device has a second key.',
      done: !singleKey,
      ...(singleKey ? { location: devicesSheet(singleKey, 'add') } : {}),
    });
  }

  if (appLock) {
    // Whether the lock is available depends on this computer's own sign-in
    // set-up, which nothing in the app changes, so the step has no link, and
    // an unavailable lock is not an open step.
    steps.push({
      id: 'app-lock',
      title: 'App lock',
      detail: appLock.available
        ? 'Available on this computer.'
        : (appLock.unavailableReason ??
          'App lock is not available on this computer.'),
      done: appLock.available,
      ...(appLock.available ? {} : { unavailable: true }),
    });
  }

  const password = snapshot.items.some(
    (item) => !isTrashPath(item.path) && kindOf(item) === 'Password',
  );
  if (password || catalogReady) {
    steps.push({
      id: 'first-password',
      title: 'Save a password',
      detail: password
        ? 'A vault on this device holds a password.'
        : 'Add a password to a vault.',
      done: password,
      ...(password ? {} : { location: { kind: 'files' } }),
    });
  }

  const team = snapshot.stores.some(
    (store) => store.kind === 'team' && store.team_kind === 'named',
  );
  if (team || inventoryReadiness(snapshot, 'teams') === 'ready') {
    steps.push({
      id: 'team',
      title: 'Join or create a team',
      detail: team
        ? 'An account on this device is in a team.'
        : 'Teams share vaults and chat between people.',
      done: team,
      ...(team ? {} : { location: { kind: 'teams' } }),
    });
  }
  return steps;
}
