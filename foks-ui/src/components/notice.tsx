/**
 * The two ways the design says "something outranks what you were doing".
 *
 * `Notice` is the full-pane panel that replaces a list: the lapsed check-in,
 * the group that reports itself inactive. It takes the model's own
 * `Severity`, so the severity a notification carries is the severity drawn:
 * `info` is a neutral panel, `warn` the amber one and `crit` the `.stop`
 * treatment. No fourth colour is invented here.
 *
 * `Band` is the compact strip beside or above what it qualifies — Proposed on
 * setup, Proposed in the first run, or the reason a section's controls are
 * inert. It is an aside, never a replacement: what it sits with still shows.
 * It takes the same `Severity` as `Notice`, so an aside about a stopped
 * server is drawn in the same red as the panel about one, and nothing has to
 * reach for the full-pane panel just to be the right colour.
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
