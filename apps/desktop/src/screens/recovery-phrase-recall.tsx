/**
 * The check that replaces "I have written this down" when a recovery phrase
 * is created: three tokens typed back from the written copy while the phrase
 * itself is hidden. It compares with the phrase setup already holds in memory,
 * so it needs no agent call; a full test of the copy is on Devices.
 */

import { useEffect, useId, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Icon } from '../components';
import {
  normalizeToken,
  recallMatches,
  tokenKind,
  tokenLabel,
} from '../recovery-phrase';
import { useConcealOnInactive } from '../use-conceal-on-inactive';

export interface RecallCheckProps {
  /** The phrase as generated, held in memory until it is committed. */
  phrase: string;
  /** Three zero-based positions from `recallPositions`, chosen once per phrase. */
  positions: readonly number[];
  /** The control the check unlocks, named in its status line. */
  unlocks: string;
  /** The control that shows the phrase again, named in the intro. */
  reveal: string;
  onMatched: (matched: boolean) => void;
}

export function RecallCheck({
  phrase,
  positions,
  unlocks,
  reveal,
  onMatched,
}: RecallCheckProps): ReactNode {
  const id = useId();
  const [values, setValues] = useState<Record<number, string>>({});
  const [left, setLeft] = useState<ReadonlySet<number>>(new Set());
  const [focused, setFocused] = useState<number | null>(null);
  // Typed tokens are masked while the window is inactive, as the phrase is.
  const [concealed, setConcealed] = useState(false);
  useConcealOnInactive(() => setConcealed(true));
  const matched = positions.map((index) =>
    recallMatches(phrase, index, values[index] ?? ''),
  );
  const all = matched.every(Boolean);
  const filled = positions.every((index) => (values[index] ?? '') !== '');
  // A wrong entry is reported once its field is left, or once all three hold
  // something; the field being typed in is not called wrong halfway through.
  const wrong = positions.filter(
    (index, at) =>
      (values[index] ?? '') !== '' &&
      !matched[at] &&
      focused !== index &&
      (left.has(index) || filled),
  );
  // The latest callback, so a caller that passes a new function each render
  // does not re-run the effects below.
  const report = useRef(onMatched);
  report.current = onMatched;
  useEffect(() => {
    report.current(all);
  }, [all]);
  // The entries are part of the phrase: they go when the check does.
  useEffect(() => () => report.current(false), []);

  return (
    <>
      <p id={`${id}-intro`}>
        Type these from what you wrote down, not from memory. The phrase stays
        hidden while you check; select <b>{reveal}</b> to see it again.
      </p>
      <div
        className="inset recall-check"
        role="group"
        aria-describedby={`${id}-intro`}
      >
        {positions.map((index, at) => {
          const label = tokenLabel(index);
          const value = values[index] ?? '';
          const bad = wrong.includes(index);
          return (
            <div className={`fr${bad ? ' bad' : ''}`} key={index}>
              <label className="k" htmlFor={`${id}-${index}`}>
                {label}
                <small>position {index + 1}</small>
              </label>
              <span className="v">
                <input
                  id={`${id}-${index}`}
                  type={concealed ? 'password' : 'text'}
                  autoComplete="off"
                  autoCapitalize="off"
                  spellCheck={false}
                  inputMode={
                    tokenKind(index) === 'number' ? 'numeric' : undefined
                  }
                  aria-invalid={bad || undefined}
                  aria-describedby={`${id}-${index}-error`}
                  value={value}
                  onChange={(event) =>
                    setValues((current) => ({
                      ...current,
                      [index]: normalizeToken(index, event.target.value),
                    }))
                  }
                  onFocus={() => {
                    setFocused(index);
                    setConcealed(false);
                  }}
                  onBlur={() => {
                    setFocused(null);
                    setLeft((current) => new Set([...current, index]));
                  }}
                />
                {matched[at] ? (
                  <span className="recall-ok">
                    <Icon name="check" />
                    Matches
                  </span>
                ) : null}
              </span>
              <span className="recall-error" id={`${id}-${index}-error`}>
                {bad
                  ? `This is not ${label.toLowerCase()} of the phrase. Compare position ${index + 1} on your copy.`
                  : null}
              </span>
            </div>
          );
        })}
      </div>
      <p className="recall-status" aria-live="polite">
        {all
          ? 'All three match your phrase.'
          : wrong.length === 1
            ? 'One entry does not match. Fix it to continue.'
            : wrong.length > 1
              ? `${wrong.length} entries do not match. Fix them to continue.`
              : `${unlocks} unlocks when all three match.`}
      </p>
    </>
  );
}
