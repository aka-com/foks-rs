/**
 * The three small labels the design uses to say a fact without a sentence.
 *
 *   `.chip`   a computed fact — "3 people", "Owner", "you"
 *   `.tag`    a smaller, quieter classification
 *   `.badge`  a count on a navigation row
 *
 * `Badge` renders **nothing at zero**. The rule that an empty Issues badge
 * is absent rather than a "0" lives here, once, so no caller can forget it.
 */

import type { ReactNode } from 'react';

export type ChipTone = 'default' | 'you' | 'warn';

export interface ChipProps {
  tone?: ChipTone;
  /** Hover text — the roster behind a "3 people" count, for instance. */
  title?: string;
  className?: string;
  children: ReactNode;
}

export function Chip({
  tone = 'default',
  title,
  className,
  children,
}: ChipProps): ReactNode {
  const classes = ['chip', tone === 'default' ? '' : tone, className ?? '']
    .filter(Boolean)
    .join(' ');
  return (
    <span className={classes} title={title}>
      {children}
    </span>
  );
}

export interface TagProps {
  tone?: 'default' | 'warn';
  title?: string;
  children: ReactNode;
}

export function Tag({ tone = 'default', title, children }: TagProps): ReactNode {
  return (
    <span className={tone === 'warn' ? 'tag warn' : 'tag'} title={title}>
      {children}
    </span>
  );
}

export interface BadgeProps {
  count: number;
  /** What the count is of, for the screen reader. */
  label?: string;
}

export function Badge({ count, label }: BadgeProps): ReactNode {
  if (!count) return null;
  return (
    <span className="badge" title={label}>
      {count}
    </span>
  );
}
