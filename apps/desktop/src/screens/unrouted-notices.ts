/**
 * Which notices have a page of their own to resolve them, and which only the
 * account avatar's dot and Settings › Account list. The shell counts the
 * latter for the rail without loading the Settings screen.
 */

import { notesNow, serverDisplayLabel } from '../model';
import type { AgentSnapshot, Notification, Server, TeamStore } from '../model';
import type { Location } from '../location';

/** Whether a note is a catalog read that only a retry can fix. */
export function canRetry(note: Notification): boolean {
  return note.action === 'Retry' && note.id.startsWith('catalog-');
}

/**
 * Returns navigation parameters for the group's Members tab, where setup
 * status and federated membership details can be reviewed and completed.
 */
function openGroup(group: TeamStore): Destination {
  return {
    label: `Open ${group.name}`,
    where: `Teams › ${group.name} › Members`,
    location: { kind: 'group-settings', ref: group.id, tab: 'people' },
  };
}

/** Where a note is resolved: the button's label, and the path it leads to. */
interface Destination {
  label: string;
  /** Human-readable path for the notification card subtitle. */
  where: string;
  location: Location;
}

/** A server profile named by a note id's reason prefix, e.g. `check-in-expired-acme`. */
function serverNamedByReason(
  snapshot: AgentSnapshot,
  id: string,
): Server | undefined {
  const reasons = [
    'check-in-expired',
    'check-in-unavailable',
    'server-status-unavailable',
    'verification-failed',
    'verification-required',
    'status-unavailable',
  ];
  for (const reason of reasons) {
    if (!id.startsWith(`${reason}-`)) continue;
    const profile = id.slice(reason.length + 1);
    const server = snapshot.servers.find(
      (entry) => entry.profileName === profile,
    );
    if (server) return server;
  }
  return undefined;
}

/** A team named by a `group-roster-unavailable-` or `group-federation-unavailable-` note id. */
function groupNamedByFailure(
  snapshot: AgentSnapshot,
  id: string,
): TeamStore | undefined {
  const prefixes = [
    'group-roster-unavailable-',
    'group-federation-unavailable-',
  ];
  for (const prefix of prefixes) {
    if (!id.startsWith(prefix)) continue;
    const storeId = id.slice(prefix.length);
    return snapshot.stores.find(
      (store): store is TeamStore =>
        store.kind === 'team' && store.id === storeId,
    );
  }
  return undefined;
}

/**
 * Where a note is resolved, when its id names a place this Mac holds. A note
 * about a server — a lapsed check-in, an unverified or blocked server, a
 * status read that failed — names the profile in its id's reason prefix and
 * resolves to that server's own Settings page, where the same state already
 * shows in "Needs attention". A note about a team's roster or federation read
 * failing names the store and resolves to that team's own page. The
 * synthetic `lease-`, `team-` and `fed-` prefixes are the fixture's own
 * shorthand for the same three destinations, kept for its notices. A note
 * whose place cannot be derived keeps the label it always had rather than
 * opening the wrong page.
 *
 * A note this resolves is not drawn on Settings › Account at all: the place
 * it names already shows the same state, so this function is used only to
 * tell such a note apart from one that has nowhere else to go.
 */
function destinationOf(
  snapshot: AgentSnapshot,
  note: Notification,
): Destination | null {
  const namedServer =
    (note.id.startsWith('lease-')
      ? snapshot.servers.find(
          (entry) => entry.profileName === note.id.slice('lease-'.length),
        )
      : undefined) ?? serverNamedByReason(snapshot, note.id);
  if (namedServer)
    return {
      label: 'Open server',
      where: `Settings › Account › ${serverDisplayLabel(namedServer)}`,
      location: {
        kind: 'settings',
        section: 'account',
        profile: namedServer.profileName,
      },
    };
  const namedGroup = groupNamedByFailure(snapshot, note.id);
  if (namedGroup) return openGroup(namedGroup);
  if (note.id.startsWith('team-')) {
    const alias = note.id.slice('team-'.length);
    const matches = snapshot.stores.filter(
      (store): store is TeamStore =>
        store.kind === 'team' && store.alias === alias,
    );
    return matches.length === 1 ? openGroup(matches[0]) : null;
  }
  if (note.id.startsWith('fed-')) {
    // `fed-homelab` is Homelab's membership *in* another group, and only the
    // host group can restore it. The note names the federated group alone, so
    // it points somewhere only while exactly one inactive membership carries
    // that alias: two hosts would make the choice of page a guess.
    const alias = note.id.slice('fed-'.length);
    const matches = snapshot.federation.filter(
      (entry) => entry.remote_team_alias === alias && !entry.active,
    );
    if (matches.length !== 1) return null;
    const host = snapshot.stores.find(
      (store): store is TeamStore =>
        store.kind === 'team' && store.id === matches[0].store,
    );
    return host ? openGroup(host) : null;
  }
  return null;
}

/**
 * The notices no tab can resolve: a catalog read that only a retry can fix,
 * or a note whose place could not be derived — an alias that matches more
 * than one store, or a federated membership this Mac cannot place. This is what the
 * rail's account-avatar dot still counts, now that every other note type has
 * a badge or an established home of its own.
 */
export function unroutedNotices(snapshot: AgentSnapshot): Notification[] {
  return notesNow(snapshot).filter(
    (note) => canRetry(note) || destinationOf(snapshot, note) === null,
  );
}
