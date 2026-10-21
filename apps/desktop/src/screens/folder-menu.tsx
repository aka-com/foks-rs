/**
 * The folder context menu, shared by the sidebar's store tree and the folder
 * rows in the Files list.
 *
 * Any element that carries `data-folder-store` and `data-folder-path` opens
 * it on a right click, the ContextMenu key or Shift-F10: New folder, Rename or
 * move…, Delete folder (stated as unsupported), and for a team its info and
 * membership commands. Opening it never changes the selection; closing it
 * returns the focus to the row it was opened from.
 */

import { useRef, useState } from 'react';
import type { KeyboardEvent, MouseEvent, ReactNode } from 'react';
import { anyDialogOpen, ContextMenu, Menu } from '/kit/overlay-primitives';
import { useOptionalToast } from '/kit/toasts';
import { MenuItem } from '../components';
import { storeAvailability, storeDescription, storeOf } from '../model';
import type { AgentSnapshot, Item, Store } from '../model';
import type { Location } from '../location';
import { manageReason } from './group-model';
import { itemActionProblem } from './store-access';
import { writeBlockReason } from './write-workflows';

export interface FolderMenuActions {
  onNewFolder?: (storeId: string, path: string) => void;
  onMove?: (item: Item) => void;
  onTeamInfo?: (storeId: string, trigger?: HTMLElement) => void;
  onNavigate: (location: Location) => void;
  accessNow?: () => number;
}

interface OpenMenu {
  x: number;
  y: number;
  storeId: string;
  path: string;
  trigger: HTMLElement;
}

export interface FolderMenu {
  /** Spread on the element whose descendants carry the folder attributes. */
  handlers: {
    onContextMenu: (event: MouseEvent<HTMLElement>) => void;
    onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
  };
  /** The open menu, or null; render it anywhere. */
  menu: ReactNode;
}

export function useFolderMenu(
  snapshot: AgentSnapshot,
  {
    onNewFolder,
    onMove,
    onTeamInfo,
    onNavigate,
    accessNow = () => Date.now() / 1000,
  }: FolderMenuActions,
): FolderMenu {
  const toasts = useOptionalToast();
  const [open, setOpen] = useState<OpenMenu | null>(null);
  const anchor = useRef<HTMLElement>(null);

  const show = (target: EventTarget, x: number, y: number): boolean => {
    if (!(target instanceof Element) || target.closest('[role="menu"]'))
      return false;
    const row = target.closest<HTMLElement>('[data-folder-store]');
    if (!row?.dataset.folderStore || !row.dataset.folderPath) return false;
    const trigger = row.matches('[role="button"], button')
      ? row
      : (row.querySelector<HTMLElement>('.fselect') ?? row);
    trigger.focus({ preventScroll: true });
    anchor.current = trigger;
    setOpen({
      x,
      y,
      storeId: row.dataset.folderStore,
      path: row.dataset.folderPath,
      trigger,
    });
    return true;
  };

  const handlers: FolderMenu['handlers'] = {
    onContextMenu: (event) => {
      if (show(event.target, event.clientX, event.clientY))
        event.preventDefault();
    },
    onKeyDown: (event) => {
      if (
        event.key !== 'ContextMenu' &&
        !(event.key === 'F10' && event.shiftKey)
      )
        return;
      const bounds = (event.target as HTMLElement).getBoundingClientRect();
      if (show(event.target, bounds.left, bounds.bottom)) {
        event.preventDefault();
        event.stopPropagation();
      }
    },
  };

  if (!open) return { handlers, menu: null };

  const availability = (candidate: Store) =>
    storeAvailability(snapshot, candidate, { nowSeconds: accessNow() });
  const folder = snapshot.items.find(
    (item) =>
      item.store === open.storeId &&
      item.path === open.path &&
      item.kind === 'Folder',
  );
  const store = storeOf(snapshot, open.storeId);
  const newFolderReason = !store
    ? 'This vault is no longer available.'
    : !availability(store).available
      ? storeDescription(snapshot, store)
      : (writeBlockReason(snapshot, store) ??
        (onNewFolder
          ? undefined
          : 'Folder creation is unavailable in this window.'));
  const close = (): void => setOpen(null);
  const allowMove = (item: Item): boolean => {
    const problem = itemActionProblem(snapshot, item, true, accessNow());
    if (!problem) return true;
    toasts?.show(problem, { tone: 'warning' });
    return false;
  };

  const menu = (
    <ContextMenu
      point={open}
      className="menu-portal"
      onClose={() => {
        setOpen(null);
        if (!anyDialogOpen()) anchor.current?.focus({ preventScroll: true });
      }}
    >
      <Menu
        className="menu"
        aria-label="Folder actions"
        anchorRef={anchor}
        initialFocus="first"
        onClose={close}
      >
        <MenuItem
          icon="plus"
          reason={newFolderReason}
          onClick={() => {
            close();
            onNewFolder?.(open.storeId, open.path);
          }}
        >
          New folder
        </MenuItem>
        {open.path !== '/' && onMove ? (
          <MenuItem
            reason={
              folder
                ? itemActionProblem(snapshot, folder, true, accessNow())
                : 'Refresh this vault to load the folder before moving it.'
            }
            onClick={() => {
              close();
              if (folder && allowMove(folder)) onMove(folder);
            }}
          >
            Rename or move…
          </MenuItem>
        ) : null}
        {open.path !== '/' ? (
          <MenuItem
            icon="trash"
            danger
            reason="Deleting folders is not supported by the desktop agent."
          >
            Delete folder
          </MenuItem>
        ) : null}
        {store?.kind === 'team' ? (
          <>
            <div role="separator" className="menu-separator" />
            <MenuItem
              icon="info"
              reason={
                onTeamInfo
                  ? undefined
                  : 'Team info is unavailable in this window.'
              }
              onClick={() => {
                close();
                onTeamInfo?.(store.id, open.trigger);
              }}
            >
              Team info
            </MenuItem>
            <div role="separator" className="menu-separator" />
            {(
              [
                ['add', 'user', 'Add FOKS user…', 'roster'],
                ['add-team', 'users', 'Add FOKS team…', 'federation'],
                ['invite', 'door', 'Invite new user…', 'roster'],
              ] as const
            ).map(([intent, icon, label, source]) => (
              <MenuItem
                key={intent}
                icon={icon}
                reason={manageReason(snapshot, store, source)}
                onClick={() => {
                  close();
                  onNavigate({ kind: 'teams', ref: store.id, open: intent });
                }}
              >
                {label}
              </MenuItem>
            ))}
          </>
        ) : null}
      </Menu>
    </ContextMenu>
  );
  return { handlers, menu };
}
