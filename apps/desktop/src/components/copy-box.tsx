/**
 * A bordered container displaying text alongside a copy button.
 */

import type { ReactNode } from 'react';
import { Button } from './button';

export interface CopyBoxProps {
  /** The exact text shown and copied. */
  text: string;
  /** Optional custom display content when different from the copied text. */
  children?: ReactNode;
  onCopy: (text: string) => void;
  /** Button label. Defaults to "Copy". */
  label?: string;
  /** Optional secondary action rendered next to the copy button. */
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
