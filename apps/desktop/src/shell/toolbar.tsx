/**
 * Add button in the Files list topbar: a borderless plus button that opens the
 * New item menu. Menu items display the native menu keyboard shortcuts (⌘N,
 * ⇧⌘N), which are handled by the shell and `install_macos_menu`.
 */

import type { ReactNode } from 'react';
import { MenuButton, MenuItem } from '../components';
import type { KindFilter } from '../location';

export interface NewItemButtonProps {
  onNew: (kind: Exclude<KindFilter, 'All'>) => void;
  /** Opens the New Folder sheet; omitted when folder creation is unavailable. */
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
