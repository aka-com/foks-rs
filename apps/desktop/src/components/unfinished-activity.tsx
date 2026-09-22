/** Summary band for pending membership and invitation operations. */

import { useCallback, useEffect } from 'react';
import type { ReactNode } from 'react';
import { useDeviceCache } from '../device-cache';
import { useMetadataQuery, useMetadataRepository } from '../query-hooks';
import {
  invitationRecoveryQuery,
  reportableTeamRequestError,
} from '../operation-queries';
import type { Bridge, PendingOperation } from '../bridge';
import type { TeamStore } from '../model';
import { Band, Button } from './index';
import { INVITATION_ACTIVITY } from '../invitation-activity';

/** Formats pending and blocking operation counts for the summary band. */
export function unfinishedActivityText(
  total: number,
  blocking: number,
): string {
  const count = `${total} incomplete membership ${total === 1 ? 'change' : 'changes'}.`;
  if (!blocking) return count;
  if (total === 1) return `${count} It blocks other actions.`;
  if (blocking === total) return `${count} All of them block other actions.`;
  return `${count} ${blocking} of them ${blocking === 1 ? 'blocks' : 'block'} other actions.`;
}

export function UnfinishedActivity({
  bridge,
  store,
  membership,
  invitations,
  onReview,
  onError,
}: {
  bridge: Bridge;
  store: TeamStore;
  /** Membership operations that block subsequent membership writes. */
  membership: readonly PendingOperation[];
  /** Include pending invitation and approval operations. */
  invitations: boolean;
  onReview: () => void;
  onError: (error: unknown) => void;
}): ReactNode {
  const devices = useDeviceCache();
  const repository = useMetadataRepository(bridge, devices?.repository);
  const query = invitations
    ? invitationRecoveryQuery(repository, bridge, store)
    : null;
  // Ignore transient read-admission failures because the band is optional
  // status metadata. Report errors that affect the active session.
  const report = useCallback(
    (error: unknown) => {
      if (reportableTeamRequestError(error)) onError(error);
    },
    [onError],
  );
  const state = useMetadataQuery(query, { onError: report });
  useEffect(() => {
    const refresh = (event: Event) => {
      const scope = (
        event as CustomEvent<{ profile?: string; account?: string }>
      ).detail;
      if (
        scope &&
        (scope.profile !== store.server || scope.account !== store.account)
      )
        return;
      repository.invalidate([
        'invitation-recovery',
        store.server,
        store.account,
      ]);
    };
    window.addEventListener(INVITATION_ACTIVITY, refresh);
    return () => window.removeEventListener(INVITATION_ACTIVITY, refresh);
  }, [repository, store.server, store.account]);
  const blocking = membership.length;
  const total = blocking + (state.data ?? 0);
  if (!total) return null;
  return (
    <Band
      // Announce newly detected blocking membership operations. Invitation
      // activity alone does not require a live-region announcement.
      live={blocking > 0}
      label="Unfinished activity"
      action={
        <Button size="sm" variant="primary" onClick={onReview}>
          Review
        </Button>
      }
    >
      {unfinishedActivityText(total, blocking)}
    </Band>
  );
}
