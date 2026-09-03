/**
 * People, as the design draws them — `shell.js`'s `avatar()` and `stack()`.
 *
 * A person is initials on a colour derived from their name (`hue`), so the
 * same person is the same colour on every screen and in every screenshot. A
 * party that is an admitted **group** is not a person: it gets the square
 * team glyph on `--c-team`, which is how a reader tells "5 people · 1 group"
 * apart at a glance.
 *
 * Nothing here decides *who* is shown. `readersOf` and `partiesOf` do that;
 * these draw what they answer.
 */

import type { ReactNode } from 'react';
import { Icon } from './icon';
import { hue, initials, partyName } from '../model';
import type { Party } from '../model';

export interface AvatarProps {
  party: Party;
  /** The base class: `av` in a stack, `pav` in the details panel's roster. */
  className?: string;
}

export function Avatar({ party, className = 'av' }: AvatarProps): ReactNode {
  if (party.party_kind !== 'user') {
    return (
      <span
        className={`${className} team`}
        style={{ background: 'var(--c-team)' }}
        title={partyName(party)}
      >
        <Icon name="people" />
      </span>
    );
  }
  const name = partyName(party);
  return (
    <span className={className} style={{ background: hue(name) }} title={name}>
      {initials(name)}
    </span>
  );
}

export type StackSize = 'md' | 'lg' | 'xs';

export interface StackProps {
  /** The store's roster. The first two are drawn, as the mock draws them. */
  parties: readonly Party[];
  size?: StackSize;
  /** Hover text for the whole stack — usually the roster summary. */
  title?: string;
}

export function Stack({ parties, size = 'md', title }: StackProps): ReactNode {
  const classes = ['stack', size === 'md' ? '' : size]
    .filter(Boolean)
    .join(' ');
  return (
    <span className={classes} title={title}>
      {parties.length ? (
        parties
          .slice(0, 2)
          .map((party) => <Avatar key={party.party_id_hex} party={party} />)
      ) : (
        // No roster at all — an account store, or a group with nobody in it
        // yet. The design gives it the neutral glyph rather than a gap.
        <span className="av" style={{ background: 'var(--c-none)' }}>
          <Icon name="people" />
        </span>
      )}
    </span>
  );
}
