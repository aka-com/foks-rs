/**
 * Folder context menu shared by the sidebar store tree and folder rows in the
 * Files list.
 *
 * Any element with `data-folder-store` and `data-folder-path` opens the menu on
 * right-click, ContextMenu, or Shift-F10, offering New Folder, Rename or Move…,
 * Move Folder to Trash, and team info and membership actions for team stores. Opening the menu preserves the current selection, and
 * closing it restores focus to the trigger row.
 */

import { useEffect, useRef, useState } from 'react';
import type { KeyboardEvent, MouseEvent, ReactNode } from 'react';
import { anyDialogOpen, ContextMenu, Menu } from '/kit/overlay-primitives';
import { useOptionalToast } from '/kit/toasts';
import { MenuItem } from '../components';
import {
  isTrashPath,
  storeAvailability,
  storeDescription,
  storeOf,
} from '../model';
import type { AgentSnapshot, Item, Store } from '../model';
import type { Location } from '../location';
import { manageReason } from './group-model';
import { itemActionProblem } from './store-access';
import { writeBlockReason } from './write-workflows';

export interface FolderMenuActions {
  onNewFolder?: (storeId: string, path: string) => void;
  onMove?: (item: Item) => void;
  /** Moves a folder with everything in it to its store's Trash. */
  onTrash?: (item: Item) => void;
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
  /** Event handlers to spread on the container of `[data-folder-store]` elements. */
  handlers: {
    onContextMenu: (event: MouseEvent<HTMLElement>) => void;
    onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
  };
  /** Rendered context menu element, or `null` when closed. */
  menu: ReactNode;
}

export function useFolderMenu(
  snapshot: AgentSnapshot,
  {
    onNewFolder,
    onMove,
    onTrash,
    onTeamInfo,
    onNavigate,
    accessNow = () => Date.now() / 1000,
  }: FolderMenuActions,
): FolderMenu {
  const toasts = useOptionalToast();
  const [open, setOpen] = useState<OpenMenu | null>(null);
  const anchor = useRef<HTMLElement>(null);
  // Mark the trigger row with `.menu-target` while its context menu is open.
  useEffect(() => {
    const target = open?.trigger;
    target?.classList.add('menu-target');
    return () => target?.classList.remove('menu-target');
  }, [open]);

  const show = (target: EventTarget, x: number, y: number): boolean => {
    if (!(target instanceof Element) || target.closest('[role="menu"]'))
      return false;
    const row = target.closest<HTMLElement>('[data-folder-store]');
    if (!row?.dataset.folderStore || !row.dataset.folderPath) return false;
    // Trash is not a folder to create in, move or delete.
    if (isTrashPath(row.dataset.folderPath)) return false;
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
          New Folder…
        </MenuItem>
        {open.path !== '/' && onMove ? (
          <MenuItem
            icon="pencil"
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
            Rename or Move…
          </MenuItem>
        ) : null}
        {open.path !== '/' && onTrash ? (
          <MenuItem
            icon="trash"
            danger
            reason={
              folder
                ? itemActionProblem(snapshot, folder, true, accessNow())
                : 'Refresh this vault to load the folder before moving it to Trash.'
            }
            onClick={() => {
              close();
              if (folder && allowMove(folder)) onTrash(folder);
            }}
          >
            Move Folder to Trash
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
              Team Info
            </MenuItem>
            <div role="separator" className="menu-separator" />
            {(
              [
                ['add', 'user', 'Add FOKS User…', 'roster'],
                ['add-team', 'users', 'Add FOKS Team…', 'federation'],
                ['invite', 'door', 'Invite New User…', 'roster'],
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
