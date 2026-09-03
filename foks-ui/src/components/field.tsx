/**
 * A labelled text field inside an `Inset`.
 *
 * `InsetRow` plus a controlled `<input>` is how every sheet in FOKS asks for
 * a value — a username, an alias, a path, a confirmation, a passphrase. The
 * write workflows had grown a local `field()` helper for it; everyone else
 * wrote the pair out by hand, which is why the `mono` treatment was applied
 * to the row on some call sites and to the input on others.
 *
 * The label is a real `<label>` — `InsetRow` gives its first control the
 * label's id — and the input keeps a matching `aria-label`. The two carry the
 * same string, so the accessible name is the same either way. The input fills
 * the value column, and a click anywhere else on the row (padding, empty
 * value space) focuses it, so the hit target is the whole row.
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
  /** A hex id, a slot number, a path — anything read character by character. */
  mono?: boolean;
  min?: number;
  max?: number;
  /** Trailing controls in the row's `.a` slot. */
  action?: ReactNode;
  /** A quiet note after the input. */
  hint?: ReactNode;
}

export function Field({
  label,
  value,
  onChange,
  type = 'text',
  placeholder,
  mono = false,
  min,
  max,
  action,
  hint,
}: FieldProps): ReactNode {
  return (
    <InsetRow
      label={label}
      valueClass={mono ? 'mono' : undefined}
      action={action}
    >
      <input
        type={type}
        aria-label={label}
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
