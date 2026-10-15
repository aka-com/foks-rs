/**
 * The mark an account carries wherever it is named: the sidebar, the account
 * switcher, the store marks of the Files list and details panel, and chat.
 */

import type { ReactNode } from 'react';
import { hue } from '../model';

/**
 * An account's mark: the initial of the username, over a color derived from
 * it. These surfaces name the account by the username the server knows it by,
 * so the mark stands for that name; a store elsewhere keeps `GroupMark`.
 */
export function AccountMark({
  name,
  size = 'sm',
  className,
}: {
  /** The username, or the local alias when the catalog has no username. */
  name: string;
  /**
   * `sm` is the switcher's 26px mark, `round` the Account header's 30px circle,
   * `big` the 48px one a card draws.
   */
  size?: 'sm' | 'md' | 'round' | 'big';
  className?: string;
}): ReactNode {
  return (
    <span
      className={[
        'kico',
        size === 'sm' ? '' : size,
        'group',
        'account',
        className,
      ]
        .filter(Boolean)
        .join(' ')}
      // The initial stands for the name beside it, which is always drawn.
      aria-hidden="true"
      style={{ background: hue(name) }}
    >
      {name.slice(0, 1).toUpperCase()}
    </span>
  );
}
