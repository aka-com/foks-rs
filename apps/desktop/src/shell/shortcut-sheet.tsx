/**
 * The keyboard shortcut sheet: every entry of the registry in `shortcuts.ts`,
 * grouped by where it applies, with the keys this platform uses. ⌘/ or `?`
 * opens it, as does Keyboard shortcuts in the search palette's Actions.
 */

import { Fragment, useEffect, useId, useRef } from 'react';
import type { ReactNode } from 'react';
import {
  anyDialogOpen,
  useDismissOnOverlayBlock,
} from '../../kit/overlay-primitives';
import { Button, SheetDialog } from '../components';
import {
  SHORTCUTS,
  SHORTCUT_SCOPES,
  chordLabel,
  isMac,
  listedChords,
  matchesShortcut,
  typingTarget,
} from '../shortcuts';
import type { ShortcutId, ShortcutScope } from '../shortcuts';

/**
 * Opens the sheet on ⌘/ (Ctrl / elsewhere), or on `?` outside a text field,
 * where it would be typed. Neither opens it over another dialog.
 */
export function useShortcutSheetKey(onOpen: () => void): void {
  const onOpenRef = useRef(onOpen);
  onOpenRef.current = onOpen;
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || !matchesShortcut(event, 'shortcuts'))
        return;
      if (event.key === '?' && typingTarget(event.target)) return;
      if (anyDialogOpen()) return;
      event.preventDefault();
      onOpenRef.current();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, []);
}

function ScopeTable({
  scope,
  label,
  mac,
}: {
  scope: ShortcutScope;
  label: string;
  mac: boolean;
}): ReactNode {
  const captionId = useId();
  const rows = (Object.keys(SHORTCUTS) as ShortcutId[])
    .filter((id) => SHORTCUTS[id].scope === scope)
    .map((id) => ({ id, chords: listedChords(id, mac) }))
    .filter(({ chords }) => chords.length > 0);
  if (!rows.length) return null;
  return (
    <table className="shortcut-table" aria-labelledby={captionId}>
      <caption id={captionId}>{label}</caption>
      <tbody>
        {rows.map(({ id, chords }) => (
          <tr key={id}>
            <th scope="row">{SHORTCUTS[id].label}</th>
            <td>
              {chords.map((chord, index) => (
                <Fragment key={index}>
                  {index ? ' ' : null}
                  <kbd>{chordLabel(chord, mac)}</kbd>
                </Fragment>
              ))}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function ShortcutSheet({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}): ReactNode {
  useDismissOnOverlayBlock(onClose, open);
  if (!open) return null;
  const mac = isMac();
  return (
    <SheetDialog
      title="Keyboard shortcuts"
      width="wide"
      onClose={onClose}
      footer={
        <Button variant="primary" onClick={onClose} data-dialog-autofocus>
          Done
        </Button>
      }
    >
      <div className="shortcut-sheet">
        {SHORTCUT_SCOPES.map(({ id, label }) => (
          <ScopeTable key={id} scope={id} label={label} mac={mac} />
        ))}
      </div>
    </SheetDialog>
  );
}
