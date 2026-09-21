import { INVITATION_ACTIVITY } from './invitation-activity';
import type { InvitationActivityDetail } from './invitation-activity';

/**
 * Profiles whose cached team rosters must be read again on the next
 * projection, whatever chain sequence the catalog reports for them.
 *
 * A roster read is otherwise skipped while a team's pinned chain sequence is
 * unchanged. Admitting or rejecting a membership request does move that
 * sequence, but the panel that made the decision knows about it before the
 * agent's next catalog read can carry it, so its signal is honored too and
 * the affected profile's rosters are read once more.
 */
const stale = new Set<string>();

/** Records that `profile`'s rosters must be read on the next projection. */
export function markProfileRostersStale(profile: string): void {
  stale.add(profile);
}

/**
 * Whether `profile`'s rosters are marked stale, clearing the mark. A mark
 * set while the read that consumed it is in flight survives it, so the next
 * projection reads the rosters again.
 */
export function consumeProfileRosterStaleness(profile: string): boolean {
  return stale.delete(profile);
}

/** Drops every mark, for a test that starts from a known state. */
export function resetProfileRosterStaleness(): void {
  stale.clear();
}

if (typeof window !== 'undefined') {
  window.addEventListener(INVITATION_ACTIVITY, (event: Event) => {
    const scope = (event as CustomEvent<InvitationActivityDetail>).detail;
    if (scope?.profile) markProfileRostersStale(scope.profile);
  });
}
