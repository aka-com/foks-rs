/**
 * Modal sheet with standardized header, content, and action footer.
 *
 * `Sheet` renders the panel within an existing dialog. `SheetDialog` supplies
 * the accessible dialog wrapper and supports standard, mid, and wide sizes.
 */

import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode, RefObject } from 'react';
import { Dialog, DismissibleDialog, inPopup } from '/kit/overlay-primitives';

/** `.sheet`, `.sheet.mid`, `.sheet.wide` — the three the stylesheet draws. */
export type SheetWidth = 'base' | 'mid' | 'wide';

const WIDTH_CLASS: Readonly<Record<SheetWidth, string>> = {
  base: '',
  mid: 'mid',
  wide: 'wide',
};

export interface SheetProps {
  /** The tinted mark beside the title — a `KindIcon`, a server mark, a glyph. */
  glyph?: ReactNode;
  title: ReactNode;
  /**
   * A short progress line under the title, for a sheet that runs in steps.
   * It names the position only — "Step 1 of 2" — and never restates the step.
   */
  step?: ReactNode;
  /** Muted subtitle below the title indicating the target store or subject. */
  subtitle?: ReactNode;
  width?: SheetWidth;
  /** The controls along the bottom. Primary action last, as the design sets. */
  footer?: ReactNode;
  /**
   * Ties the panel's `<h2>` to the dialog's `aria-labelledby`. `SheetDialog`
   * supplies one; a bare `Sheet` only needs it if its caller labels by id.
   */
  titleId?: string;
  children: ReactNode;
}

export function Sheet({
  glyph,
  title,
  step,
  subtitle,
  width = 'base',
  footer,
  titleId,
  children,
}: SheetProps): ReactNode {
  const classes = ['sheet', WIDTH_CLASS[width]].filter(Boolean).join(' ');
  const more = useMoreBelow<HTMLDivElement>();
  return (
    <div className={classes} onKeyDown={pressDefaultButton}>
      <div className="hd">
        {glyph}
        <span className="t">
          <h2 id={titleId}>{title}</h2>
          {subtitle === undefined ? null : (
            <span className="sub">{subtitle}</span>
          )}
          {step === undefined ? null : <span className="step">{step}</span>}
        </span>
      </div>
      <div className="sb" ref={more.ref}>
        {children}
      </div>
      {footer === undefined ? null : (
        <div className={more.below ? 'ft more' : 'ft'}>{footer}</div>
      )}
    </div>
  );
}

/**
 * Activates the sheet's last enabled primary footer button on Enter, matching
 * macOS dialog behavior. Ignores controls that handle Enter directly
 * (textareas, selects, expanded comboboxes, buttons, links, and popup options)
 * and events already handled by nested overlays. Pressing Enter on a focused
 * radio button still triggers the sheet's default button.
 */
function pressDefaultButton(event: KeyboardEvent<HTMLDivElement>): void {
  if (
    event.key !== 'Enter' ||
    event.defaultPrevented ||
    event.nativeEvent.isComposing ||
    event.metaKey ||
    event.ctrlKey ||
    event.altKey ||
    event.shiftKey
  )
    return;
  const target = event.target instanceof Element ? event.target : null;
  if (
    !target ||
    inPopup(target) ||
    (!target.closest('[role="radio"]') &&
      target.closest(
        'textarea, select, button, a[href], [contenteditable="true"], ' +
          '[role="combobox"][aria-expanded="true"], [role="option"], ' +
          '[role="menuitem"], [role="button"], input[type="checkbox"], ' +
          'input[type="file"]',
      ))
  )
    return;
  // Ignore Enter events from nested `.sheet` elements in the React tree.
  const sheet = target.closest('.sheet');
  if (sheet !== event.currentTarget) return;
  const primaries = sheet.querySelectorAll<HTMLButtonElement>(
    ':scope > .ft .btn.primary',
  );
  const button = primaries[primaries.length - 1];
  if (!button) return;
  event.preventDefault();
  if (button.disabled || button.getAttribute('aria-disabled') === 'true')
    return;
  button.click();
}

/**
 * Tracks whether a scrollable element has content below the visible viewport so
 * the footer can render a top border when the sheet body overflows (for example,
 * at the 640px minimum window height).
 */
function useMoreBelow<E extends HTMLElement>(): {
  ref: RefObject<E | null>;
  below: boolean;
} {
  const ref = useRef<E | null>(null);
  const [below, setBelow] = useState(false);
  useEffect(() => {
    const node = ref.current;
    if (!node) return;
    const measure = (): void =>
      setBelow(node.scrollTop + node.clientHeight < node.scrollHeight - 1);
    measure();
    node.addEventListener('scroll', measure, { passive: true });
    const observer =
      typeof ResizeObserver === 'undefined'
        ? null
        : new ResizeObserver(measure);
    observer?.observe(node);
    for (const child of node.children) observer?.observe(child);
    return () => {
      node.removeEventListener('scroll', measure);
      observer?.disconnect();
    };
  }, []);
  return { ref, below };
}

export interface SheetDialogProps extends Omit<SheetProps, 'titleId'> {
  /**
   * An `alertdialog` is for a sheet that must be confirmed — a removal or
   * revocation. Everything else is a plain dialog.
   */
  danger?: boolean;
  /**
   * Called when the sheet is dismissed. Omit it to prevent dismissal, which is
   * required for an unrecoverable failure.
   */
  onClose?: () => void;
  /** False while a write is in flight, so a click outside cannot abandon it. */
  dismissible?: boolean;
}

export function SheetDialog({
  danger = false,
  onClose,
  dismissible = true,
  ...sheet
}: SheetDialogProps): ReactNode {
  const titleId = useId();
  const role = danger ? 'alertdialog' : 'dialog';
  const panel = <Sheet {...sheet} titleId={titleId} />;
  if (!onClose) {
    return (
      <Dialog className="backdrop" role={role} titleId={titleId}>
        {panel}
      </Dialog>
    );
  }
  return (
    <DismissibleDialog
      className="backdrop"
      role={role}
      titleId={titleId}
      onDismiss={onClose}
      dismissible={dismissible}
    >
      {panel}
    </DismissibleDialog>
  );
}
