/**
 * A macOS 11+ alert: a 272px card centred in the window with a mark, a short
 * title, the message and two equal buttons (`.alert` in shell.css).
 *
 * It is modal: the window behind it is inert, Tab stays inside, a click on
 * the scrim does nothing and Escape cancels. The focus returns to the
 * control that opened it.
 *
 * Return activates the default button wherever the focus is, as on macOS,
 * and Space activates the focused one. In a destructive alert the default is
 * Cancel, which also takes the initial focus, so Return never confirms the
 * destructive action; the destructive button is red text and also answers
 * ⌘⌫.
 */

import { useId, useRef } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { DismissibleDialog } from '/kit/overlay-primitives';
import { Button } from './button';
import { Icon } from './icon';
import type { FoksIconName } from '../icons';

export interface AlertDialogProps {
  title: ReactNode;
  /** One or two sentences under the title. */
  message?: ReactNode;
  /** More content under the message, such as a list of what is affected. */
  children?: ReactNode;
  /** The confirming button's title-case label ("Delete", "Discard"). */
  confirmLabel: string;
  cancelLabel?: string;
  /** Red confirm, Cancel as the default, ⌘⌫ as the confirm key. */
  destructive?: boolean;
  /** The mark's glyph; the app mark when omitted. */
  icon?: FoksIconName;
  /**
   * The action is running: both buttons are disabled and neither Escape
   * nor Return acts until it settles.
   */
  busy?: boolean;
  /** The confirm button cannot act (an access problem, an uncertain result). */
  confirmDisabled?: boolean;
  /** The confirm button's tooltip, such as why it cannot act. */
  confirmTitle?: string;
  /** An element that explains the confirm button, by id. */
  confirmDescribedBy?: string;
  onConfirm: () => void;
  onCancel: () => void;
}

export function AlertDialog({
  title,
  message,
  children,
  confirmLabel,
  cancelLabel = 'Cancel',
  destructive = false,
  icon = 'pawPrint',
  busy = false,
  confirmDisabled = false,
  confirmTitle,
  confirmDescribedBy,
  onConfirm,
  onCancel,
}: AlertDialogProps): ReactNode {
  const titleId = useId();
  const messageId = useId();
  const cancelRef = useRef<HTMLButtonElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (busy) return;
    if (
      event.key === 'Enter' &&
      !event.metaKey &&
      !event.ctrlKey &&
      !event.altKey &&
      (event.target as Element).tagName !== 'TEXTAREA'
    ) {
      event.preventDefault();
      (destructive ? cancelRef : confirmRef).current?.click();
      return;
    }
    if (destructive && event.key === 'Backspace' && event.metaKey) {
      event.preventDefault();
      confirmRef.current?.click();
    }
  };
  const cancelButton = (
    <Button
      ref={cancelRef}
      variant={destructive ? 'primary' : 'plain'}
      disabled={busy}
      data-dialog-autofocus={destructive ? 'true' : undefined}
      onClick={onCancel}
    >
      {cancelLabel}
    </Button>
  );
  const confirmButton = (
    <Button
      ref={confirmRef}
      variant={destructive ? 'plain' : 'primary'}
      danger={destructive}
      disabled={busy || confirmDisabled}
      title={confirmTitle}
      aria-describedby={confirmDescribedBy}
      data-dialog-autofocus={destructive ? undefined : 'true'}
      aria-keyshortcuts={destructive ? 'Meta+Backspace' : undefined}
      onClick={onConfirm}
    >
      {confirmLabel}
    </Button>
  );
  // The default button sits at the trailing edge, as macOS draws it: Cancel
  // in a destructive alert, the confirm button otherwise.
  return (
    <DismissibleDialog
      className="backdrop alert-back"
      role="alertdialog"
      titleId={titleId}
      // VoiceOver reads the message with the title, not only the title.
      aria-describedby={message ? messageId : undefined}
      onDismiss={onCancel}
      dismissible={!busy}
      onKeyDown={onKeyDown}
    >
      <div className="alert">
        <span className="alert-mark" aria-hidden="true">
          <Icon name={icon} />
        </span>
        <h2 id={titleId}>{title}</h2>
        {message ? <p id={messageId}>{message}</p> : null}
        {children ? <div className="alert-body">{children}</div> : null}
        <div className="btns">
          {destructive ? confirmButton : cancelButton}
          {destructive ? cancelButton : confirmButton}
        </div>
      </div>
    </DismissibleDialog>
  );
}
