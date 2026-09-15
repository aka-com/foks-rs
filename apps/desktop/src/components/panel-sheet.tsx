/**
 * Sheet presentation for the account panels reached from Settings → Accounts.
 *
 * Each panel keeps its own bridge state machine and renders its form in the
 * sheet body and its actions in the sheet footer. The header carries the
 * workflow name alone; the account it applies to is named in the subtitle.
 */

import type { ReactNode } from 'react';
import { Icon } from './icon';
import { SheetDialog } from './sheet';

export interface PanelPresentation {
  /** The workflow name. Never qualified with the account alias. */
  title: string;
  /** `"{username} on {server}"`, or the alias when no username is known. */
  subtitle: string;
  onClose: () => void;
}

export interface PanelSheetProps {
  presentation: PanelPresentation;
  /** True while a bridge write is in flight; blocks dismissal. */
  busy?: boolean;
  /** The sheet's actions. Primary action last. */
  footer: ReactNode;
  children: ReactNode;
}

export function PanelSheet({
  presentation,
  busy = false,
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
        <span className="server-mark">
          <Icon name="gear" />
        </span>
      }
    >
      <div className="panel-body">{children}</div>
    </SheetDialog>
  );
}
