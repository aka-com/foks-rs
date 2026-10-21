/** Status labels: the design's `.chip`. */

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
