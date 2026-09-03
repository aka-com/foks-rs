/**
 * A block of text with the button that copies it.
 *
 * FOKS asks people to send sentences to each other — "Add sam on acme to
 * Household", a server address, an invitation — and the design gives each one
 * a bordered block with a Copy beside it. Three call sites wrote that block by
 * hand and labelled the button three different ways ("Copy", "Copy the
 * sentence", "Copy message").
 *
 * The text and the copied string are one prop, so the button cannot come to
 * copy something other than what the reader is looking at — which is the
 * failure the hand-written version invited, each site repeating its sentence
 * once for display and once inside the click handler.
 */

import type { ReactNode } from 'react';
import { Button } from './button';

export interface CopyBoxProps {
  /** The exact text shown and copied. */
  text: string;
  /** How it reads on screen, when that differs from what is copied. */
  children?: ReactNode;
  onCopy: (text: string) => void;
  /** Defaults to "Copy"; a long message names itself. */
  label?: string;
  /** A second action beside Copy, wrapped so the pair wraps as a unit. */
  extra?: ReactNode;
}

export function CopyBox({
  text,
  children,
  onCopy,
  label = 'Copy',
  extra,
}: CopyBoxProps): ReactNode {
  const copy = <Button onClick={() => onCopy(text)}>{label}</Button>;
  return (
    <div className="copybox">
      <span className="v">{children ?? text}</span>
      {extra ? (
        <span className="copybox-actions">
          {copy}
          {extra}
        </span>
      ) : (
        copy
      )}
    </div>
  );
}
