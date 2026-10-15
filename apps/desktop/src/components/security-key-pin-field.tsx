/**
 * Security-key PIN or PUK row for an Inset container.
 *
 * Every prompt for a card credential uses this row so that each one applies
 * the same rule as `foks_yubi::Pin::new` (see `security-key-pin.ts`): a value
 * the card would refuse is named under the input, and autofill and text
 * correction are off. The input has no `maxLength`: a pasted value cut to
 * eight characters would be submitted as a different PIN, which costs an
 * attempt or, for a new PIN, sets one the reader did not type. While the
 * prompt holds a refusal from the agent, the row also states how many
 * attempts remain, or that the PIN is blocked, so the next submission is made
 * knowing that.
 */

import { useId } from 'react';
import type { ReactNode } from 'react';
import { InsetRow } from './inset';
import {
  securityKeyPinProblem,
  securityKeyPinRefusal,
  securityKeyPinRefusalText,
} from '../security-key-pin';

export interface SecurityKeyPinFieldProps {
  /** `Security key PIN` unless the prompt names the credential otherwise. */
  label?: string;
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  placeholder?: string;
  title?: string;
  /**
   * The failure of the last submission, or the message the prompt shows for
   * it. A PIN refusal in it is stated under the input.
   */
  refusal?: unknown;
}

export function SecurityKeyPinField({
  label = 'Security key PIN',
  value,
  onChange,
  disabled = false,
  placeholder,
  title,
  refusal,
}: SecurityKeyPinFieldProps): ReactNode {
  const problemId = useId();
  const refusalId = useId();
  const problem = securityKeyPinProblem(value);
  const refused = securityKeyPinRefusal(refusal);
  const tone =
    refused?.kind === 'blocked'
      ? 'pin-note crit'
      : refused?.remaining === 1
        ? 'pin-note warn'
        : 'pin-note';
  const describedBy = [problem ? problemId : '', refused ? refusalId : '']
    .filter(Boolean)
    .join(' ');
  return (
    <InsetRow label={label}>
      <input
        type="password"
        aria-label={label}
        aria-invalid={problem ? true : undefined}
        aria-describedby={describedBy || undefined}
        autoComplete="off"
        autoCorrect="off"
        autoCapitalize="off"
        spellCheck={false}
        placeholder={placeholder}
        title={title}
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
      />
      {problem ? (
        <small id={problemId} className="pin-note">
          {problem}
        </small>
      ) : null}
      {refused ? (
        <small id={refusalId} className={tone}>
          {securityKeyPinRefusalText(refused)}
        </small>
      ) : null}
    </InsetRow>
  );
}
