/**
 * The eyebrow above a group of things.
 *
 * Two places, two classes, one meaning: the sidebar writes it as a real
 * `<h6>` (`shell.css`'s `.side h6`), and a panel writes it as `.sec`, which
 * can carry a trailing link. Both are uppercase, letterspaced and quiet; the
 * heading level is the part that matters to a screen reader, so the sidebar's
 * stays a heading rather than becoming a styled div.
 */

import type { ReactNode } from 'react';

export interface SectionLabelProps {
  /** `side` for the sidebar's `<h6>`, `panel` for a `.sec` eyebrow. */
  as?: 'side' | 'panel';
  /** A trailing control, right-aligned — the design's `.sec .lnk`. */
  action?: ReactNode;
  /** A modifier the sheet already draws, such as `danger-title`. */
  className?: string;
  children: ReactNode;
}

export function SectionLabel({
  as = 'panel',
  action,
  className,
  children,
}: SectionLabelProps): ReactNode {
  if (as === 'side') return <h6 className={className}>{children}</h6>;
  return (
    <div className={['sec', className ?? ''].filter(Boolean).join(' ')}>
      {children}
      {action}
    </div>
  );
}
