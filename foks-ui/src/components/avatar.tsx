/**
 * Avatar and avatar stack components.
 *
 * Renders user initials on a deterministic hue derived from their name, or a
 * team glyph for group parties.
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
  /** The store's roster. Renders the first two avatars. */
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
        // Render neutral placeholder icon when the roster is empty.
        <span className="av" style={{ background: 'var(--c-none)' }}>
          <Icon name="people" />
        </span>
      )}
    </span>
  );
}
