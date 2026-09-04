/**
 * The design's button, in the four treatments `shell.css` draws.
 *
 * Every class here is one the design already styles — `.btn`, `.btn.primary`,
 * `.btn.danger`, `.btn.cap`, `.btn.icon`. Nothing invents a class name; if a
 * treatment is wanted that the sheet does not draw, the sheet is where it is
 * added.
 *
 *   variant  primary  `.btn.primary`  the one affirmative action on a surface
 *            plain    `.btn`          everything else
 *            danger   `.btn.danger`   destructive, red ink on hover wash
 *            quiet    `.btn.icon`     the square icon toggle in the toolbar,
 *                                     which carries an `on` state
 *   size     sm       `.btn.cap`      the 24px pill used by row affordances
 */

import type { ButtonHTMLAttributes, ReactNode, Ref } from 'react';
import { Icon } from './icon';
import type { FoksIconName } from '../icons';

export type ButtonVariant = 'primary' | 'plain' | 'danger' | 'quiet';
export type ButtonSize = 'md' | 'sm';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  danger?: boolean;
  size?: ButtonSize;
  /** A leading icon, rendered the way the mock's `ic()` places one. */
  icon?: FoksIconName;
  /**
   * A toggle's pressed state. Present means the button is a toggle: it gets
   * `.on` and `aria-pressed`, so a screen reader reads the state the colour
   * shows.
   */
  on?: boolean;
  /**
   * The element itself, for a menu that anchors to it. React 19 passes a ref
   * through as an ordinary prop, so there is no `forwardRef` here.
   */
  ref?: Ref<HTMLButtonElement>;
}

const VARIANT_CLASS: Readonly<Record<ButtonVariant, string>> = {
  primary: 'primary',
  plain: '',
  danger: 'danger',
  quiet: 'icon',
};

export function Button({
  variant = 'plain',
  size = 'md',
  danger,
  icon,
  on,
  className,
  children,
  type = 'button',
  ...rest
}: ButtonProps): ReactNode {
  const isDanger =
    danger || variant === 'danger' || className?.includes('danger');
  const variantClass = variant === 'danger' ? '' : VARIANT_CLASS[variant];
  const classes = [
    'btn',
    variantClass,
    isDanger ? 'danger' : '',
    size === 'sm' ? 'cap' : '',
    on ? 'on' : '',
    className ?? '',
  ]
    .filter(Boolean)
    .join(' ');
  return (
    <button
      {...rest}
      type={type}
      className={classes}
      aria-pressed={on === undefined ? undefined : on}
    >
      {icon ? <Icon name={icon} /> : null}
      {children}
    </button>
  );
}
