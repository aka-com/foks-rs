/**
 * Modal sheet with standardized header, content, and action footer.
 *
 * `Sheet` renders the panel within an existing dialog. `SheetDialog` supplies
 * the accessible dialog wrapper and supports standard, mid, and wide sizes.
 */

import { useId } from 'react';
import type { ReactNode } from 'react';
import { Dialog, DismissibleDialog } from '../../kit/overlay-primitives';
import type { FoksIconName } from '../icons';
import { Icon } from './icon';

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
  width = 'base',
  footer,
  titleId,
  children,
}: SheetProps): ReactNode {
  const classes = ['sheet', WIDTH_CLASS[width]].filter(Boolean).join(' ');
  return (
    <div className={classes}>
      <div className="hd">
        {glyph}
        <span className="t">
          <h2 id={titleId}>{title}</h2>
          {step === undefined ? null : <span className="step">{step}</span>}
        </span>
      </div>
      <div className="sb">{children}</div>
      {footer === undefined ? null : <div className="ft">{footer}</div>}
    </div>
  );
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
  /**
   * The id of the element whose text states what the sheet asks, for the
   * dialog's `aria-describedby`, so a screen reader announces it with the title.
   */
  describedBy?: string;
  /**
   * Draws `glyph` as a server mark holding this icon, tinted as a danger mark
   * when `danger` is set. An explicit `glyph` takes precedence.
   */
  icon?: FoksIconName;
}

/** The round server mark the settings and device sheets show beside a title. */
export function ServerMarkGlyph({
  icon,
  danger = false,
}: {
  icon: FoksIconName;
  danger?: boolean;
}): ReactNode {
  return (
    <span className={danger ? 'server-mark danger' : 'server-mark'}>
      <Icon name={icon} />
    </span>
  );
}

export function SheetDialog({
  danger = false,
  onClose,
  dismissible = true,
  describedBy,
  icon,
  glyph,
  ...sheet
}: SheetDialogProps): ReactNode {
  const titleId = useId();
  const role = danger ? 'alertdialog' : 'dialog';
  const panel = (
    <Sheet
      {...sheet}
      glyph={
        glyph ??
        (icon === undefined ? undefined : (
          <ServerMarkGlyph icon={icon} danger={danger} />
        ))
      }
      titleId={titleId}
    />
  );
  if (!onClose) {
    return (
      <Dialog
        className="backdrop"
        role={role}
        titleId={titleId}
        aria-describedby={describedBy}
      >
        {panel}
      </Dialog>
    );
  }
  return (
    <DismissibleDialog
      className="backdrop"
      role={role}
      titleId={titleId}
      aria-describedby={describedBy}
      onDismiss={onClose}
      dismissible={dismissible}
    >
      {panel}
    </DismissibleDialog>
  );
}
