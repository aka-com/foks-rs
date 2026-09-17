/**
 * Alert notices and banner strips.
 *
 * `Notice` renders an inset message container. `Band` renders a compact
 * full-width banner. Both support `info`, `warn`, and `crit` severities.
 */

import type { ReactNode } from 'react';
import { Icon } from './icon';
import type { Severity } from '../model';

export interface NoticeProps {
  severity?: Severity;
  /** Optional uppercase line above the heading. */
  eyebrow?: ReactNode;
  title: ReactNode;
  children: ReactNode;
  /** Optional paragraph under the body. */
  footnote?: ReactNode;
  /** The row of controls at the bottom. */
  actions?: ReactNode;
}

export function Notice({
  severity = 'warn',
  eyebrow,
  title,
  children,
  footnote,
  actions,
}: NoticeProps): ReactNode {
  return (
    <div
      className={
        severity === 'crit'
          ? 'notice stop'
          : severity === 'info'
            ? 'notice info'
            : 'notice'
      }
    >
      {eyebrow === undefined ? null : <div className="who">{eyebrow}</div>}
      <h2>{title}</h2>
      {children}
      {footnote === undefined ? null : <p className="fn">{footnote}</p>}
      {actions === undefined ? null : <div className="acts2">{actions}</div>}
    </div>
  );
}

export interface BandProps {
  /** Optional bold prefix such as "Deferred" or "Proposed". The mock group
   *  band omits it and uses a complete sentence. */
  label?: string;
  /** Tooltip describing what is deferred. */
  title?: string;
  /** `warn` is the amber default; `info` the quiet band; `crit` the danger treatment. */
  severity?: Severity;
  /** Optional control associated with the band's message, displayed at the right edge. */
  action?: ReactNode;
  children: ReactNode;
}

export function Band({
  label,
  title,
  severity = 'warn',
  action,
  children,
}: BandProps): ReactNode {
  return (
    <div
      className={
        severity === 'crit'
          ? 'band stop'
          : severity === 'info'
            ? 'band info'
            : 'band'
      }
      title={title}
    >
      <Icon name={severity === 'info' ? 'info' : 'alert'} />
      <span className="t">
        {label ? <b>{label}</b> : null}
        {label ? ' ' : null}
        {children}
      </span>
      {action === undefined ? null : <span className="a">{action}</span>}
    </div>
  );
}
