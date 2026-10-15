/**
 * A party (a person, a machine or a member team) as one row: its name, a
 * "you" chip on the reader's own party, a caption, and the role it holds.
 */

import type { ReactNode } from 'react';
import { partyName, roleChipText, roleText } from '../model';
import type { Party, RoleWire } from '../model';
import { Chip } from './chips';

/** Displays a role and optional Member visibility level in one chip. */
export function RoleChip({
  role,
}: {
  role: RoleWire | null | undefined;
}): ReactNode {
  return (
    <span className="rolecell">
      <Chip>{role ? roleChipText(role) : '—'}</Chip>
    </span>
  );
}

/**
 * `roster` is a row of a team's member list. `listing` is a row of the
 * details panel's Sharing list, which writes the role as `roleText` does, so
 * it reads the same as the item's access threshold above the list.
 */
export type PartyRowVariant = 'roster' | 'listing';

export function PartyRow({
  party,
  variant,
  mark,
  caption,
  actions,
}: {
  party: Party;
  variant: PartyRowVariant;
  /** The mark before the name. Drawn by `roster` only. */
  mark?: ReactNode;
  /** The line under the name; omitted when empty. */
  caption?: string;
  /** Controls after the role chip, such as an actions menu. `roster` only. */
  actions?: ReactNode;
}): ReactNode {
  const name = partyName(party);
  const you = party.label ? <Chip tone="you">you</Chip> : null;
  const small = caption ? <small>{caption}</small> : null;
  if (variant === 'listing') {
    return (
      <div className="party">
        <span className="t">
          {name}
          {you ? <> {you}</> : null}
          {small}
        </span>
        <Chip>{roleText(party.destination_role)}</Chip>
      </div>
    );
  }
  return (
    <div className="prow">
      <span className="who2">
        {mark}
        <span className="t">
          <b>
            <span>{name}</span>
            {you}
          </b>
          {small}
        </span>
      </span>
      <span className="rowtail">
        <RoleChip role={party.destination_role} />
        {actions}
      </span>
    </div>
  );
}
