import { useEffect, useState } from 'react';
import type { Bridge } from '../bridge';
import type { InvitationReply } from '../invitation-contract';
import type { TeamStore } from '../model';
import { Band, Button } from './index';
import { INVITATION_ACTIVITY } from './invitation-panel';
const rows = (value: InvitationReply) =>
  Array.isArray(value) ? value : (value.rows ?? [value]);

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
  const [count, setCount] = useState(0);
  useEffect(() => {
    let live = true;
    let sequence = 0;
    const refresh = () => {
      const request = ++sequence;
      void Promise.all([
        bridge.invitation(
          store.server,
          store.account,
          { action: 'list' },
          null,
        ),
        bridge.invitation(
          store.server,
          store.account,
          {
            action: 'pending-approvals',
            team_alias: store.alias,
          },
          null,
        ),
      ])
        .then(([operations, approvals]) => {
          if (!live || request !== sequence) return;
          setCount(
            rows(operations).filter(
              (row) =>
                row.team_id === store.team_id_hex && row.state !== 'cancelled',
            ).length +
              rows(approvals).filter(
                (row) => row.request_id && row.state !== 'complete',
              ).length,
          );
        })
        .catch((error: unknown) => {
          if (live && request === sequence) onError(error);
        });
    };
    refresh();
    window.addEventListener(INVITATION_ACTIVITY, refresh);
    return () => {
      live = false;
      window.removeEventListener(INVITATION_ACTIVITY, refresh);
    };
  }, [
    bridge,
    store.server,
    store.account,
    store.alias,
    store.team_id_hex,
    onError,
  ]);
  return count ? (
    <Band
      label="Invitation activity"
      action={<Button onClick={onReview}>Review invitations</Button>}
    >
      Recover saved invitations and finish pending requests for this group.
    </Band>
  ) : null;
}
