/**
 * Button component supporting standard variants (primary, plain, danger, quiet)
 * and sizes (md, sm).
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
  /** A leading icon rendered before the button label. */
  icon?: FoksIconName;
  /**
   * Toggle pressed state. When defined, applies `.on` and `aria-pressed`.
   */
  on?: boolean;
  /** Optional ref passed to the underlying button element. */
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
