/**
 * Modal sheet with standardized header, content, and action footer.
 *
 * `Sheet` renders the panel within an existing dialog. `SheetDialog` supplies
 * the accessible dialog wrapper and supports standard, mid, and wide sizes.
 */

import { useId } from 'react';
import type { ReactNode } from 'react';
import { Dialog, DismissibleDialog } from '/kit/overlay-primitives';

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
  /** The quiet line under the title. */
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
  subtitle,
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
          {subtitle === undefined ? null : <small>{subtitle}</small>}
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
