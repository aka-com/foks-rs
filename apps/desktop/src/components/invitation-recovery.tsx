import { useEffect } from 'react';
import { useDeviceCache } from '../device-cache';
import { useMetadataQuery, useMetadataRepository } from '../query-hooks';
import { invitationRecoveryQuery } from '../operation-queries';
import type { Bridge } from '../bridge';
import type { TeamStore } from '../model';
import { Band, Button } from './index';
import { INVITATION_ACTIVITY } from './invitation-panel';
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
  const state = useMetadataQuery(query, { onError });
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
      action={<Button onClick={onReview}>Review invitations</Button>}
    >
      Recover saved invitations and finish pending requests for this team.
    </Band>
  ) : null;
}
