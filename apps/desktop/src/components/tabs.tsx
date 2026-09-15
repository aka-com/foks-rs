/** Navigation tab strip with an active indicator and optional badge counts. */

import { useId, useRef } from 'react';
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
  /**
   * The base the tab and panel ids are derived from. Pass the same base to
   * `tabId` and `tabPanelId` on the panel; omit it and the strip generates one,
   * which only works where the panel does not need to point back.
   */
  idBase?: string;
}

/** The id of one tab, for a panel's `aria-labelledby`. */
export function tabId(base: string, id: string): string {
  return `${base}-tab-${id}`;
}

/** The id of one tab's panel, for a tab's `aria-controls`. */
export function tabPanelId(base: string, id: string): string {
  return `${base}-panel-${id}`;
}

export function Tabs<T extends string>({
  items,
  value,
  onChange,
  label,
  idBase,
}: TabsProps<T>): ReactNode {
  const generated = useId();
  const base = idBase ?? generated;
  const strip = useRef<HTMLDivElement>(null);
  // One tab is in the page's tab order and the arrows walk the rest, so the
  // strip costs one Tab press rather than one per tab. Selection follows
  // focus: every panel is already mounted by the choice itself.
  const moveTo = (index: number): void => {
    const next = items[(index + items.length) % items.length];
    if (!next) return;
    onChange(next.id);
    strip.current
      ?.querySelector<HTMLButtonElement>(`[data-tab="${next.id}"]`)
      ?.focus();
  };
  const index = items.findIndex((item) => item.id === value);
  return (
    <div
      className="tabs"
      role="tablist"
      aria-label={label}
      ref={strip}
      onKeyDown={(event) => {
        if (index < 0) return;
        if (event.key === 'ArrowRight') moveTo(index + 1);
        else if (event.key === 'ArrowLeft') moveTo(index - 1);
        else if (event.key === 'Home') moveTo(0);
        else if (event.key === 'End') moveTo(items.length - 1);
        else return;
        event.preventDefault();
      }}
    >
      {items.map((item) => {
        const selected = item.id === value;
        return (
          <button
            key={item.id}
            type="button"
            role="tab"
            id={tabId(base, item.id)}
            data-tab={item.id}
            className={selected ? 'tab on' : 'tab'}
            aria-selected={selected}
            // Only the chosen panel is mounted, so only the chosen tab names
            // one: a tab pointing at markup that is not there names nothing.
            aria-controls={selected ? tabPanelId(base, item.id) : undefined}
            tabIndex={selected ? 0 : -1}
            onClick={() => {
              onChange(item.id);
            }}
          >
            {item.label}
            {item.count === undefined ? null : (
              <span className={item.count ? 'n' : 'n zero'}>{item.count}</span>
            )}
          </button>
        );
      })}
    </div>
  );
}
