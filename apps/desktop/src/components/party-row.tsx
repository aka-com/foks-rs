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

interface PartyRowBase {
  party: Party;
  /** The name to draw; `partyName(party)` when absent. */
  name?: string;
  /** The line under the name; omitted when empty. */
  caption?: string;
}

/**
 * `roster` is a row of a team's member list. `listing` is a row of the
 * details panel's Sharing list, which writes the role as `roleText` does, so
 * it reads the same as the item's access threshold above the list. Only
 * `roster` draws a mark and controls, so only its props declare them.
 */
export type PartyRowProps =
  | (PartyRowBase & { variant: 'listing' })
  | (PartyRowBase & {
      variant: 'roster';
      /** The mark before the name. */
      mark?: ReactNode;
      /** Controls after the role chip, such as an actions menu. */
      actions?: ReactNode;
    });

export type PartyRowVariant = PartyRowProps['variant'];

export function PartyRow(props: PartyRowProps): ReactNode {
  const { party, caption } = props;
  const name = props.name ?? partyName(party);
  const you = party.label ? <Chip tone="you">you</Chip> : null;
  const small = caption ? <small>{caption}</small> : null;
  if (props.variant === 'listing') {
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
        {props.mark}
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
        {props.actions}
      </span>
    </div>
  );
}
