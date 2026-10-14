/** Controlled search input with optional scope context and keyboard badge. */

import { useEffect, useId, useRef } from 'react';
import type { ReactNode } from 'react';
import { Icon } from './icon';

export interface SearchFieldProps {
  value: string;
  onChange: (value: string) => void;
  /** Input placeholder describing the current search scope. */
  placeholder: string;
  /**
   * Whether to bind the ⌘K shortcut and display its badge. Set to false to
   * avoid conflicting with the global search palette shortcut.
   */
  shortcut?: boolean;
  /**
   * Draw the ⌘K badge without binding the key. The header's field says which
   * key opens the palette beside it; it does not claim the key itself.
   */
  kbd?: boolean;
  /** Optional search scope used in the input's accessible label. */
  scope?: string;
  /** The input's accessible name, in place of the one `scope` makes. */
  label?: string;
  /**
   * A visible scope drawn before the input, naming what typing searches, with
   * a button that drops it. Backspace in the empty field drops it too.
   */
  badge?: { label: string; clearLabel: string; onClear: () => void };
  /** Whether the search input should be disabled (e.g. when the active view contains no searchable items). */
  disabled?: boolean;
  /** Additional CSS class name for custom layout and positioning. */
  className?: string;
}

function isMac(): boolean {
  if (typeof navigator === 'undefined') return true;
  return /Mac|iPhone|iPad/i.test(
    `${navigator.platform} ${navigator.userAgent}`,
  );
}

export function SearchField({
  value,
  onChange,
  placeholder,
  shortcut = true,
  kbd = false,
  scope,
  label: accessibleName,
  badge,
  disabled = false,
  className,
}: SearchFieldProps): ReactNode {
  const inputRef = useRef<HTMLInputElement>(null);
  const inputId = useId();

  useEffect(() => {
    if (!shortcut) return;
    const onKeyDown = (event: KeyboardEvent): void => {
      if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== 'k')
        return;
      event.preventDefault();
      inputRef.current?.focus();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [shortcut]);

  const label = accessibleName ?? (scope ? `Search ${scope}` : placeholder);
  // Dropping the badge removes its button; focus stays in the field.
  const clearBadge = (): void => {
    badge?.onClear();
    inputRef.current?.focus();
  };
  // The label names the input as its control, so a click on the icon or on
  // the badge's text focuses the field. Without `htmlFor` the badge's button,
  // the first labelable element inside, would be the control.
  return (
    <label
      htmlFor={inputId}
      className={['search', shortcut ? '' : 'no-shortcut', className ?? '']
        .filter(Boolean)
        .join(' ')}
    >
      <Icon name="search" />
      {badge ? (
        <span className="search-badge">
          <span className="n">{badge.label}</span>
          <button
            type="button"
            aria-label={badge.clearLabel}
            title={badge.clearLabel}
            disabled={disabled}
            onClick={clearBadge}
          >
            <Icon name="close" />
          </button>
        </span>
      ) : null}
      <input
        ref={inputRef}
        id={inputId}
        type="text"
        value={value}
        placeholder={placeholder}
        aria-label={label}
        autoComplete="off"
        disabled={disabled}
        onChange={(event) => {
          onChange(event.target.value);
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape' && value) {
            event.stopPropagation();
            onChange('');
          } else if (event.key === 'Backspace' && !value && badge) {
            event.preventDefault();
            clearBadge();
          }
        }}
      />
      {shortcut || kbd ? (
        <kbd aria-hidden="true">{isMac() ? '⌘K' : 'Ctrl K'}</kbd>
      ) : null}
    </label>
  );
}
