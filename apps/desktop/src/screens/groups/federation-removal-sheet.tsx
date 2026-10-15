/** Removes a federated team from a group and rotates the group's keys. */

import {
  attemptMutation,
  reportMutationOutcome,
} from '../../data/command-policy';
import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Inset, InsetRow, SheetDialog } from '../../components';
import { partiesOf, shortId } from '../../model';
import type { FederationEntry, Store, AgentSnapshot } from '../../model';
import type { Bridge } from '../../bridge';
import type { MutationFailureHandler } from '../../data/mutation-recovery';
import { markProfileRostersStale } from '../../data/roster-staleness';
import { GroupMark } from '../../components/group-mark';
import { useToast } from '../../../kit/toasts';
import {
  entriesOf,
  exposureOrder,
  memberExposureReadiness,
  memberReads,
  withExposureReview,
} from '../../exposure-review';
import { ExposureSummary, unstoredReview } from '../exposure-review';

export function FederationRemovalSheet({
  snapshot,
  bridge,
  store,
  entry,
  onClose,
  onApplied,
  onMutationError,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  store: Extract<Store, { kind: 'team' }>;
  entry: FederationEntry;
  onClose: () => void;
  /**
   * Takes the profile, so the read back after a removal is that profile's
   * rather than every profile's. The parent already passes a handler with
   * this signature; the narrower declaration here only hid the argument.
   */
  onApplied: (message: string, profile?: string) => Promise<void>;
  onMutationError: MutationFailureHandler;
}): ReactNode {
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const toasts = useToast();
  const name = entry.remote_team_alias;
  // The remote team is a party of this team's roster. What its members could
  // read through it is listed before the write and kept for review after it,
  // as for a removed member.
  const party = partiesOf(snapshot, store.id).find(
    (candidate) =>
      candidate.party_kind !== 'user' &&
      candidate.party_id_hex === entry.remote_team_id_hex &&
      (!candidate.scoped_host_id_hex ||
        candidate.scoped_host_id_hex === entry.remote_host_id_hex),
  );
  const readiness = memberExposureReadiness(snapshot, store.id);
  const exposed =
    party && readiness === 'ready'
      ? exposureOrder(memberReads(snapshot, party))
      : [];
  const apply = async (): Promise<void> => {
    if (!confirmed || busy || !entry.active || readiness === 'loading') return;
    const entries = entriesOf(exposed);
    setBusy(true);
    try {
      const result = await attemptMutation(
        { kind: 'resumable', operation: 'remove-federated-membership' },
        () => {
          // Removing a federated team rekeys the team, which moves its chain. The
          // read back must not reuse the roster it holds even if the agent's
          // catalog has not yet caught up with that sequence.
          markProfileRostersStale(store.server);
          return withExposureReview(
            {
              scope: store.id,
              party: party?.party_id_hex ?? entry.remote_team_id_hex,
              name,
              cause: 'removed',
              entries,
            },
            () =>
              bridge.removeFederatedGroup({
                storeId: store.id,
                remoteHostIdHex: entry.remote_host_id_hex,
                remoteTeamIdHex: entry.remote_team_id_hex,
              }),
            () => toasts.show(unstoredReview(name)),
          );
        },
        () =>
          onApplied(
            `${entry.remote_team_alias} removed and team keys rotated`,
            store.server,
          ),
      );
      if (
        await reportMutationOutcome(result, onMutationError, () => {
          toasts.show('Federated team removed. Refresh pending.');
        })
      )
        onClose();
    } finally {
      setBusy(false);
    }
  };
  return (
    <SheetDialog
      danger
      dismissible={false}
      onClose={onClose}
      glyph={<GroupMark store={store} />}
      title={`Remove ${entry.remote_team_alias} from ${store.name}?`}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            danger
            disabled={
              busy || !confirmed || !entry.active || readiness === 'loading'
            }
            onClick={() => void apply()}
          >
            Remove and rekey
          </Button>
        </>
      }
    >
      <p>
        Every member of <b>{entry.remote_team_alias}</b> will lose access to
        this team. Team encryption keys will be updated to block future access.
      </p>
      <Inset>
        <InsetRow label="Remote host">
          <code>{entry.remote_host_id_hex}</code>
        </InsetRow>
        <InsetRow label="Remote team">
          <code title={entry.remote_team_id_hex}>
            {shortId(entry.remote_team_id_hex)}
          </code>
        </InsetRow>
      </Inset>
      <label className="checkline">
        <input
          type="checkbox"
          checked={confirmed}
          disabled={busy}
          onChange={(event) => setConfirmed(event.target.checked)}
        />
        Remove every member of {entry.remote_team_alias} and rotate keys
      </label>
      {entry.active ? (
        <ExposureSummary
          snapshot={snapshot}
          name={name}
          heading={`Items ${name} could read`}
          empty={`${name} cannot read any item this account can list.`}
          items={exposed}
          readiness={readiness}
        />
      ) : null}
    </SheetDialog>
  );
}
