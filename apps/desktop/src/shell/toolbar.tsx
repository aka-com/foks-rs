/**
 * The primary New control.
 *
 * It used to sit in a toolbar of its own across the top of the folder
 * browser, beside the kind filter and a scoped search field. The header row
 * carries the search field and this button now, and the kind filter belongs
 * to the tree column, so nothing is left here but the control itself — the
 * header draws it, and the browser's empty states draw it again where the
 * reader is already looking.
 */

import type { ReactNode } from 'react';
import { KindIcon, MenuButton } from '../components';
import { KIND_LIST, kindLabel } from '../model';
import type { KindFilter } from '../location';

export interface NewItemButtonProps {
  onNew: (kind: Exclude<KindFilter, 'All'>) => void;
  /**
   * The store or folder the items screen would save into, named in a header
   * line above the kinds. Omitted where the destination is not settled, in
   * which case the menu is the kinds alone.
   */
  destination?: string;
  /** Tooltip message explaining why item creation is disabled, if applicable. */
  reason?: string | null;
  disabled?: boolean;
}

/** Primary button that displays a dropdown menu of item kinds to create. */
export function NewItemButton({
  onNew,
  destination,
  reason = null,
  disabled = false,
}: NewItemButtonProps): ReactNode {
  return (
    <MenuButton
      label="New"
      variant="primary"
      icon="plus"
      menuLabel="Create item"
      align="end"
      className="newwrap"
      disabled={disabled || reason !== null}
      {...(reason ? { title: reason } : {})}
    >
      {(close) => (
        <>
          {destination ? <div className="mh">New in {destination}</div> : null}
          {KIND_LIST.map((name) => (
            <button
              key={name}
              type="button"
              role="menuitem"
              className="kind-menu-item"
              onClick={() => {
                close();
                onNew(name);
              }}
            >
              <KindIcon kind={name} />
              {kindLabel(name)}
            </button>
          ))}
        </>
      )}
    </MenuButton>
  );
}
