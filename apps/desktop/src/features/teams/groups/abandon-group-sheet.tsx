/** Drops the local record of a team whose creation cannot be finished. */

import {
  attemptMutation,
  reportMutationOutcome,
} from '../../../data/command-policy';
import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Inset, InsetRow, SheetDialog } from '../../../components';
import { serverDisplayLabelForStore as displayServerName } from '../../../model';
import type { TeamStore, AgentSnapshot } from '../../../model';
import type { Bridge } from '../../../bridge';
import type { MutationFailureHandler } from '../../../data/mutation-recovery';
import { useSheetGuard } from '../../../navigation-guard';
import { markProfileRostersStale } from '../../../data/roster-staleness';
import { GroupMark } from '../../../components/group-mark';
import { useToast } from '../../../../kit/toasts';

/**
 * The way out of a creation that finishing cannot fix: a name the server
 * refused, or a saved identity bound to a host, account or device that no
 * longer matches. Only the local record is dropped — nothing here reaches the
 * server — so the sheet first says which of the two cases this record is in,
 * because dropping keys the server already knows about is not the same act as
 * dropping keys it never saw.
 */
export function AbandonGroupSheet({
  snapshot,
  bridge,
  store,
  onClose,
  onRemoved,
  onMutationError,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  store: TeamStore;
  onClose: () => void;
  onRemoved: () => Promise<void>;
  onMutationError: MutationFailureHandler;
}): ReactNode {
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  const toasts = useToast();
  const serverName = displayServerName(snapshot, store);
  // Only the two phases that prove nothing was accepted are treated as
  // server-clean. Every other phase, including an unknown one, is read as
  // possibly-created, which is the direction that cannot mislead.
  const reachedServer =
    store.creation_phase !== 'preparing' && store.creation_phase !== 'rejected';
  // The removal is already with the agent once it starts, the same as the
  // device removal sheet, so Escape, the backdrop and Cancel stop answering.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the removal to finish.' }
      : null,
  );
  return (
    <SheetDialog
      danger
      onClose={() => {
        if (busy) return;
        onClose();
      }}
      dismissible={!busy}
      glyph={<GroupMark store={store} />}
      title={`Remove ${store.name}?`}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            danger
            disabled={confirmation !== store.alias || busy}
            onClick={() => {
              setBusy(true);
              void attemptMutation(
                { kind: 'mutation' },
                () => {
                  // The team and its roster leave this profile's catalog.
                  markProfileRostersStale(store.server);
                  return bridge.abandonGroupCreation(store.id);
                },
                onRemoved,
              )
                .then((result) =>
                  reportMutationOutcome(result, onMutationError, () => {
                    toasts.show('Team removed. Refresh pending.');
                  }),
                )
                .finally(() => setBusy(false));
            }}
          >
            Remove team
          </Button>
        </>
      }
    >
      <p>
        {reachedServer
          ? `${store.name} may already have been created on ${serverName}. Removing it from this device will delete its encryption keys, permanently removing access to the team.`
          : `Creation of ${store.name} was not submitted to ${serverName}. Removing it will discard its local draft and keys without affecting the server.`}
      </p>
      <p className="fn">
        This team has no members, channels or items, so nothing else is lost.
        The name is not reserved by this record and can be used again.
      </p>
      <Inset>
        <InsetRow label="Stored as">
          <code>{store.alias}</code>
        </InsetRow>
        <InsetRow label="Confirm">
          <input
            value={confirmation}
            onChange={(event) => setConfirmation(event.target.value)}
            placeholder={`type ${store.alias}`}
          />
        </InsetRow>
      </Inset>
    </SheetDialog>
  );
}
