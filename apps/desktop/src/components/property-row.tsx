/**
 * Form-style 38px property rows for details columns and pages, with a label
 * column, value column, and trailing actions in hairline-divided groups
 * (`.prop-*` in shell.css). Add `prop-scope` to the scrolling container so the
 * label column narrows (104px) and stacks based on the container width rather
 * than the window width.
 */

import type { ReactNode } from 'react';

export interface PropertyGroupProps {
  /** 12px muted heading displayed above the group (e.g. "Sharing"). */
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
  /** Trailing action buttons (e.g. Show, Copy). */
  actions?: ReactNode;
  /** Renders the value in a monospace font (for paths and identifiers). */
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
