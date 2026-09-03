/**
 * The design's `.radio` — one choice out of a few, each with a reason.
 *
 * `shell.css` draws this control completely (the dot, the `on` fill, the
 * dimmed `off` state, the `store-choice` variant), but it had never become a
 * component, so nine call sites across three screens wrote the button, the
 * `.rb` dot and the `.t` title/description block out by hand. They also
 * disagreed about what to tell a screen reader: some set `aria-pressed`, most
 * set nothing, so a listener could not hear which store or role was chosen.
 *
 * `RadioCard` is a real radio — `role="radio"` and `aria-checked` — and
 * `RadioGroup` is the `radiogroup` around a set of them, which is what makes
 * the choice legible as a choice rather than as a row of unrelated buttons.
 */

import type { ReactNode } from 'react';

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
  onSelect?: () => void;
  disabled?: boolean;
  /** Dimmed and inert — the design's `.radio.off`. */
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
      disabled={disabled || off}
      onClick={onSelect}
    >
      <span className="rb" />
      <span className="t">
        <b>{title}</b>
        {detail === undefined ? null : <small>{detail}</small>}
      </span>
      {tail}
    </button>
  );
}
