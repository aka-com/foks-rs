/**
 * Form-style property rows for a details column or a page: a label column,
 * the value, and trailing actions, 38px high, in groups divided by a
 * hairline (`.prop-*` in shell.css). Put `prop-scope` on the scrolling
 * column so the label column narrows (104px) and finally stacks with the
 * column's own width rather than the window's.
 */

import type { ReactNode } from 'react';

export interface PropertyGroupProps {
  /** A 12px muted label over the group ("Sharing"). */
  label?: ReactNode;
  className?: string;
  children: ReactNode;
}

export function PropertyGroup({
  label,
  className,
  children,
}: PropertyGroupProps): ReactNode {
  return (
    <div className={['prop-group', className ?? ''].filter(Boolean).join(' ')}>
      {label ? <div className="prop-label">{label}</div> : null}
      {children}
    </div>
  );
}

export interface PropertyRowProps {
  label: ReactNode;
  /** Buttons at the trailing edge (Show, Copy). */
  actions?: ReactNode;
  /** Draws the value in the monospace face (paths, identifiers). */
  mono?: boolean;
  className?: string;
  children?: ReactNode;
}

export function PropertyRow({
  label,
  actions,
  mono = false,
  className,
  children,
}: PropertyRowProps): ReactNode {
  return (
    <div className={['prop-row', className ?? ''].filter(Boolean).join(' ')}>
      <span className="k">{label}</span>
      <span className={mono ? 'v mono' : 'v'}>{children}</span>
      <span className="a">{actions}</span>
    </div>
  );
}
