/**
 * Form field row styled for use within an Inset container.
 *
 * Supports text, masked password, number, monospace, trailing action, and
 * descriptive hint variants.
 */

import type { ReactNode } from 'react';
import { InsetRow } from './inset';

export interface FieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  /** `password` for a secret, `number` for a bounded count. */
  type?: 'text' | 'password' | 'number';
  placeholder?: string;
  disabled?: boolean;
  /** A hex id, a slot number, a path — anything read character by character. */
  mono?: boolean;
  min?: number;
  max?: number;
  /** Trailing controls in the row's `.a` slot. */
  action?: ReactNode;
  /** A quiet note after the input. */
  hint?: ReactNode;
  /** Extra classes on the row. */
  className?: string;
  /** Ids of elements that describe the input. */
  describedBy?: string;
  /**
   * Entered exactly, as a password is: no spelling correction,
   * capitalization or autofill, also while it is shown as text.
   */
  verbatim?: boolean;
}

export function Field({
  label,
  value,
  onChange,
  type = 'text',
  placeholder,
  disabled = false,
  mono = false,
  min,
  max,
  action,
  hint,
  className,
  describedBy,
  verbatim = false,
}: FieldProps): ReactNode {
  return (
    <InsetRow
      label={label}
      valueClass={mono ? 'mono' : undefined}
      action={action}
      className={className}
    >
      <input
        type={type}
        disabled={disabled}
        aria-label={label}
        aria-describedby={describedBy}
        spellCheck={verbatim ? false : undefined}
        autoCorrect={verbatim ? 'off' : undefined}
        autoCapitalize={verbatim ? 'off' : undefined}
        autoComplete={verbatim ? 'off' : undefined}
        className={mono ? 'mono' : undefined}
        placeholder={placeholder}
        min={min}
        max={max}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
      {hint === undefined ? null : <small>{hint}</small>}
    </InsetRow>
  );
}
