/** Accessible card-styled radio button and radio group components. */

import type { ReactNode } from 'react';
import { Icon } from './icon';
import type { FoksIconName } from '../icons';

export interface RadioGroupProps {
  /** What is being chosen, for the screen reader. */
  label: string;
  className?: string;
  children: ReactNode;
}

export function RadioGroup({
  label,
  className,
  children,
}: RadioGroupProps): ReactNode {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={['radios', className].filter(Boolean).join(' ')}
    >
      {children}
    </div>
  );
}

export interface RadioCardProps {
  /** The bold first line. */
  title: ReactNode;
  /** Supporting text under the label. */
  detail?: ReactNode;
  selected: boolean;
  /** A glyph before the title, saying what kind of thing the card is. */
  icon?: FoksIconName;
  onSelect?: () => void;
  disabled?: boolean;
  /**
   * Dimmed and inert — the design's `.radio.off`. The card keeps its place
   * and does nothing: like a menu item that does not apply, it is marked
   * `aria-disabled` rather than `disabled`, so the keyboard reaches it and
   * reads why. `detail` is that reason, and carries it into the `title` as
   * well, because WKWebView suppresses hover on a natively disabled control.
   */
  off?: boolean;
  /** A trailing element, such as the store roster on a `store-choice`. */
  tail?: ReactNode;
  /** A modifier the sheet draws, such as `store-choice`. */
  className?: string;
}

export function RadioCard({
  title,
  detail,
  selected,
  icon,
  onSelect,
  disabled = false,
  off = false,
  tail,
  className,
}: RadioCardProps): ReactNode {
  const classes = [
    'radio',
    className ?? '',
    selected ? 'on' : '',
    off ? 'off' : '',
  ]
    .filter(Boolean)
    .join(' ');
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      className={classes}
      disabled={disabled}
      aria-disabled={off ? true : undefined}
      tabIndex={off ? 0 : undefined}
      title={off && typeof detail === 'string' ? detail : undefined}
      onClick={off ? undefined : onSelect}
    >
      <span className="rb" />
      {icon ? (
        <span className="rico" aria-hidden="true">
          <Icon name={icon} />
        </span>
      ) : null}
      <span className="t">
        <b>{title}</b>
        {detail === undefined ? null : <small>{detail}</small>}
      </span>
      {tail}
    </button>
  );
}
