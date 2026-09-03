/**
 * The design's sheet — `shell.css`'s `.sheet` and its three parts.
 *
 * Every overlay in FOKS is the same panel: a header (`.hd`) carrying a glyph
 * and a title, a scrolling body (`.sb`), and a row of controls (`.ft`). That
 * shape was written out by hand nine times across five screens, and two
 * screens had each grown their own private `SheetFrame` helper with slightly
 * different props. This is the one of them.
 *
 * Two components, because there are genuinely two situations:
 *
 *   `Sheet`        the panel alone, for a caller that already supplies the
 *                  dialog around it (the write workflows do, so their
 *                  `aria-label` stays on the dialog where it belongs);
 *   `SheetDialog`  the panel and its dialog together, which is what every
 *                  other caller wants.
 *
 * Width is a named size rather than a class string, so `wide` and `mid` come
 * from `shell.css` and nobody invents a third by writing `.sheet big`.
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
   * An `alertdialog` is for a sheet the reader must answer — a removal, a
   * revocation. Everything else is a plain dialog.
   */
  danger?: boolean;
  /**
   * Called when the reader dismisses the sheet. Omit it and the sheet cannot
   * be dismissed at all, which is what a hard failure wants.
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
