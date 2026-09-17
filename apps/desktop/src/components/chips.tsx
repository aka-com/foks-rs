/** Status labels, classification tags, and optional numeric count badges. */

import type { ReactNode } from 'react';

/** Visual tone for status chips. */
export type ChipTone = 'default' | 'you' | 'warn' | 'ok' | 'bad';

export interface ChipProps {
  tone?: ChipTone;
  /** Optional hover text describing the status. */
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

export function Tag({
  tone = 'default',
  title,
  children,
}: TagProps): ReactNode {
  return (
    <span className={tone === 'warn' ? 'tag warn' : 'tag'} title={title}>
      {children}
    </span>
  );
}

export interface BadgeProps {
  count: number;
  /** Accessible label describing the counted entity. */
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
