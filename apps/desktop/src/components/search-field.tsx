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
   * Binds ⌘F (Ctrl+F off a Mac) to focus the field and select its text, and
   * draws the badge. The list bar's field is the one that takes it; the ⌘K
   * palette has its own field and its own key.
   */
  shortcut?: boolean;
  /** Optional search scope used in the input's accessible label. */
  scope?: string;
  /** Whether the search input should be disabled (e.g. when the active view contains no searchable items). */
  disabled?: boolean;
  /** Additional CSS class name for custom layout and positioning. */
  className?: string;
  /**
   * The listbox the field narrows, as a selector. With it, Down moves to
   * the first result (focused and selected), Return selects the first
   * result and Escape in an empty field returns to the list, as an
   * NSSearchField over a list does.
   */
  results?: string;
}

export function SearchField({
  value,
  onChange,
  placeholder,
  shortcut = true,
  scope,
  disabled = false,
  className,
  results,
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
    // The native Edit ▸ Find item, whose accelerator takes ⌘F before the
    // page sees it, asks for the same thing.
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
            return;
          }
          if (!results || event.nativeEvent.isComposing) return;
          const plain =
            !event.altKey &&
            !event.shiftKey &&
            !event.metaKey &&
            !event.ctrlKey;
          if (!plain) return;
          const list = document.querySelector<HTMLElement>(results);
          const options = [
            ...(list?.querySelectorAll<HTMLElement>('[role="option"]') ?? []),
          ];
          const first = options[0];
          if (event.key === 'Escape') {
            // The list's own Tab stop: the selected row, else the first.
            const stop =
              options.find((option) => option.tabIndex === 0) ?? first;
            if (!stop) return;
            event.preventDefault();
            event.stopPropagation();
            stop.focus();
            return;
          }
          if (!first || (event.key !== 'ArrowDown' && event.key !== 'Enter'))
            return;
          event.preventDefault();
          if (event.key === 'ArrowDown') first.focus();
          // Rows select on a click; a folder row would open instead, so it
          // only takes the focus.
          if (
            first.getAttribute('aria-selected') !== 'true' &&
            !first.hasAttribute('data-folder-path')
          )
            first.click();
        }}
      />
      {shortcut ? <kbd aria-hidden="true">{shortcutLabel('⌘F')}</kbd> : null}
    </label>
  );
}

/** Dispatched on `window` to focus the list bar's search field. */
export const FIND_EVENT = 'foks:find';

function focusSearch(input: HTMLInputElement | null): void {
  if (!input || input.disabled || anyDialogOpen()) return;
  input.focus();
  input.select();
}
