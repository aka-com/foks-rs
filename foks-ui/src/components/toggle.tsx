/**
 * A disclosure control with a Lucide chevron-down, used for inspect blocks
 * that the native `<details>` triangle made look unfinished.
 *
 * Closed, the chevron points right (the same orientation as the browser
 * marker). Open, it points down. The panel stays in the DOM when closed so
 * inspect JSON remains available to tests and to copy.
 */

import { createElement, useId, useState } from 'react';
import type { ReactNode } from 'react';
import { ChevronDown, type IconNode } from 'lucide';

const REACT_ATTR_NAMES: Readonly<Record<string, string>> = {
  class: 'className',
  'stroke-linecap': 'strokeLinecap',
  'stroke-linejoin': 'strokeLinejoin',
  'stroke-width': 'strokeWidth',
};

function lucideNode(node: IconNode | { default?: IconNode }): IconNode {
  if (Array.isArray(node)) return node;
  if (node && Array.isArray(node.default)) return node.default;
  throw new Error('lucide icon node is missing');
}

function ChevronDownIcon(): ReactNode {
  const node = lucideNode(ChevronDown);
  return (
    <svg
      className="toggle-chevron"
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      width={13}
      height={13}
      aria-hidden="true"
      focusable="false"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {node.map(([tag, attributes], index) =>
        createElement(tag, {
          ...Object.fromEntries(
            Object.entries(attributes ?? {}).map(([key, value]) => [
              REACT_ATTR_NAMES[key] ?? key,
              value,
            ]),
          ),
          key: index,
        }),
      )}
    </svg>
  );
}

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
        <ChevronDownIcon />
        {label}
      </button>
      <div id={panelId} className="toggle-body" hidden={!open}>
        {children}
      </div>
    </div>
  );
}
