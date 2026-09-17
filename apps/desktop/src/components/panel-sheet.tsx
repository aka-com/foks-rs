/**
 * Sheet presentation for the account panels reached from the Account tab.
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
  /** Set only when the page that opened the sheet does not name the account. */
  subtitle?: string;
  onClose: () => void;
}

export interface PanelSheetProps {
  presentation: PanelPresentation;
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
  busy = false,
  glyph,
  footer,
  children,
}: PanelSheetProps): ReactNode {
  return (
    <SheetDialog
      width="wide"
      title={presentation.title}
      subtitle={presentation.subtitle}
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
