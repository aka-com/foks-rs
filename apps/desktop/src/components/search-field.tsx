/** Controlled search input with an optional scope chip and keyboard badge. */

import { useEffect, useRef } from 'react';
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
  /** Optional label displayed as a badge inside the search input to indicate the current search filter scope. */
  scope?: string;
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
  disabled = false,
  className,
}: SearchFieldProps): ReactNode {
  const inputRef = useRef<HTMLInputElement>(null);

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

  const label = scope ? `Search ${scope}` : placeholder;
  return (
    <label
      className={['search', shortcut ? '' : 'no-shortcut', className ?? '']
        .filter(Boolean)
        .join(' ')}
    >
      {scope ? <span className="scope">{scope}</span> : null}
      <Icon name="search" />
      <input
        ref={inputRef}
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
          }
        }}
      />
      {shortcut || kbd ? (
        <kbd aria-hidden="true">{isMac() ? '⌘K' : 'Ctrl K'}</kbd>
      ) : null}
    </label>
  );
}
