/** Header label for content sections with an optional trailing action. */

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
      {action ? <span className="right">{action}</span> : null}
    </div>
  );
}
