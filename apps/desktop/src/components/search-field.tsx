/** Controlled search input with a clear button and keyboard shortcut. */

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

  return (
    <label className={shortcut ? 'search' : 'search no-shortcut'}>
      <Icon name="search" />
      <input
        ref={inputRef}
        type="text"
        value={value}
        placeholder={placeholder}
        aria-label={placeholder}
        autoComplete="off"
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
      {shortcut ? (
        <kbd aria-hidden="true">{isMac() ? '⌘K' : 'Ctrl K'}</kbd>
      ) : null}
    </label>
  );
}
