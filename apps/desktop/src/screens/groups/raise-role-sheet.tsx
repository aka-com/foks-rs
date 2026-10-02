/** Grants a higher role while retaining the authenticated membership identity. */
import { useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import type { RoleDto } from '../../bridge';
import {
  Button,
  Inset,
  InsetRow,
  Notice,
  RadioCard,
  RadioGroup,
  SheetDialog,
} from '../../components';
import {
  groupDetailFailure,
  parseRole,
  partiesOf,
  partyName,
} from '../../model';
import type { Party } from '../../model';
import { markProfileRostersStale } from '../../roster-staleness';
import { GroupMark } from '../group-mark';
import {
  canTarget,
  DetailUnavailable,
  PartyFactRow,
  useSheetWrite,
  VisibilityStepper,
  VIS_MAX,
} from './sheet-support';
import type { GroupSheetBaseProps } from './sheet-support';

export function RaiseRoleSheet({
  snapshot,
  bridge,
  store,
  target,
  onClose,
  onApplied,
  onMutationError,
}: GroupSheetBaseProps & { target: Party | null }): ReactNode {
  const current = target ? parseRole(target.destination_role) : null;
  const mine = partiesOf(snapshot, store.id).find(
    (party) => party.label === 'you',
  );
  const owner = mine && parseRole(mine.destination_role)?.kind === 'owner';
  const initial = useMemo<RoleDto | null>(() => {
    const role = target ? parseRole(target.destination_role) : null;
    return role?.kind === 'member'
      ? { role: 'Admin' }
      : role?.kind === 'admin' && owner
        ? { role: 'Owner' }
        : null;
  }, [target, owner]);
  const [destination, setDestination] = useState<RoleDto | null>(initial);
  useEffect(() => {
    setDestination(initial);
  }, [initial]);
  const { busy, write } = useSheetWrite(onApplied, onClose);
  const failure = groupDetailFailure(snapshot, store.id, 'roster');
  const actionable = Boolean(target && canTarget(snapshot, target) && initial);
  const name = target ? partyName(target) : 'member';
  const title = `Raise ${name}’s role`;
  const minimum = (current?.visibility ?? 0) + 1;
  const apply = (): Promise<void> => {
    if (failure || !target || !actionable || !destination)
      return Promise.resolve();
    return write(
      title,
      async () => {
        markProfileRostersStale(store.server);
        await bridge.promoteGroupMember({
          storeId: store.id,
          username: target.username!,
          destination,
        });
        return { profile: store.server };
      },
      onMutationError,
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
            disabled={busy || Boolean(failure) || !actionable || !destination}
            onClick={() => void apply()}
          >
            Change role
          </Button>
        </>
      }
    >
      {failure ? <DetailUnavailable failure={failure} /> : null}
      {target ? (
        <Inset>
          <PartyFactRow party={target} caption="now" />
        </Inset>
      ) : null}
      {!actionable ? (
        <Notice title="No higher role is available">
          <p>Your current role cannot grant this member more access.</p>
        </Notice>
      ) : (
        <Inset>
          <RadioGroup label="New role">
            {owner ? (
              <RadioCard
                selected={destination?.role === 'Owner'}
                onSelect={() => setDestination({ role: 'Owner' })}
                title="Owner"
                detail="Full control of the team, including its owners."
              />
            ) : null}
            {current?.kind === 'member' ? (
              <RadioCard
                selected={destination?.role === 'Admin'}
                onSelect={() => setDestination({ role: 'Admin' })}
                title="Admin"
                detail="Access team items and manage members."
              />
            ) : null}
            {current?.kind === 'member' && minimum <= VIS_MAX ? (
              <RadioCard
                selected={destination?.role === 'Member'}
                onSelect={() =>
                  setDestination({ role: 'Member', visibility: minimum })
                }
                title="Higher member visibility"
                detail="Grant access to a higher member level without member-management privileges."
              />
            ) : null}
          </RadioGroup>
        </Inset>
      )}
      {destination?.role === 'Member' ? (
        <Inset>
          <InsetRow
            label="Visibility"
            action={
              <VisibilityStepper
                value={destination.visibility}
                min={minimum}
                onChange={(visibility) =>
                  setDestination({ role: 'Member', visibility })
                }
              />
            }
          />
        </Inset>
      ) : null}
      <p className="fn">
        The member keeps their identity and receives the keys for their new
        access level.
      </p>
    </SheetDialog>
  );
}
