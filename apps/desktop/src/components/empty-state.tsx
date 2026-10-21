/**
 * An empty or placeholder state centred in its column: an optional glyph,
 * a short title in the muted ink, one line of explanation, and the actions
 * that lead out of it. The details column's "No Selection" is the bare form.
 */

import type { ReactNode } from 'react';
import { Icon } from './icon';
import type { FoksIconName } from '../icons';

export interface EmptyStateProps {
  icon?: FoksIconName;
  title: ReactNode;
  body?: ReactNode;
  /** Buttons under the text. */
  children?: ReactNode;
  /** Top-aligned inside a list instead of centred in the column. */
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
