/**
 * The dropdown form of `RadioCard` — the same card, collapsed to the one
 * that is chosen.
 *
 * "Save in" listed every vault, group and ad-hoc share as a radio card, so a
 * person with a handful of groups met a wall of rows before reaching the
 * fields the sheet is actually asking for. The choice itself is unchanged —
 * a title and the reason under it — so this shows only the chosen card and
 * reopens the full list on demand.
 *
 * The list is the kit's `Listbox` in a `Popover`, which is what keeps the
 * arrow keys, Escape, dismissal on an outside pointer-down and focus return
 * to the trigger; the options are real `role="option"` buttons, so an
 * unavailable one is `aria-disabled` and the roving focus steps over it
 * rather than landing on a choice that cannot be taken.
 */

import { useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { Listbox, Popover } from '/kit/overlay-primitives';
import { Icon } from './icon';

export interface CardOption {
  /** What `onChange` reports, and what `value` is compared against. */
  id: string;
  /** The bold first line. */
  title: ReactNode;
  /** The quiet explanation under it — why a reader would pick this one. */
  detail?: ReactNode;
  /** Dimmed and unchoosable — the design's `.radio.off`. */
  off?: boolean;
}

export interface CardSelectProps {
  /** What is being chosen, for the screen reader. */
  label: string;
  options: CardOption[];
  /** The chosen option's id, or one no option carries for "nothing yet". */
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
          // Down and Up open the list the way a native select does; the
          // Listbox then puts focus on the chosen option.
          if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
          event.preventDefault();
          setOpen(true);
        }}
      >
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
