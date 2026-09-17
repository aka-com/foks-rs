/**
 * Segmented control for switching between mutually exclusive options.
 * Supports text and icon display variants.
 */

import type { ReactNode } from 'react';
import { Icon } from './icon';
import type { FoksIconName } from '../icons';

export interface SegmentedItem<T extends string> {
  id: T;
  label?: string;
  icon?: FoksIconName;
  title?: string;
}

export interface SegmentedControlProps<T extends string> {
  items: readonly SegmentedItem<T>[];
  value: T;
  onChange: (value: T) => void;
  /** `text` is the word row; `icon` the square pair. */
  variant?: 'text' | 'icon';
  /** What the group is choosing, for the screen reader. */
  label: string;
  disabled?: boolean;
}

export function SegmentedControl<T extends string>({
  items,
  value,
  onChange,
  variant = 'text',
  label,
  disabled = false,
}: SegmentedControlProps<T>): ReactNode {
  return (
    <span
      className={variant === 'text' ? 'seg txt' : 'seg'}
      role="group"
      aria-label={label}
    >
      {items.map((item) => (
        <button
          key={item.id}
          type="button"
          className={item.id === value ? 'on' : ''}
          aria-pressed={item.id === value}
          title={item.title}
          disabled={disabled}
          onClick={() => {
            onChange(item.id);
          }}
        >
          {item.icon ? <Icon name={item.icon} /> : null}
          {item.label}
        </button>
      ))}
    </span>
  );
}
