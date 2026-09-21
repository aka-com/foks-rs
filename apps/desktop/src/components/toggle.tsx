/**
 * Collapsible disclosure section with chevron indicator and expandable content.
 *
 * Displays a disclosure button that expands or collapses associated panel content.
 */

import { useId, useState } from 'react';
import type { ReactNode } from 'react';
import { Icon } from './icon';

export interface ToggleProps {
  label: ReactNode;
  children: ReactNode;
  /** Controlled open state. When set, the trigger does not toggle itself. */
  open?: boolean;
  defaultOpen?: boolean;
  disabled?: boolean;
  className?: string;
}

export function Toggle({
  label,
  children,
  open: openProp,
  defaultOpen = false,
  disabled = false,
  className,
}: ToggleProps): ReactNode {
  const [uncontrolled, setUncontrolled] = useState(defaultOpen);
  const controlled = openProp !== undefined;
  const open = controlled ? openProp : uncontrolled;
  const panelId = useId();
  return (
    <div className={['toggle', className].filter(Boolean).join(' ')}>
      <button
        type="button"
        className="toggle-trigger"
        aria-expanded={open}
        aria-controls={panelId}
        disabled={disabled}
        onClick={() => {
          if (controlled || disabled) return;
          setUncontrolled((value) => !value);
        }}
      >
        <Icon name="chevronDown" className="toggle-chevron" size={13} />
        {label}
      </button>
      <div id={panelId} className="toggle-body" hidden={!open}>
        {children}
      </div>
    </div>
  );
}
