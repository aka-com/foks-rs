/** Accessible card-styled radio button and radio group components. */

import { useLayoutEffect, useRef } from 'react';
import type { KeyboardEvent, ReactNode, RefObject } from 'react';
import { Icon } from './icon';
import type { FoksIconName } from '../icons';

/** The radios that belong to `group` itself, not to a group nested in it. */
function ownRadios(group: HTMLElement): HTMLElement[] {
  return [...group.querySelectorAll<HTMLElement>('[role="radio"]')].filter(
    (radio) =>
      radio.closest('[role="radiogroup"]') === group &&
      !(radio instanceof HTMLButtonElement && radio.disabled),
  );
}

/** One Tab stop: the checked radio, else the first one that can be chosen. */
function syncTabStop(group: HTMLElement): void {
  const radios = ownRadios(group);
  const stop =
    radios.find((radio) => radio.getAttribute('aria-checked') === 'true') ??
    radios.find((radio) => radio.getAttribute('aria-disabled') !== 'true') ??
    radios[0];
  for (const radio of radios) radio.tabIndex = radio === stop ? 0 : -1;
}

/**
 * The WAI-ARIA radio group model for a container of `role="radio"` buttons:
 * the group is one Tab stop (the checked radio, else the first), and the
 * arrow keys move to the next or previous radio, wrapping, and choose it by
 * clicking it. A radio marked `aria-disabled` takes focus, so its reason is
 * read, but its click does nothing. Return is left to the enclosing sheet,
 * which presses its default button (see `Sheet`).
 *
 * Spread the result on the element that carries `role="radiogroup"`.
 */
export function useRadioGroupKeys<E extends HTMLElement>(): {
  ref: RefObject<E | null>;
  onKeyDown: (event: KeyboardEvent<E>) => void;
} {
  const ref = useRef<E | null>(null);
  useLayoutEffect(() => {
    const group = ref.current;
    if (!group) return;
    syncTabStop(group);
    if (typeof MutationObserver === 'undefined') return;
    const observer = new MutationObserver(() => syncTabStop(group));
    observer.observe(group, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ['aria-checked', 'aria-disabled', 'disabled'],
    });
    return () => observer.disconnect();
  }, []);
  const onKeyDown = (event: KeyboardEvent<E>): void => {
    const group = ref.current;
    if (!group || event.metaKey || event.ctrlKey || event.altKey) return;
    const step =
      event.key === 'ArrowDown' || event.key === 'ArrowRight'
        ? 1
        : event.key === 'ArrowUp' || event.key === 'ArrowLeft'
          ? -1
          : 0;
    if (step === 0) return;
    const target = event.target instanceof Element ? event.target : null;
    const current = target?.closest<HTMLElement>('[role="radio"]');
    const radios = ownRadios(group);
    const index = current ? radios.indexOf(current) : -1;
    if (index < 0) return;
    event.preventDefault();
    const next = radios[(index + step + radios.length) % radios.length];
    next.focus();
    if (next.getAttribute('aria-checked') !== 'true') next.click();
  };
  return { ref, onKeyDown };
}

export interface RadioGroupProps {
  /** What is being chosen, for the screen reader. */
  label: string;
  className?: string;
  children: ReactNode;
}

export function RadioGroup({
  label,
  className,
  children,
}: RadioGroupProps): ReactNode {
  const keys = useRadioGroupKeys<HTMLDivElement>();
  return (
    <div
      ref={keys.ref}
      onKeyDown={keys.onKeyDown}
      role="radiogroup"
      aria-label={label}
      className={['radios', className].filter(Boolean).join(' ')}
    >
      {children}
    </div>
  );
}

export interface RadioCardProps {
  /** The bold first line. */
  title: ReactNode;
  /** Supporting text under the label. */
  detail?: ReactNode;
  selected: boolean;
  /** A glyph before the title, saying what kind of thing the card is. */
  icon?: FoksIconName;
  onSelect?: () => void;
  disabled?: boolean;
  /**
   * Dimmed and inert — the design's `.radio.off`. The card keeps its place
   * and does nothing: like a menu item that does not apply, it is marked
   * `aria-disabled` rather than `disabled`, so the arrow keys reach it and
   * VoiceOver reads why. `detail` is that reason, and carries it into the `title` as
   * well, because WKWebView suppresses hover on a natively disabled control.
   */
  off?: boolean;
  /** A trailing element, such as the store roster on a `store-choice`. */
  tail?: ReactNode;
  /** A modifier the sheet draws, such as `store-choice`. */
  className?: string;
}

export function RadioCard({
  title,
  detail,
  selected,
  icon,
  onSelect,
  disabled = false,
  off = false,
  tail,
  className,
}: RadioCardProps): ReactNode {
  const classes = [
    'radio',
    className ?? '',
    selected ? 'on' : '',
    off ? 'off' : '',
  ]
    .filter(Boolean)
    .join(' ');
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      className={classes}
      disabled={disabled}
      aria-disabled={off ? true : undefined}
      title={off && typeof detail === 'string' ? detail : undefined}
      onClick={off ? undefined : onSelect}
    >
      <span className="rb" />
      {icon ? (
        <span className="rico" aria-hidden="true">
          <Icon name={icon} />
        </span>
      ) : null}
      <span className="t">
        <b>{title}</b>
        {detail === undefined ? null : <small>{detail}</small>}
      </span>
      {tail}
    </button>
  );
}
