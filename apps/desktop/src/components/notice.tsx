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
  /** The band's heading, such as "Roster unavailable". The mock group band
   *  omits it and uses a complete sentence. */
  label?: string;
  /** Tooltip describing what is deferred. */
  title?: string;
  /** `warn` is the amber default; `info` the quiet band; `crit` the danger treatment. */
  severity?: Severity;
  /**
   * Announce the band, for one mounted in response to an action. A band the
   * page always draws is left silent: it would otherwise speak on every mount,
   * saying a condition the reader did not act on.
   */
  live?: boolean;
  /** Optional control associated with the band's message, displayed at the right edge. */
  action?: ReactNode;
  children: ReactNode;
}

export function Band({
  label,
  title,
  severity = 'warn',
  live = false,
  action,
  children,
}: BandProps): ReactNode {
  // A band mounted in response to an action is announced — a critical one
  // interrupts, the rest are polite. One the page always draws is a labeled
  // group instead, reachable by its label without announcing itself.
  const role = live
    ? severity === 'crit'
      ? 'alert'
      : 'status'
    : label
      ? 'group'
      : undefined;
  return (
    <div
      className={
        severity === 'crit'
          ? 'band stop'
          : severity === 'info'
            ? 'band info'
            : 'band'
      }
      role={role}
      // The label names the group; a live band already reads its own text, so
      // naming it again would say the label twice.
      aria-label={role === 'group' ? label : undefined}
      title={title}
    >
      <Icon name={severity === 'info' ? 'info' : 'alert'} />
      <span className="t">
        {/* A heading cannot nest in this line; the label is the bold lead-in
            it has always been drawn as. */}
        {label ? <b>{label}</b> : null}
        {label ? ' ' : null}
        {children}
      </span>
      {action === undefined ? null : <span className="a">{action}</span>}
    </div>
  );
}
