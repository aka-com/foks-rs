/** Header label for content sections with an optional trailing action. */

import type { ReactNode } from 'react';

export interface SectionLabelProps {
  /** `side` for the sidebar's `<h6>`, `panel` for a `.sec` eyebrow. */
  as?: 'side' | 'panel';
  /** A trailing control, right-aligned — the design's `.sec .lnk`. */
  action?: ReactNode;
  /** A modifier the sheet already draws, such as `danger-title`. */
  className?: string;
  /**
   * Names the label's own text, for a `region` around the section that points
   * at it with `aria-labelledby`. It sits on the text alone, so a trailing
   * action is not read as part of the section's name.
   */
  id?: string;
  children: ReactNode;
}

export function SectionLabel({
  as = 'panel',
  action,
  className,
  id,
  children,
}: SectionLabelProps): ReactNode {
  const text = id ? <span id={id}>{children}</span> : children;
  if (as === 'side') return <h6 className={className}>{text}</h6>;
  return (
    <div className={['sec', className ?? ''].filter(Boolean).join(' ')}>
      {text}
      {action ? <span className="right">{action}</span> : null}
    </div>
  );
}
