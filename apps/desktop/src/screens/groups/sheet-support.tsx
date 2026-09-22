/** Shared types, helpers, and components for team-management sheets. */

import { useState } from 'react';
import type { ReactNode } from 'react';
import type { Bridge, RoleDto } from '../../bridge';
import { Button, Chip, InsetRow, Notice } from '../../components';
import {
  actionableGroupMember,
  isMachine,
  parseRole,
  partiesOf,
  partyName,
  roleChipLabel,
  roleRank,
} from '../../model';
import type {
  AgentSnapshot,
  GroupDetailFailure,
  Item,
  Party,
  Store,
  StoreRef,
} from '../../model';
import type { MutationFailureHandler } from '../../mutation-recovery';
import { synchronizeApplied } from '../../operation-outcome';
import { useToast } from '/kit/toasts';

export const VIS_MIN = -32768;
export const VIS_MAX = 32767;

/** Placeholder value for an empty team-name field. */
export const SUGGESTED_GROUP = 'Platform';

/**
 * Returns the server team name, or null if the name is invalid. This mirrors
 * `server_team_name` in foks-client-app: whitespace is collapsed to an
 * underscore, then the lowercase result must contain 3 to 25 letters, digits,
 * or single underscores after dots and dashes are replaced with underscores.
 */
export function serverTeamName(name: string): string | null {
  const folded = name.trim().split(/\s+/).filter(Boolean).join('_');
  const normalized = folded.toLowerCase().replace(/[.-]/g, '_');
  return /^[a-z0-9]+(?:_[a-z0-9]+)*$/.test(normalized) &&
    normalized.length >= 3 &&
    normalized.length <= 25
    ? folded
    : null;
}

/** Converts a display name to its local team alias. */
export function teamAliasOf(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');
}

export function canTarget(snapshot: AgentSnapshot, party: Party): boolean {
  if (!actionableGroupMember(snapshot, party)) return false;
  const parties = partiesOf(snapshot, party.store);
  const mine = parties.find((candidate) => candidate.label === 'you');
  if (!mine) return false;
  const myRank = roleRank(mine.destination_role);
  const targetRank = roleRank(party.destination_role);
  if (myRank < 2) return false;
  if (targetRank >= 3 && myRank < 3) return false;
  return true;
}

/** Returns the next lower role, or null when no lower role exists. */
export function demotionFor(party: Party): RoleDto | null {
  const role = parseRole(party.destination_role);
  if (!role) return null;
  if (role.kind === 'owner' || role.kind === 'admin')
    return { role: 'Member', visibility: 0 };
  const visibility = role.visibility ?? 0;
  return visibility > VIS_MIN
    ? { role: 'Member', visibility: visibility - 1 }
    : null;
}

/** Formats a role for display in team-management UI. */
export function fmtRole(role: Item['read']): string {
  const parsed = parseRole(role);
  return parsed
    ? roleChipLabel(parsed)
    : typeof role === 'string'
      ? role
      : role.role;
}

export function roleText(party: Party): string {
  return fmtRole(party.destination_role);
}

/** Displays a role and optional Member visibility level in one chip. */
export function RoleChip({
  role,
}: {
  role: Item['read'] | RoleDto | null | undefined;
}): ReactNode {
  const parsed = role ? parseRole(role) : null;
  return (
    <span className="rolecell">
      <Chip>
        {parsed
          ? roleChipLabel(parsed)
          : role
            ? typeof role === 'string'
              ? role
              : role.role
            : '—'}
      </Chip>
    </span>
  );
}

/** Returns the secondary label for a person or machine member row. */
export function partySubtitle(party: Party): string {
  return isMachine(party) ? 'machine' : 'person';
}

/** Displays the selected member and current role. */
export function PartyFactRow({
  party,
  caption,
}: {
  party: Party;
  caption?: string;
}): ReactNode {
  return (
    <InsetRow
      action={
        <>
          {caption ? <span className="dim">{caption}</span> : null}
          <RoleChip role={party.destination_role} />
        </>
      }
    >
      <span className="t">
        <b>{partyName(party)}</b>
        <small>{partySubtitle(party)}</small>
      </span>
    </InsetRow>
  );
}

/** Displays a visibility value with decrement and increment controls. */
export function VisibilityStepper({
  value,
  min = VIS_MIN,
  max = VIS_MAX,
  disabled = false,
  onChange,
}: {
  value: number;
  min?: number;
  max?: number;
  disabled?: boolean;
  onChange: (next: number) => void;
}): ReactNode {
  return (
    <>
      <Button
        size="sm"
        aria-label="Lower the visibility band"
        disabled={disabled || value <= min}
        onClick={() => onChange(value - 1)}
      >
        −
      </Button>
      <span className="vis-value">Visibility {value}</span>
      <Button
        size="sm"
        aria-label="Raise the visibility band"
        disabled={disabled || value >= max}
        onClick={() => onChange(value + 1)}
      >
        +
      </Button>
    </>
  );
}

/** Displays a roster or federation read failure. */
export function DetailUnavailable({
  failure,
}: {
  failure: GroupDetailFailure;
}): ReactNode {
  return (
    <Notice
      severity="warn"
      title={`${failure.source === 'roster' ? 'Roster' : 'Federation'} unavailable`}
    >
      <p>
        {failure.message} Close this sheet and refresh before making changes.
      </p>
    </Notice>
  );
}

/** Refresh information returned after a successful write. */
export interface AppliedOptions {
  created?: { accountStoreId: StoreRef; teamAlias: string };
  /**
   * Profile to refresh after the write. If absent, refresh the full catalog.
   */
  profile?: string;
}

/** Props shared by all team-management sheets. */
export interface GroupSheetBaseProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  store: Store;
  onClose: () => void;
  onApplied: (message: string, options?: AppliedOptions) => Promise<void>;
  onMutationError: MutationFailureHandler;
}

/** Runs a sheet write, refreshes affected state, and closes on success. */
export function useSheetWrite(
  onApplied: GroupSheetBaseProps['onApplied'],
  onClose: () => void,
) {
  const toasts = useToast();
  const [busy, setBusy] = useState(false);
  const write = async (
    title: string,
    work: () => Promise<AppliedOptions | undefined>,
    onFailure: (error: unknown) => Promise<void>,
  ): Promise<void> => {
    if (busy) return;
    setBusy(true);
    try {
      const options = await work();
      const result = await synchronizeApplied(() =>
        onApplied(`${title} completed`, options),
      );
      if (result.synchronization === 'pending')
        toasts.show(`${title} completed. Refresh pending.`);
      onClose();
    } catch (error) {
      await onFailure(error);
    } finally {
      setBusy(false);
    }
  };
  return { busy, write };
}
