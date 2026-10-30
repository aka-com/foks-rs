/**
 * Modal alert dialog styled after macOS 11+: a 272px centered card with an
 * icon, short title, message, and two equal-width buttons (`.alert` in
 * shell.css).
 *
 * The background window is inert while the alert is open. Tab cycles within the
 * alert, clicking the backdrop is ignored, Escape cancels, and closing restores
 * focus to the trigger control.
 *
 * Return activates the default button from anywhere in the dialog, and Space
 * activates the focused button. For destructive alerts, Cancel is the default
 * button and receives initial focus so Return never confirms the destructive
 * action; the destructive button uses danger text styling and responds to ⌘⌫.
 */

import { useId, useRef } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { DismissibleDialog } from '/kit/overlay-primitives';
import { Button } from './button';
import { Icon } from './icon';
import type { FoksIconName } from '../icons';

export interface AlertDialogProps {
  title: ReactNode;
  /** Supporting text displayed below the title. */
  message?: ReactNode;
  /** Additional content below the message, such as a list of affected items. */
  children?: ReactNode;
  /** Title-case label for the confirm button ("Delete", "Discard"). */
  confirmLabel: string;
  cancelLabel?: string;
  /** Styles confirm as danger, makes Cancel the default button, and binds ⌘⌫ to confirm. */
  destructive?: boolean;
  /** Leading icon name; defaults to the application mark. */
  icon?: FoksIconName;
  /**
   * Disables both buttons and blocks Escape and Return while the action is in
   * progress.
   */
  busy?: boolean;
  /** Disables the confirm button (e.g. when access is blocked or state is uncertain). */
  confirmDisabled?: boolean;
  /** Tooltip for the confirm button, such as why it is disabled. */
  confirmTitle?: string;
  /** ID of an element that describes the confirm button. */
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
  // Place the default button at the trailing edge (Cancel for destructive
  // alerts, confirm otherwise).
  return (
    <DismissibleDialog
      className="backdrop alert-back"
      role="alertdialog"
      titleId={titleId}
      // Associate the message element so screen readers announce both title and message.
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
