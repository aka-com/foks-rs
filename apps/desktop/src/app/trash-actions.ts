import {
  useCallback,
  useEffect,
  useRef,
  type Dispatch,
  type SetStateAction,
} from 'react';
import type { ToastController, ToastOptions } from '/kit/toasts';
import { anyDialogOpen } from '/kit/overlay-primitives';
import { normalizeCommandError, type Bridge } from '../bridge';
import type { LocationStore } from '../location';
import {
  nameOf,
  storeOf,
  type AgentSnapshot,
  type Item,
  type TrashEntry,
} from '../model';
import type { MutationFailureHandler } from '../mutation-recovery';
import type { WriteWorkflow } from '../screens/write-workflows';

/** How long the Undo of a move to Trash is offered. */
export const UNDO_DURATION_MS = 8_000;

export interface TrashActions {
  /** Moves an item, or a folder with its contents, to Trash, offering Undo. */
  moveToTrash: (item: Item) => void;
  /**
   * Moves a Trash entry back to where it was. An occupied original path
   * opens the restore-conflict sheet instead.
   */
  restore: (entry: TrashEntry) => void;
  /** Moves a Trash entry to `destination`; the conflict sheet's step. */
  restoreTo: (item: Item, destination: string) => Promise<void>;
}

function occupied(
  snapshot: AgentSnapshot,
  store: string,
  path: string,
): boolean {
  return snapshot.items.some(
    (item) => item.store === store && item.path === path,
  );
}

/** Whether a key press belongs to a text field, which keeps its own ⌘Z. */
function editing(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName)
  );
}

/**
 * Trash actions shared by the Files list, the folder menu, the details panel
 * and the Trash view. Deleting is a move into Trash, so it asks for no
 * confirmation; the toast offers Undo, and ⌘Z does the same while it shows.
 */
export function useTrashActions({
  bridge,
  shown,
  refresh,
  refreshSnapshot,
  mutationError,
  toasts,
  locations,
  setWorkflow,
}: {
  bridge: Bridge;
  shown: AgentSnapshot;
  refresh: (
    message: string,
    profile?: string,
    options?: ToastOptions,
  ) => Promise<void>;
  refreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
  mutationError: MutationFailureHandler;
  toasts: ToastController;
  locations: LocationStore;
  setWorkflow: Dispatch<SetStateAction<WriteWorkflow>>;
}): TrashActions {
  const pendingUndo = useRef<{ run: () => void; until: number } | null>(null);
  const busy = useRef(false);

  const restoreTo = useCallback(
    async (item: Item, destination: string): Promise<void> => {
      const profile = storeOf(shown, item.store)?.server;
      try {
        await bridge.restoreTrashItem({
          storeId: item.store,
          path: item.path,
          version: item.version,
          direntId: item.direntId,
          destination,
        });
      } catch (error) {
        if (normalizeCommandError(error).code === 'already-exists') {
          await mutationError(error, { report: false });
          setWorkflow({ kind: 'restore-conflict', item, destination });
          return;
        }
        await mutationError(error, { item });
        return;
      }
      await refresh(`Restored ${nameOf(destination)}`, profile, {
        action: {
          label: 'Show',
          onAction: () =>
            locations.navigateAndSelect(
              { kind: 'store', ref: item.store },
              { store: item.store, path: destination },
            ),
        },
      });
    },
    [bridge, locations, mutationError, refresh, setWorkflow, shown],
  );

  const restoreItem = useCallback(
    (snapshot: AgentSnapshot, item: Item, destination: string) => {
      if (occupied(snapshot, item.store, destination)) {
        setWorkflow({ kind: 'restore-conflict', item, destination });
        return Promise.resolve();
      }
      return restoreTo(item, destination);
    },
    [restoreTo, setWorkflow],
  );

  const restore = useCallback(
    (entry: TrashEntry) => {
      if (busy.current) return;
      busy.current = true;
      void (async () => {
        // A hashed name keeps the original path in its `.origin` file. An
        // entry with no recorded location returns to the top level under the
        // name it has in Trash.
        const destination =
          entry.original ??
          (entry.origin
            ? (
                await bridge.readItem({
                  storeId: entry.origin.store,
                  path: entry.origin.path,
                  version: entry.origin.version,
                })
              ).value
            : `/${entry.name}`);
        await restoreItem(shown, entry.item, destination);
      })()
        .catch((error: unknown) => mutationError(error, { item: entry.item }))
        .finally(() => {
          busy.current = false;
        });
    },
    [bridge, mutationError, restoreItem, shown],
  );

  const undo = useCallback(
    (original: Item, trashPath: string) => {
      pendingUndo.current = null;
      void (async () => {
        // The toast outlives the snapshot it was shown with: find the entry
        // in a current catalog, which the host's selection also requires.
        const next = await refreshSnapshot();
        const entry = next.items.find(
          (item) => item.store === original.store && item.path === trashPath,
        );
        if (!entry) {
          toasts.show('This item is no longer in Trash.', { tone: 'warning' });
          return;
        }
        await restoreItem(next, entry, original.path);
      })().catch((error: unknown) => mutationError(error, { item: original }));
    },
    [mutationError, refreshSnapshot, restoreItem, toasts],
  );

  const moveToTrash = useCallback(
    (item: Item) => {
      if (busy.current) return;
      busy.current = true;
      const name = nameOf(item.path);
      void (async () => {
        let path: string;
        try {
          ({ path } = await bridge.trashItem({
            storeId: item.store,
            path: item.path,
            version: item.version,
            direntId: item.direntId,
          }));
        } catch (error) {
          if (normalizeCommandError(error).code === 'conflict') {
            // Changed or already gone: a refresh tells which.
            await mutationError(error, { report: false });
            const next = await refreshSnapshot(true);
            toasts.show(
              next.items.some(
                (candidate) =>
                  candidate.store === item.store &&
                  candidate.path === item.path,
              )
                ? 'This item was changed by another user or session. Review it before moving it to Trash.'
                : 'This item was already moved or deleted elsewhere.',
              { tone: 'warning' },
            );
          } else await mutationError(error, { item });
          return;
        }
        const run = () => undo(item, path);
        pendingUndo.current = { run, until: Date.now() + UNDO_DURATION_MS };
        await refresh(
          `Moved ${name} to Trash`,
          storeOf(shown, item.store)?.server,
          {
            action: { label: 'Undo', onAction: run },
            durationMs: UNDO_DURATION_MS,
          },
        );
      })()
        .catch((error: unknown) => mutationError(error, { item }))
        .finally(() => {
          busy.current = false;
        });
    },
    [bridge, mutationError, refresh, refreshSnapshot, shown, toasts, undo],
  );

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      const pending = pendingUndo.current;
      if (
        !pending ||
        !(event.metaKey || event.ctrlKey) ||
        event.shiftKey ||
        event.altKey ||
        event.key.toLowerCase() !== 'z' ||
        editing(event.target) ||
        anyDialogOpen()
      )
        return;
      if (Date.now() > pending.until) {
        pendingUndo.current = null;
        return;
      }
      event.preventDefault();
      pending.run();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, []);

  return { moveToTrash, restore, restoreTo };
}
