/**
 * The bordered card of labelled rows the design uses twice.
 *
 * `shell.css` draws it in two places with the same geometry and two class
 * names: `.inset > .fr` in a sheet's field block, and `.prev > .irow` in the
 * details panel's preview. One component covers both — the variant chooses
 * the class pair and tells its rows which one they are in through context, so
 * a row cannot end up wearing the other's class.
 */

import {
  Children,
  cloneElement,
  createContext,
  isValidElement,
  useContext,
  useId,
} from 'react';
import type { ReactElement, ReactNode } from 'react';

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

/** The elements a row's label should point at, if the row holds one. */
const CONTROL_TAGS = new Set(['input', 'textarea', 'select']);

/**
 * Give the row's first form control the label's `id`, so `.k` can be a real
 * `<label htmlFor>` rather than a `<span>` that only *looks* like one.
 *
 * The walk is shallow on purpose: every caller writes the control as a direct
 * child of `InsetRow`, and a row that nests one deeper than its own value
 * wrapper is not a labelled field. A control that already carries an `id` is
 * left alone — the caller has provided the intended identifier.
 */
function adoptControl(
  node: ReactNode,
  id: string,
  depth = 0,
): { node: ReactNode; found: boolean } {
  if (depth > 2 || !isValidElement(node)) return { node, found: false };
  const element = node as ReactElement<Record<string, unknown>>;
  if (typeof element.type === 'string' && CONTROL_TAGS.has(element.type)) {
    if (element.props.id !== undefined) return { node, found: true };
    return { node: cloneElement(element, { id }), found: true };
  }
  const children = element.props.children;
  if (children === undefined) return { node, found: false };
  let found = false;
  const mapped = Children.map(children as ReactNode, (child) => {
    if (found) return child;
    const result = adoptControl(child, id, depth + 1);
    found = result.found;
    return result.node;
  });
  if (!found) return { node, found: false };
  return { node: cloneElement(element, undefined, mapped), found: true };
}

export function InsetRow({
  label,
  children,
  valueClass,
  action,
  className,
}: InsetRowProps): ReactNode {
  const variant = useContext(InsetContext);
  const classes = [ROW_CLASS[variant], className ?? '']
    .filter(Boolean)
    .join(' ');
  const controlId = useId();

  let body = children;
  let labelled = false;
  if (children !== undefined && label !== undefined) {
    let found = false;
    body = Children.map(children, (child) => {
      if (found) return child;
      const result = adoptControl(child, controlId);
      found = result.found;
      return result.node;
    });
    labelled = found;
  }

  return (
    <div
      className={classes}
      onMouseDown={(event) => {
        const target = event.target;
        if (!(target instanceof Element)) return;
        if (target.closest('input, textarea, select, button, a, label')) return;
        event.currentTarget
          .querySelector<HTMLElement>('input, textarea, select')
          ?.focus();
      }}
    >
      {label === undefined ? null : labelled ? (
        <label className="k" htmlFor={controlId}>
          {label}
        </label>
      ) : (
        <span className="k">{label}</span>
      )}
      {body === undefined ? null : (
        <span className={['v', valueClass ?? ''].filter(Boolean).join(' ')}>
          {body}
        </span>
      )}
      {action === undefined ? null : <span className="a">{action}</span>}
    </div>
  );
}
