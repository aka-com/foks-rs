/**
 * Dropdown selector displaying the chosen card option and expanding a
 * popover listbox of selectable options on demand.
 */

import { useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { Listbox, Popover } from '/kit/overlay-primitives';
import { Icon } from './icon';

export interface CardOption {
  /** The unique option identifier. */
  id: string;
  /** Optional leading mark shared by the trigger and option row. */
  mark?: ReactNode;
  /** Primary label text. */
  title: ReactNode;
  /** Secondary explanatory text displayed below the title. */
  detail?: ReactNode;
  /** Disables the option and displays it in a dimmed state. */
  off?: boolean;
}

export interface CardSelectProps {
  /** Accessible label describing the selection list. */
  label: string;
  options: CardOption[];
  /** The chosen option ID, or an empty string when unselected. */
  value: string;
  onChange: (id: string) => void;
  /** The trigger's first line when `value` names no option. */
  placeholder?: ReactNode;
  className?: string;
  disabled?: boolean;
}

export function CardSelect({
  label,
  options,
  value,
  onChange,
  placeholder,
  className,
  disabled = false,
}: CardSelectProps): ReactNode {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const chosen = options.find((option) => option.id === value);
  const close = (): void => {
    setOpen(false);
  };
  return (
    <div className={['card-select', className ?? ''].filter(Boolean).join(' ')}>
      <button
        type="button"
        ref={anchorRef}
        className="card-select-trigger"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={label}
        disabled={disabled}
        onClick={() => {
          setOpen((was) => !was);
        }}
        onKeyDown={(event: KeyboardEvent<HTMLButtonElement>) => {
          // Arrow keys open the popover listbox and focus the current option.
          if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
          event.preventDefault();
          setOpen(true);
        }}
      >
        {chosen?.mark}
        <span className="t">
          <b>{chosen ? chosen.title : placeholder}</b>
          {chosen?.detail === undefined ? null : <small>{chosen.detail}</small>}
        </span>
        <Icon name="chev" className="card-select-chevron" />
      </button>
      {open ? (
        <Popover
          anchorRef={anchorRef}
          className="menu-portal"
          align="start"
          gap={4}
          matchAnchorWidth
          onClose={close}
        >
          <Listbox
            className="card-select-menu"
            anchorRef={anchorRef}
            onClose={close}
            initialFocus="selected"
            aria-label={label}
          >
            {options.map((option) => {
              const selected = option.id === value;
              return (
                <button
                  key={option.id}
                  type="button"
                  role="option"
                  aria-selected={selected}
                  aria-disabled={option.off ? true : undefined}
                  disabled={option.off ?? false}
                  className={[
                    'card-select-option',
                    selected ? 'on' : '',
                    option.off ? 'off' : '',
                  ]
                    .filter(Boolean)
                    .join(' ')}
                  onClick={() => {
                    onChange(option.id);
                    close();
                  }}
                >
                  {option.mark}
                  <span className="t">
                    <b>{option.title}</b>
                    {option.detail === undefined ? null : (
                      <small>{option.detail}</small>
                    )}
                  </span>
                  {selected ? (
                    <Icon name="check" className="card-select-check" />
                  ) : null}
                </button>
              );
            })}
          </Listbox>
        </Popover>
      ) : null}
    </div>
  );
}
