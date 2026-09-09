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
  /** The uppercase line above the heading — who and where. */
  eyebrow?: ReactNode;
  title: ReactNode;
  children: ReactNode;
  /** The quiet paragraph under the body. */
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
  /** The bold prefix — "Deferred", "Proposed". Optional; the mock's group
   *  band carries none and reads as a sentence. */
  label?: string;
  /** Hover text: which part of the plan defers this. */
  title?: string;
  /** `warn` is the amber default; `info` the quiet band; `crit` the danger treatment. */
  severity?: Severity;
  /** The one control the band's situation calls for, at its right edge. */
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
