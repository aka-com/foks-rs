/**
 * The shell's icon, rendered the way `ui/kit/icon.tsx`'s `AppIcon` renders
 * AKA's: one `<svg>` whose children are built with `createElement` from
 * structured data, never from a markup string.
 *
 * The wrapper attributes reproduce `shell.css`'s bare `svg` rule — the mock
 * gives every icon `fill: none`, `stroke: currentColor`, `stroke-width: 1.7`
 * and round caps and joins from the stylesheet, so an icon lifted out of that
 * page keeps its weight here.
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
   * Explicit pixel size. Omit it and the icon is `1em`, which is how the mock
   * sizes icons: the container's `font-size` decides.
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
