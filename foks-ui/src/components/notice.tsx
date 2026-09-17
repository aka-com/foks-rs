/**
 * The two ways the design says "something outranks what you were doing".
 *
 * `Notice` is the full-pane panel that replaces a list: the lapsed check-in,
 * the group that reports itself inactive. It takes the model's own
 * `Severity`, so the severity a notification carries is the severity drawn —
 * `shell.css` gives `info` and `warn` the one amber panel and `crit` the
 * `.stop` treatment, and no fourth colour is invented here.
 *
 * `Band` is the one-line strip above a list — for example Proposed on setup,
 * Proposed in the first run. It is an aside, never a replacement: the list
 * below it still lists.
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
    <div className={severity === 'crit' ? 'notice stop' : 'notice'}>
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
  children: ReactNode;
}

export function Band({ label, title, children }: BandProps): ReactNode {
  return (
    <div className="band" title={title}>
      <Icon name="alert" />
      <span>
        {label ? <b>{label}</b> : null}
        {label ? ' ' : null}
        {children}
      </span>
    </div>
  );
}
