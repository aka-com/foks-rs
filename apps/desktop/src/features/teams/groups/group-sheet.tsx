/** The team-management sheet a group page or the Teams list opens. */

import { AddPersonSheet } from './add-person-sheet';
import { AddTeamSheet } from './add-team-sheet';
import { CreateTeamSheet } from './create-team-sheet';
import { RaiseRoleSheet } from './raise-role-sheet';
import { LowerRoleSheet } from './lower-role-sheet';
import { RemoveMemberSheet } from './remove-member-sheet';
import type { GroupSheetBaseProps } from './sheet-support';
import type { ReactNode } from 'react';
import type { Party } from '../../../model';

/** The team-management actions that open a sheet. */
export type GroupSheetKind =
  'add' | 'promote' | 'demote' | 'remove' | 'add-team' | 'create';

/** Selects the sheet component for a team-management action. */
export function GroupSheet({
  sheet,
  target,
  onInvite,
  ...base
}: GroupSheetBaseProps & {
  sheet: GroupSheetKind;
  target: Party | null;
  /** Starts the invitation flow for someone without a FOKS account. */
  onInvite?: () => void;
}): ReactNode {
  switch (sheet) {
    case 'add':
      return <AddPersonSheet {...base} onInvite={onInvite} />;
    case 'add-team':
      return <AddTeamSheet {...base} />;
    case 'promote':
      return <RaiseRoleSheet {...base} target={target} />;
    case 'demote':
      return <LowerRoleSheet {...base} target={target} />;
    case 'remove':
      return <RemoveMemberSheet {...base} target={target} />;
    case 'create':
      return <CreateTeamSheet {...base} />;
  }
}
