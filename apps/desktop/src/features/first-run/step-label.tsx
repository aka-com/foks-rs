import type { ReactNode } from 'react';
import { SectionLabel } from '../../components';

/**
 * A section label with its position in the pane's sequence. The account
 * pages are read top to bottom, each section unlocking the next, so the
 * labels are numbered over the sections actually shown.
 */
export function StepLabel({
  n,
  children,
}: {
  n: number;
  children: ReactNode;
}): ReactNode {
  return (
    <SectionLabel className="step">
      <span className="n">{n}</span>
      {children}
    </SectionLabel>
  );
}
