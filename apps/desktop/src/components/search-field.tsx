/** Controlled search input with optional scope context and keyboard badge. */

import { useEffect, useRef } from 'react';
import type { ReactNode } from 'react';
import { commandKey, shortcutLabel } from '/kit/keys';
import { anyDialogOpen } from '/kit/overlay-primitives';
import { Icon } from './icon';
import { ariaShortcut } from './menus';

export interface SearchFieldProps {
  value: string;
  onChange: (value: string) => void;
  /** Input placeholder describing the current search scope. */
  placeholder: string;
  /**
   * Binds ⌘F (Ctrl+F on non-macOS platforms) to focus the field and select its
   * text, and renders the shortcut badge. The list bar's search field enables
   * this; the ⌘K search palette manages its own field and shortcut.
   */
  shortcut?: boolean;
  /** Optional search scope used in the input's accessible label. */
  scope?: string;
  /** Whether the search input should be disabled (e.g. when the active view contains no searchable items). */
  disabled?: boolean;
  /** Additional CSS class name for custom layout and positioning. */
  className?: string;
}

export function SearchField({
  value,
  onChange,
  placeholder,
  shortcut = true,
  scope,
  disabled = false,
  className,
}: SearchFieldProps): ReactNode {
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!shortcut || disabled) return;
    const onKeyDown = (event: KeyboardEvent): void => {
      if (
        !commandKey(event) ||
        event.altKey ||
        event.shiftKey ||
        event.key.toLowerCase() !== 'f'
      )
        return;
      // This listener runs on document, beyond a dialog's event boundary.
      if (anyDialogOpen()) return;
      event.preventDefault();
      focusSearch(inputRef.current);
    };
    document.addEventListener('keydown', onKeyDown);
    // Native Edit ▸ Find intercepts ⌘F before it reaches the webview and
    // dispatches `FIND_EVENT` to focus the search input.
    const onFind = (): void => focusSearch(inputRef.current);
    window.addEventListener(FIND_EVENT, onFind);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
      window.removeEventListener(FIND_EVENT, onFind);
    };
  }, [shortcut, disabled]);

  const label = scope ? `Search ${scope}` : placeholder;
  return (
    <label
      className={['search', shortcut ? '' : 'no-shortcut', className ?? '']
        .filter(Boolean)
        .join(' ')}
    >
      <Icon name="search" />
      <input
        ref={inputRef}
        type="text"
        value={value}
        placeholder={placeholder}
        aria-label={label}
        aria-keyshortcuts={shortcut ? ariaShortcut('⌘F') : undefined}
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
      {shortcut ? <kbd aria-hidden="true">{shortcutLabel('⌘F')}</kbd> : null}
    </label>
  );
}

/** Event name dispatched on `window` to focus the list bar's search field. */
export const FIND_EVENT = 'foks:find';

function focusSearch(input: HTMLInputElement | null): void {
  if (!input || input.disabled || anyDialogOpen()) return;
  input.focus();
  input.select();
}
