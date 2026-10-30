/**
 * Centered empty or placeholder state within a column: optional icon, muted
 * title, single-line description, and action buttons. The details column's
 * "No Selection" state uses the minimal title-only form.
 */

import type { ReactNode } from 'react';
import { Icon } from './icon';
import type { FoksIconName } from '../icons';

export interface EmptyStateProps {
  icon?: FoksIconName;
  title: ReactNode;
  body?: ReactNode;
  /** Action buttons rendered below the description. */
  children?: ReactNode;
  /** Top-aligns the state inside a list instead of centering in the column. */
  inList?: boolean;
  className?: string;
}

export function EmptyState({
  icon,
  title,
  body,
  children,
  inList = false,
  className,
}: EmptyStateProps): ReactNode {
  return (
    <div
      className={['empty', inList ? 'in-list' : '', className ?? '']
        .filter(Boolean)
        .join(' ')}
    >
      {icon ? (
        <div className="big">
          <Icon name={icon} />
        </div>
      ) : null}
      <h2>{title}</h2>
      {body ? <p>{body}</p> : null}
      {children ? <div className="empty-acts">{children}</div> : null}
    </div>
  );
}
