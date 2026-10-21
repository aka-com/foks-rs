/**
 * Form field row styled for use within an Inset container.
 *
 * Supports text, masked password, number, monospace, trailing action, and
 * descriptive hint variants.
 */

import { useId } from 'react';
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
  /** Takes the focus when its dialog opens (`data-dialog-autofocus`). */
  autoFocus?: boolean;
  /**
   * Why the value was refused. The input is marked `aria-invalid` and
   * described by the message, which is drawn under it as an alert; the focus
   * keeper lands a dialog's lost focus on the invalid field.
   */
  error?: string;
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
  autoFocus = false,
  error,
}: FieldProps): ReactNode {
  const errorId = useId();
  return (
    <InsetRow
      label={label}
      valueClass={mono ? 'mono' : undefined}
      action={action}
    >
      <input
        type={type}
        disabled={disabled}
        aria-label={label}
        data-dialog-autofocus={autoFocus ? 'true' : undefined}
        aria-invalid={error ? true : undefined}
        aria-describedby={error ? errorId : undefined}
        className={mono ? 'mono' : undefined}
        placeholder={placeholder}
        min={min}
        max={max}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
      {hint === undefined ? null : <small>{hint}</small>}
      {error ? (
        <small id={errorId} className="field-error" role="alert">
          {error}
        </small>
      ) : null}
    </InsetRow>
  );
}
