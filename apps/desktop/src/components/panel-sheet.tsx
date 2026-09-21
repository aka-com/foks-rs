/**
 * Sheet presentation for the account panels reached from Settings › Account.
 *
 * Each panel keeps its own bridge state machine and renders its form in the
 * sheet body and its actions in the sheet footer. The header carries the
 * workflow name alone; the page that opened the sheet names the account.
 */

import type { ReactNode } from 'react';
import { Icon } from './icon';
import { SheetDialog } from './sheet';

export interface PanelPresentation {
  /** The workflow name. Never qualified with the account alias. */
  title: string;
  onClose: () => void;
}

export interface PanelSheetProps {
  presentation: PanelPresentation;
  /**
   * Overrides the workflow name in the header, for a panel whose later steps
   * name what the reader is deciding rather than what they opened.
   */
  title?: ReactNode;
  /** A progress line under the title, for a panel that runs in steps. */
  step?: ReactNode;
  /** True while a bridge write is in flight; blocks dismissal. */
  busy?: boolean;
  /** The mark beside the title. The account panels' gear, by default. */
  glyph?: ReactNode;
  /** The sheet's actions. Primary action last. */
  footer: ReactNode;
  children: ReactNode;
}

export function PanelSheet({
  presentation,
  title,
  step,
  busy = false,
  glyph,
  footer,
  children,
}: PanelSheetProps): ReactNode {
  return (
    <SheetDialog
      width="wide"
      title={title ?? presentation.title}
      step={step}
      dismissible={!busy}
      onClose={presentation.onClose}
      footer={footer}
      glyph={
        glyph ?? (
          <span className="server-mark">
            <Icon name="gear" />
          </span>
        )
      }
    >
      <div className="panel-body">{children}</div>
    </SheetDialog>
  );
}
