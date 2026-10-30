/** Accessible card-styled radio button and radio group components. */

import { useLayoutEffect, useRef } from 'react';
import type { KeyboardEvent, ReactNode, RefObject } from 'react';
import { Icon } from './icon';
import type { FoksIconName } from '../icons';

/** Radio buttons belonging directly to `group`, excluding nested radio groups. */
function ownRadios(group: HTMLElement): HTMLElement[] {
  return [...group.querySelectorAll<HTMLElement>('[role="radio"]')].filter(
    (radio) =>
      radio.closest('[role="radiogroup"]') === group &&
      !(radio instanceof HTMLButtonElement && radio.disabled),
  );
}

/** Sets a single Tab stop on the checked radio, or the first enabled radio. */
function syncTabStop(group: HTMLElement): void {
  const radios = ownRadios(group);
  const stop =
    radios.find((radio) => radio.getAttribute('aria-checked') === 'true') ??
    radios.find((radio) => radio.getAttribute('aria-disabled') !== 'true') ??
    radios[0];
  for (const radio of radios) radio.tabIndex = radio === stop ? 0 : -1;
}

/**
 * Implements WAI-ARIA radio group keyboard navigation for `role="radio"`
 * buttons: the group has a single Tab stop (the checked radio, or the first
 * radio), and arrow keys wrap through the options and activate the focused
 * radio. Radios with `aria-disabled` remain focusable so screen readers
 * announce their disabled reason, but clicking them is a no-op. Enter bubbles
 * to the enclosing `Sheet` to activate its default button.
 *
 * Spread the returned props onto the `role="radiogroup"` element.
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
   * Renders the card in the dimmed `.radio.off` state using `aria-disabled`
   * instead of `disabled`, so arrow keys can focus it and VoiceOver can read
   * its `detail` reason. Also sets `title` to `detail` because WKWebView
   * suppresses hover tooltips on natively disabled controls.
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
