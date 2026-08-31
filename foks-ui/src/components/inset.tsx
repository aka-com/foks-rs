/**
 * The bordered card of labelled rows the design uses twice.
 *
 * `shell.css` draws it in two places with the same geometry and two class
 * names: `.inset > .fr` in a sheet's field block, and `.prev > .irow` in the
 * details panel's preview. One component covers both — the variant chooses
 * the class pair and tells its rows which one they are in through context, so
 * a row cannot end up wearing the other's class.
 */

import { createContext, useContext } from 'react';
import type { ReactNode } from 'react';

export type InsetVariant = 'field' | 'preview';

const ROW_CLASS: Readonly<Record<InsetVariant, string>> = {
  field: 'fr',
  preview: 'irow',
};

const InsetContext = createContext<InsetVariant>('field');

export interface InsetProps {
  variant?: InsetVariant;
  className?: string;
  /** Dimmed, non-interactive — the design's `.inset.off`. */
  off?: boolean;
  children: ReactNode;
}

export function Inset({
  variant = 'field',
  className,
  off = false,
  children,
}: InsetProps): ReactNode {
  const classes = [
    variant === 'field' ? 'inset' : 'prev',
    off ? 'off' : '',
    className ?? '',
  ]
    .filter(Boolean)
    .join(' ');
  return (
    <InsetContext.Provider value={variant}>
      <div className={classes}>{children}</div>
    </InsetContext.Provider>
  );
}

export interface InsetRowProps {
  /** The row's label — `.k`, the fixed-width first column. */
  label?: ReactNode;
  /** The value — `.v`. `valueClass` carries the design's `mono` / `mask`. */
  children?: ReactNode;
  valueClass?: string;
  /** Trailing controls — `.a`. */
  action?: ReactNode;
  className?: string;
}

export function InsetRow({
  label,
  children,
  valueClass,
  action,
  className,
}: InsetRowProps): ReactNode {
  const variant = useContext(InsetContext);
  const classes = [ROW_CLASS[variant], className ?? ''].filter(Boolean).join(' ');
  return (
    <div className={classes}>
      {label === undefined ? null : <span className="k">{label}</span>}
      {children === undefined ? null : (
        <span className={['v', valueClass ?? ''].filter(Boolean).join(' ')}>
          {children}
        </span>
      )}
      {action === undefined ? null : <span className="a">{action}</span>}
    </div>
  );
}
