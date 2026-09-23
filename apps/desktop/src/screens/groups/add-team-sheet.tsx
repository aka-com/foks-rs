import { CollectionStatus } from '../../components/collection-status';
import { inventoryReadiness } from '../../model';
/** Adds an administered team from another server as a federated member. */

import { useState } from 'react';
import type { ReactNode } from 'react';
import { normalizeCommandError } from '../../bridge';
import {
  Button,
  Chip,
  Inset,
  InsetRow,
  Notice,
  RadioCard,
  RadioGroup,
  SectionLabel,
  SheetDialog,
} from '../../components';
import { isNestingRefusal } from '../../invitation-writes';
import {
  groupDetailFailure,
  serverDisplayLabel,
  serverDisplayLabelForStore as displayServerName,
  serverOf,
  storeOperationAvailability,
} from '../../model';
import type { Store } from '../../model';
import {
  requireWorkflow,
  workflowAvailability,
  workflowMessage,
} from '../../model/workflow-availability';
import { useTabSheetState } from '../../navigation-guard';
import { markProfileRostersStale } from '../../roster-staleness';
import { GroupMark } from '../group-mark';
import { manageReason } from '../group-model';
import {
  DetailUnavailable,
  useSheetWrite,
  VisibilityStepper,
} from './sheet-support';
import type { GroupSheetBaseProps } from './sheet-support';
import { useToast } from '/kit/toasts';

export function AddTeamSheet({
  snapshot,
  bridge,
  store,
  onClose,
  onApplied,
  onMutationError,
}: GroupSheetBaseProps): ReactNode {
  const [visibility, setVisibility] = useTabSheetState('group.visibility', 0);
  const remotes = snapshot.stores.filter(
    (candidate): candidate is Extract<Store, { kind: 'team' }> =>
      candidate.kind === 'team' &&
      candidate.active &&
      candidate.team_kind === 'named' &&
      storeOperationAvailability(snapshot, candidate, 'federation').available &&
      candidate.server !== store.server,
  );
  const [remoteStoreId, setRemoteStoreId] = useTabSheetState(
    'group.remoteStoreId',
    remotes[0]?.id ?? '',
  );
  const remote =
    remotes.find((candidate) => candidate.id === remoteStoreId) ?? remotes[0];
  const [rangeRefusal, setRangeRefusal] = useState('');
  const [rangeBusy, setRangeBusy] = useState(false);
  const toasts = useToast();
  const { busy: writing, write } = useSheetWrite(onApplied, onClose);
  const busy = writing || rangeBusy;
  const serverName = displayServerName(snapshot, store);
  const title = `Add FOKS team to ${store.name}`;
  const federationTarget = {
    profile: store.server,
    account: store.account,
    remoteProfile: remote?.server,
  };
  const federatedMemberReason =
    (store.kind === 'team'
      ? manageReason(snapshot, store, 'federation')
      : 'Select a named team.') ??
    workflowMessage(
      workflowAvailability(snapshot, 'federate', federationTarget),
    );
  // A failed federation read may be reported as either a federation or roster
  // failure.
  const requiredFailure =
    groupDetailFailure(snapshot, store.id, 'federation') ??
    groupDetailFailure(snapshot, store.id, 'roster');
  const changeRange = async (raise: boolean): Promise<void> => {
    if (store.kind !== 'team' || busy) return;
    setRangeBusy(true);
    try {
      await bridge.invitation(
        store.server,
        store.account,
        { action: 'range', team_alias: store.alias, raise },
        null,
      );
      setRangeRefusal('');
      toasts.show(
        raise
          ? `${store.name}’s range raised.`
          : `${store.name}’s range lowered.`,
      );
    } catch (error) {
      await onMutationError(error);
    } finally {
      setRangeBusy(false);
    }
  };
  const apply = (): Promise<void> => {
    if (requiredFailure || !remote) return Promise.resolve();
    setRangeRefusal('');
    return write(
      title,
      async () => {
        if (federatedMemberReason) throw new Error(federatedMemberReason);
        requireWorkflow(snapshot, 'federate', federationTarget);
        // Invalidate both rosters. Returning no profile requests a full
        // catalog refresh after the cross-server change.
        markProfileRostersStale(store.server);
        markProfileRostersStale(remote.server);
        await bridge.addFederatedTeamMember({
          storeId: store.id,
          remoteStoreId: remote.id,
          visibility,
        });
        return undefined;
      },
      async (error) => {
        if (isNestingRefusal(error)) {
          setRangeRefusal(normalizeCommandError(error).message);
          await onMutationError(error, { report: false });
        } else await onMutationError(error);
      },
    );
  };
  return (
    <SheetDialog
      onClose={onClose}
      dismissible={!busy}
      glyph={<GroupMark store={store} />}
      title={title}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            title={federatedMemberReason}
            disabled={
              busy ||
              Boolean(requiredFailure) ||
              !remote ||
              Boolean(federatedMemberReason)
            }
            onClick={() => void apply()}
          >
            Add {remote?.alias ?? 'team'}
          </Button>
        </>
      }
    >
      {requiredFailure ? <DetailUnavailable failure={requiredFailure} /> : null}
      <p>
        Every member of the team you pick will be able to act as a Member in{' '}
        {store.name}. Only teams you administer on another server are listed.
      </p>
      {rangeRefusal ? (
        <Notice
          severity="crit"
          title="Nesting order"
          actions={
            <>
              <Button disabled={busy} onClick={() => void changeRange(false)}>
                Lower {store.name}’s range
              </Button>
              <Button disabled={busy} onClick={() => void changeRange(true)}>
                Raise {store.name}’s range
              </Button>
            </>
          }
        >
          <p>
            {remote?.alias ?? 'The team'} cannot join {store.name} because of
            where the two teams sit in the hierarchy. A team must sit below the
            team it joins. {rangeRefusal}
          </p>
        </Notice>
      ) : null}
      <SectionLabel>Team</SectionLabel>
      <Inset>
        {remotes.length ? (
          <RadioGroup label="Team">
            {remotes.map((group) => {
              const host = serverOf(snapshot, group.id);
              return (
                <RadioCard
                  key={group.id}
                  selected={remote?.id === group.id}
                  onSelect={() => setRemoteStoreId(group.id)}
                  title={group.alias}
                  detail={`on ${host ? serverDisplayLabel(host) : group.server}`}
                />
              );
            })}
          </RadioGroup>
        ) : inventoryReadiness(snapshot, 'teams') !== 'ready' ? (
          <CollectionStatus
            state={inventoryReadiness(snapshot, 'teams')}
            label="teams"
          />
        ) : (
          <InsetRow label="Team">
            <span className="dim">
              No eligible teams. You are not an Admin or Owner of any team on a
              server other than {serverName}.
            </span>
          </InsetRow>
        )}
      </Inset>
      <SectionLabel>Role</SectionLabel>
      <Inset>
        <InsetRow label="Role">
          <Chip>Member</Chip>
        </InsetRow>
        <InsetRow
          label="Visibility"
          action={
            <VisibilityStepper value={visibility} onChange={setVisibility} />
          }
        />
      </Inset>
    </SheetDialog>
  );
}
