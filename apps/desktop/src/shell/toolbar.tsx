/**
 * The Files list bar's add control: a borderless plus that opens the New
 * menu. The menu states the native menu's key equivalents beside each item
 * (⌘N, ⇧⌘N); the keys themselves are bound by the shell and by
 * `install_macos_menu`, not by this menu.
 */

import type { ReactNode } from 'react';
import { MenuButton, MenuItem } from '../components';
import type { KindFilter } from '../location';

export interface NewItemButtonProps {
  onNew: (kind: Exclude<KindFilter, 'All'>) => void;
  /** Opens the New folder sheet; omitted where no folder can be made. */
  onNewFolder?: () => void;
  /** Tooltip message explaining why item creation is disabled, if applicable. */
  reason?: string | null;
  disabled?: boolean;
}

export function NewItemButton({
  onNew,
  onNewFolder,
  reason = null,
  disabled = false,
}: NewItemButtonProps): ReactNode {
  return (
    <MenuButton
      label=""
      aria-label="New"
      variant="quiet"
      icon="plus"
      trailingIcon={null}
      menuLabel="Create item"
      align="end"
      className="newwrap"
      disabled={disabled || reason !== null}
      title={reason ?? 'New item'}
    >
      {() => (
        <>
          <MenuItem icon="key" shortcut="⌘N" onClick={() => onNew('Password')}>
            New Password
          </MenuItem>
          <MenuItem
            icon="fileText"
            shortcut="⇧⌘N"
            onClick={() => onNew('Document')}
          >
            New Document
          </MenuItem>
          {onNewFolder ? (
            <>
              <div className="menu-separator" role="separator" />
              <MenuItem icon="folder" onClick={onNewFolder}>
                New Folder…
              </MenuItem>
            </>
          ) : null}
        </>
      )}
    </MenuButton>
  );
}
