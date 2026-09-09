/**
 * Renders SVG icons from structured element definitions without raw markup strings.
 *
 * Standard icon stroke width, line caps, and fill attributes default to consistent
 * design system values.
 */

import { createElement } from 'react';
import type { ReactNode } from 'react';
import { FOKS_ICONS } from '../icons';
import type { FoksIconName } from '../icons';

const REACT_ATTR_NAMES: Readonly<Record<string, string>> = {
  'stroke-linecap': 'strokeLinecap',
  'stroke-linejoin': 'strokeLinejoin',
  'stroke-width': 'strokeWidth',
};

export interface IconProps {
  name: FoksIconName;
  /**
   * Explicit pixel size. Defaults to `1em`, inheriting font size from the
   * containing element.
   */
  size?: number;
  className?: string;
}

export function Icon({ name, size, className }: IconProps): ReactNode {
  return (
    <svg
      className={className ? `ic ${className}` : 'ic'}
      viewBox="0 0 24 24"
      width={size}
      height={size}
      aria-hidden="true"
      focusable="false"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.7}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {FOKS_ICONS[name].map(([tag, attrs], index) =>
        createElement(tag, {
          ...Object.fromEntries(
            Object.entries(attrs).map(([key, value]) => [
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
