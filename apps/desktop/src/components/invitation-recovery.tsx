import { useCallback, useEffect } from 'react';
import { useDeviceCache } from '../device-cache';
import { useMetadataQuery, useMetadataRepository } from '../query-hooks';
import {
  invitationRecoveryQuery,
  reportableTeamRequestError,
} from '../operation-queries';
import type { Bridge } from '../bridge';
import type { TeamStore } from '../model';
import { Band, Button } from './index';
import { INVITATION_ACTIVITY } from '../invitation-activity';
/** Both native journals are required: invitation operations and membership approvals. */
export function InvitationRecovery({
  bridge,
  store,
  onReview,
  onError,
}: {
  bridge: Bridge;
  store: TeamStore;
  onReview: () => void;
  onError: (error: unknown) => void;
}) {
  const devices = useDeviceCache();
  const repository = useMetadataRepository(bridge, devices?.repository);
  const query = invitationRecoveryQuery(repository, bridge, store);
  // The band is a badge, read on every visit to a team the user manages: a
  // profile queue that could not admit the read, or an agent that was busy,
  // leaves it blank rather than saying so. What changes the session itself
  // still reaches the handler, as it does for the team request count.
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
  const count = state.data ?? 0;
  return count ? (
    <Band
      label="Invitation activity"
      action={<Button onClick={onReview}>Review</Button>}
    >
      {count === 1
        ? '1 invitation or approval has not finished.'
        : `${count} invitations or approvals have not finished.`}
    </Band>
  ) : null;
}
