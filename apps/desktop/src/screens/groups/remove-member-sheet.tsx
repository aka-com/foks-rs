/** Confirms and applies removal of a team member. */

import type { ReactNode } from 'react';
import { Button, Inset, Notice, SheetDialog } from '../../components';
import {
  entriesOf,
  exposureOrder,
  memberExposureReadiness,
  memberReads,
  withExposureReview,
} from '../../exposure-review';
import { groupDetailFailure, partyName } from '../../model';
import type { Party } from '../../model';
import { markProfileRostersStale } from '../../roster-staleness';
import { ExposureSummary, unstoredReview } from '../exposure-review';
import { GroupMark } from '../group-mark';
import { useToast } from '../../../kit/toasts';
import {
  canTarget,
  DetailUnavailable,
  PartyFactRow,
  useSheetWrite,
} from './sheet-support';
import type { GroupSheetBaseProps } from './sheet-support';

export function RemoveMemberSheet({
  snapshot,
  bridge,
  store,
  target,
  onClose,
  onApplied,
  onMutationError,
}: GroupSheetBaseProps & { target: Party | null }): ReactNode {
  const { busy, write } = useSheetWrite(onApplied, onClose);
  const toasts = useToast();
  const requiredFailure = groupDetailFailure(snapshot, store.id, 'roster');
  const name = target ? partyName(target) : 'them';
  const title = `Remove ${name} from ${store.name}?`;
  const actionable = target ? canTarget(snapshot, target) : false;
  // What the member could read is listed before the write and kept for review
  // after it; the list waits for the team's items and roster to be read.
  const readiness = memberExposureReadiness(snapshot, store.id);
  const exposed =
    target && readiness === 'ready'
      ? exposureOrder(memberReads(snapshot, target))
      : [];
  const apply = (): Promise<void> => {
    if (requiredFailure || !target || !actionable || readiness === 'loading')
      return Promise.resolve();
    const entries = entriesOf(exposed);
    return write(
      title,
      async () => {
        markProfileRostersStale(store.server);
        await withExposureReview(
          {
            scope: store.id,
            party: target.party_id_hex,
            name,
            cause: 'removed',
            entries,
          },
          () =>
            bridge.removeGroupMember({
              storeId: store.id,
              username: target.username!,
            }),
          () => toasts.show(unstoredReview(name)),
        );
        return { profile: store.server };
      },
      (error) => onMutationError(error),
    );
  };
  return (
    <SheetDialog
      danger
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
            danger
            disabled={
              busy ||
              Boolean(requiredFailure) ||
              !actionable ||
              readiness === 'loading'
            }
            onClick={() => void apply()}
          >
            Remove and rekey
          </Button>
        </>
      }
    >
      {requiredFailure ? <DetailUnavailable failure={requiredFailure} /> : null}
      <p>
        Removing blocks future reads and rekeys the team. This user may retain a
        local copy of their current records.
      </p>
      {target ? (
        <Inset>
          <PartyFactRow party={target} />
        </Inset>
      ) : null}
      {target && !actionable ? (
        <Notice title={`${name} cannot be removed here`}>
          {/* Explain why the selected member cannot be removed. */}
          <p>This member is managed by another server or account.</p>
        </Notice>
      ) : null}
      {target && actionable && !requiredFailure ? (
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
