/**
 * Password row for the login editors: a masked `Field` with Show and
 * Generate, and strength feedback under the value.
 *
 * Generate opens a panel below the row that draws a password in the renderer
 * (see `password-generator.ts`). Use password puts it in the field, and while
 * the field still holds that value its strength is the exact entropy of the
 * draw. A typed value is only checked for common weaknesses; a warning never
 * disables saving.
 */

import { useId, useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { Button } from './button';
import { Field } from './field';
import { Icon } from './icon';
import { SegmentedControl } from './segmented-control';
import {
  LENGTH_RANGE,
  PASSPHRASE_SEPARATORS,
  WORD_RANGE,
  generatePassword,
  rememberGeneratorPolicy,
  storedGeneratorPolicy,
  strengthOfBits,
  typedPasswordStrength,
} from '../password-generator';
import type {
  CharacterSet,
  GeneratedPassword,
  GeneratorMode,
  GeneratorPolicy,
  PassphraseSeparator,
  Strength,
} from '../password-generator';
import { useConcealOnInactive } from '../use-conceal-on-inactive';

export interface PasswordInputProps {
  label?: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  /** The value is shown as text; the caller resets this when it conceals. */
  shown: boolean;
  onShownChange: (shown: boolean) => void;
  /** Why Show is unavailable while the value is masked. */
  showProblem?: string;
  /** The element that states `showProblem` for assistive technology. */
  showProblemId?: string;
  /** The site and user name, which a typed password should not contain. */
  context?: readonly string[];
}

export function PasswordInput({
  label = 'Password',
  value,
  onChange,
  placeholder = 'password',
  shown,
  onShownChange,
  showProblem,
  showProblemId,
  context = [],
}: PasswordInputProps): ReactNode {
  const [generatorOpen, setGeneratorOpen] = useState(false);
  // The last password taken from the generator, with its exact entropy.
  const [generated, setGenerated] = useState<GeneratedPassword | null>(null);
  const generateRef = useRef<HTMLButtonElement>(null);
  const strengthId = useId();
  const panelId = useId();
  // The panel shows a candidate in plain text.
  useConcealOnInactive(() => setGeneratorOpen(false), generatorOpen);

  const strength =
    value && generated?.value === value
      ? strengthOfBits(generated.bits)
      : typedPasswordStrength(value, context);
  const blocked = !shown && showProblem !== undefined;
  const name = label.toLowerCase();
  const closeGenerator = (): void => {
    setGeneratorOpen(false);
    generateRef.current?.focus();
  };
  return (
    <>
      <Field
        label={label}
        value={value}
        onChange={onChange}
        type={shown ? 'text' : 'password'}
        placeholder={placeholder}
        className="pw-row"
        verbatim
        describedBy={strength ? strengthId : undefined}
        hint={<PasswordStrength id={strengthId} strength={strength} live />}
        action={
          <>
            <button
              className="value-action"
              type="button"
              aria-label={shown ? `Hide ${name}` : `Show ${name}`}
              title={blocked ? showProblem : shown ? 'Hide' : 'Show'}
              disabled={blocked}
              aria-describedby={blocked ? showProblemId : undefined}
              onClick={() => onShownChange(!shown)}
            >
              <Icon name={shown ? 'eyeOff' : 'eye'} />
            </button>
            <button
              ref={generateRef}
              className="value-action"
              type="button"
              aria-expanded={generatorOpen}
              aria-controls={generatorOpen ? panelId : undefined}
              onClick={() => setGeneratorOpen((open) => !open)}
            >
              <Icon name="dices" />
              Generate
            </button>
          </>
        }
      />
      {generatorOpen ? (
        <PasswordGenerator
          id={panelId}
          onUse={(password) => {
            setGenerated(password);
            onChange(password.value);
            closeGenerator();
          }}
          onCancel={closeGenerator}
        />
      ) : null}
    </>
  );
}

/**
 * The meter and text for a strength. A `live` text is announced when it
 * changes; it stays mounted, empty, while there is nothing to rate, since a
 * live region added together with its content may not be announced.
 */
function PasswordStrength({
  id,
  strength,
  live = false,
}: {
  id?: string;
  strength: Strength | null;
  live?: boolean;
}): ReactNode {
  return (
    <span
      id={id}
      className={strength?.weak ? 'pw-strength weak' : 'pw-strength'}
    >
      {strength === null || strength.level === null ? null : (
        <span
          className="pw-meter"
          data-level={strength.level}
          aria-hidden="true"
        >
          <i />
          <i />
          <i />
          <i />
        </span>
      )}
      <span
        className="pw-strength-text"
        aria-live={live ? 'polite' : undefined}
      >
        {strength?.text}
      </span>
    </span>
  );
}

const SET_BUTTONS: readonly {
  set: CharacterSet;
  text: string;
  label: string;
}[] = [
  { set: 'upper', text: 'A–Z', label: 'A–Z, uppercase letters' },
  { set: 'lower', text: 'a–z', label: 'a–z, lowercase letters' },
  { set: 'digits', text: '0–9', label: '0–9, digits' },
  { set: 'symbols', text: '!@#', label: '!@#, symbols' },
];

const SEPARATOR_NAMES: Readonly<Record<PassphraseSeparator, string>> = {
  '-': 'Hyphen (-)',
  ' ': 'Space',
  '.': 'Period (.)',
  _: 'Underscore (_)',
};

function PasswordGenerator({
  id,
  onUse,
  onCancel,
}: {
  id: string;
  onUse: (password: GeneratedPassword) => void;
  onCancel: () => void;
}): ReactNode {
  const [policy, setPolicy] = useState(storedGeneratorPolicy);
  const [candidate, setCandidate] = useState(() => generatePassword(policy));
  const [hint, setHint] = useState('');
  const lengthId = useId();
  const wordsId = useId();
  const separatorId = useId();

  const apply = (next: GeneratorPolicy): void => {
    setPolicy(next);
    rememberGeneratorPolicy(next);
    setCandidate(generatePassword(next));
    setHint('');
  };
  const toggleSet = (set: CharacterSet): void => {
    const selected = Object.values(policy.sets).filter(Boolean).length;
    if (policy.sets[set] && selected === 1) {
      setHint('Keep at least one set of characters.');
      return;
    }
    apply({ ...policy, sets: { ...policy.sets, [set]: !policy.sets[set] } });
  };
  // Escape closes the panel, not the sheet or panel around it.
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    onCancel();
  };

  return (
    <div
      id={id}
      className="pw-gen"
      role="group"
      aria-label="Generate a password"
      onKeyDown={onKeyDown}
    >
      <SegmentedControl<GeneratorMode>
        label="Password style"
        items={[
          { id: 'characters', label: 'Characters' },
          { id: 'passphrase', label: 'Passphrase' },
        ]}
        value={policy.mode}
        onChange={(mode) => apply({ ...policy, mode })}
      />
      {policy.mode === 'characters' ? (
        <>
          <div className="pw-gen-row">
            <label htmlFor={lengthId}>Length</label>
            <input
              id={lengthId}
              type="range"
              min={LENGTH_RANGE.min}
              max={LENGTH_RANGE.max}
              value={policy.length}
              onChange={(event) =>
                apply({ ...policy, length: Number(event.target.value) })
              }
            />
            <output htmlFor={lengthId}>{policy.length}</output>
          </div>
          <div
            className="pw-gen-sets"
            role="group"
            aria-label="Characters to use"
          >
            {SET_BUTTONS.map(({ set, text, label }) => (
              <button
                key={set}
                type="button"
                className={policy.sets[set] ? 'on' : undefined}
                aria-pressed={policy.sets[set]}
                aria-label={label}
                title={label}
                onClick={() => toggleSet(set)}
              >
                {text}
              </button>
            ))}
          </div>
          <label className="pw-gen-check">
            <input
              type="checkbox"
              checked={policy.avoidAmbiguous}
              onChange={(event) =>
                apply({ ...policy, avoidAmbiguous: event.target.checked })
              }
            />
            Avoid ambiguous characters (0 O 1 l I)
          </label>
          <p className="pw-gen-hint" role="status">
            {hint}
          </p>
        </>
      ) : (
        <>
          <div className="pw-gen-row">
            <label htmlFor={wordsId}>Words</label>
            <input
              id={wordsId}
              type="range"
              min={WORD_RANGE.min}
              max={WORD_RANGE.max}
              value={policy.words}
              onChange={(event) =>
                apply({ ...policy, words: Number(event.target.value) })
              }
            />
            <output htmlFor={wordsId}>{policy.words}</output>
          </div>
          <div className="pw-gen-row">
            <label htmlFor={separatorId}>Separator</label>
            <select
              id={separatorId}
              value={policy.separator}
              onChange={(event) =>
                apply({
                  ...policy,
                  separator: event.target.value as PassphraseSeparator,
                })
              }
            >
              {PASSPHRASE_SEPARATORS.map((separator) => (
                <option key={separator} value={separator}>
                  {SEPARATOR_NAMES[separator]}
                </option>
              ))}
            </select>
          </div>
        </>
      )}
      <div
        className="pw-gen-preview"
        role="group"
        aria-label="Generated password"
      >
        <code>{candidate.value}</code>
        <button
          type="button"
          className="value-action"
          aria-label="Regenerate"
          title="Regenerate"
          onClick={() => setCandidate(generatePassword(policy))}
        >
          <Icon name="refresh" />
        </button>
      </div>
      <PasswordStrength strength={strengthOfBits(candidate.bits)} />
      <div className="pw-gen-actions">
        <Button size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button size="sm" variant="primary" onClick={() => onUse(candidate)}>
          Use password
        </Button>
      </div>
    </div>
  );
}
