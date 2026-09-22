/** Lowers a member's role or Member visibility level. */

import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import type { RoleDto } from '../../bridge';
import {
  Button,
  Inset,
  InsetRow,
  Notice,
  RadioCard,
  RadioGroup,
  SectionLabel,
  SheetDialog,
} from '../../components';
import { groupDetailFailure, parseRole, partyName } from '../../model';
import type { Party } from '../../model';
import { markProfileRostersStale } from '../../roster-staleness';
import { GroupMark } from '../group-mark';
import {
  canTarget,
  demotionFor,
  DetailUnavailable,
  PartyFactRow,
  useSheetWrite,
  VIS_MIN,
  VisibilityStepper,
} from './sheet-support';
import type { GroupSheetBaseProps } from './sheet-support';

export function LowerRoleSheet({
  snapshot,
  bridge,
  store,
  target,
  onClose,
  onApplied,
  onMutationError,
}: GroupSheetBaseProps & { target: Party | null }): ReactNode {
  const [demotion, setDemotion] = useState<RoleDto | null>(() =>
    target ? demotionFor(target) : null,
  );
  useEffect(() => {
    setDemotion(target ? demotionFor(target) : null);
  }, [target]);
  const { busy, write } = useSheetWrite(onApplied, onClose);
  const requiredFailure = groupDetailFailure(snapshot, store.id, 'roster');
  const name = target ? partyName(target) : 'them';
  const title = `Lower ${target ? `${name}’s` : 'their'} role`;
  const currentRole = target ? parseRole(target.destination_role) : null;
  // Members can only move to a lower visibility level. Admins and Owners may
  // move to any Member visibility level.
  const maxMemberVisibility =
    currentRole?.kind === 'member' ? (currentRole.visibility ?? 0) - 1 : 0;
  const lowerable = target ? demotionFor(target) !== null : false;
  const actionable = target ? canTarget(snapshot, target) : false;
  const apply = (): Promise<void> => {
    if (requiredFailure || !target || !actionable || !demotion)
      return Promise.resolve();
    return write(
      title,
      async () => {
        markProfileRostersStale(store.server);
        await bridge.demoteGroupMember({
          storeId: store.id,
          username: target.username!,
          destination: demotion,
        });
        return { profile: store.server };
      },
      (error) => onMutationError(error),
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
            disabled={
              busy ||
              Boolean(requiredFailure) ||
              !target ||
              !actionable ||
              !demotion
            }
            onClick={() => void apply()}
          >
            Change role
          </Button>
        </>
      }
    >
      {requiredFailure ? <DetailUnavailable failure={requiredFailure} /> : null}
      {target ? (
        <Inset>
          <PartyFactRow party={target} caption="now" />
        </Inset>
      ) : null}
      {target && !actionable ? (
        <Notice title={`${name}’s role cannot be changed here`}>
          <p>This member is managed by another server or account.</p>
        </Notice>
      ) : null}
      <SectionLabel>New role</SectionLabel>
      {lowerable ? (
        <Inset>
          <RadioGroup label="New role">
            {currentRole?.kind === 'owner' ? (
              <RadioCard
                selected={demotion?.role === 'Admin'}
                onSelect={() => setDemotion({ role: 'Admin' })}
                title="Admin"
                detail="Full access to team items and permission to manage members."
              />
            ) : null}
            <RadioCard
              selected={demotion?.role === 'Member'}
              disabled={maxMemberVisibility < VIS_MIN}
              onSelect={() =>
                setDemotion({ role: 'Member', visibility: maxMemberVisibility })
              }
              title={
                currentRole?.kind === 'member'
                  ? `Member (${demotion?.role === 'Member' ? demotion.visibility : maxMemberVisibility})`
                  : 'Member'
              }
              detail={
                currentRole?.kind === 'member'
                  ? `Same role, lower level; ${maxMemberVisibility} at most.`
                  : 'Cannot manage members; can only access items allowed by their member role.'
              }
            />
          </RadioGroup>
        </Inset>
      ) : (
        <Notice title={`${name} is already at the lowest level`}>
          <p>
            A Member at visibility {VIS_MIN} cannot be lowered further. To take
            access away, remove {name} from {store.name}.
          </p>
        </Notice>
      )}
      {demotion?.role === 'Member' ? (
        <Inset className="visibility-inset">
          <InsetRow
            label="Visibility"
            action={
              <VisibilityStepper
                value={demotion.visibility}
                max={maxMemberVisibility}
                onChange={(visibility) =>
                  setDemotion({ role: 'Member', visibility })
                }
              />
            }
          />
        </Inset>
      ) : null}
      <p className="fn">
        Roles can only be lowered here. To grant {name} a higher role, remove
        them and add them again with the new role.
      </p>
    </SheetDialog>
  );
}
