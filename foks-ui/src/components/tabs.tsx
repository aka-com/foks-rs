/**
 * The group page's tab strip — underline, quiet count pills, a faint zero.
 *
 * The mock's `.tab.on::after` is what makes the selected tab look selected;
 * the old toolbar used `Button on=` and had no underline.
 */

import type { ReactNode } from 'react';

export interface TabItem<T extends string> {
  id: T;
  label: string;
  /** Omit the pill when the tab has no count (Settings). */
  count?: number;
}

export interface TabsProps<T extends string> {
  items: readonly TabItem<T>[];
  value: T;
  onChange: (value: T) => void;
  /** Accessible name describing the tab choices. */
  label: string;
}

export function Tabs<T extends string>({
  items,
  value,
  onChange,
  label,
}: TabsProps<T>): ReactNode {
  return (
    <div className="tabs" role="tablist" aria-label={label}>
      {items.map((item) => (
        <button
          key={item.id}
          type="button"
          role="tab"
          className={item.id === value ? 'tab on' : 'tab'}
          aria-selected={item.id === value}
          onClick={() => {
            onChange(item.id);
          }}
        >
          {item.label}
          {item.count === undefined ? null : (
            <span className={item.count ? 'n' : 'n zero'}>{item.count}</span>
          )}
        </button>
      ))}
    </div>
  );
}
