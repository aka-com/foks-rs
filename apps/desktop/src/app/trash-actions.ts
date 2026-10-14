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
  ownsTeam,
  storeOf,
  trashEntries,
  trashEntryExpired,
  trashUnavailable,
  UNDO_DURATION_MS,
  type AgentSnapshot,
  type Item,
  type TrashEntry,
} from '../model';
import type { MutationFailureHandler } from '../mutation-recovery';
import { itemActionProblem } from '../screens/store-access';
import type { WriteWorkflow } from '../screens/write-workflows';
import { matchesShortcut, typingTarget } from '../shortcuts';
import { storedTrashAutoEmpty } from '../trash-auto-empty';

/** How often automatic emptying looks at one store's Trash again. */
const AUTO_EMPTY_INTERVAL_MS = 60 * 60 * 1000;

/**
 * How long the catalog stays unchanged before automatic emptying starts.
 * Every completed change publishes a new catalog, so the job does not take
 * the host's single mutation permit right after one.
 */
const AUTO_EMPTY_QUIET_MS = 3_000;

const DAY_MS = 24 * 60 * 60 * 1000;

/** Why a folder cannot be deleted in a team whose Trash is not set up. */
const FOLDER_WITHOUT_TRASH =
  'Folders can be deleted only through Trash, which is not set up in this team yet. An owner sets it up by moving any item to Trash.';

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
  accessNow,
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
  accessNow: () => number;
}): TrashActions {
  const pendingUndo = useRef<{ run: () => void; until: number } | null>(null);
  // A Trash action the person started; a second one started meanwhile is
  // ignored.
  const busy = useRef(false);
  const undoing = useRef(false);
  // Automatic emptying while it holds the host's mutation permit. A Trash
  // action started meanwhile waits for it rather than being refused.
  const emptying = useRef<Promise<void> | null>(null);
  const autoEmptied = useRef(new Map<string, number>());

  const restoreTo = useCallback(
    async (item: Item, destination: string): Promise<void> => {
      const profile = storeOf(shown, item.store)?.server;
      await emptying.current;
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
      undoing.current = true;
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
      })()
        .catch((error: unknown) => mutationError(error, { item: original }))
        .finally(() => {
          undoing.current = false;
        });
    },
    [mutationError, refreshSnapshot, restoreItem, toasts],
  );

  /**
   * Offers a member who cannot set up a team's Trash permanent deletion of
   * the item instead, after confirmation. Folders are deleted only through
   * Trash.
   */
  const deleteWithoutTrash = useCallback(
    (item: Item) => {
      if (item.kind === 'Folder')
        toasts.show(FOLDER_WITHOUT_TRASH, { tone: 'warning' });
      else setWorkflow({ kind: 'delete', item, outsideTrash: true });
    },
    [setWorkflow, toasts],
  );

  const moveToTrash = useCallback(
    (item: Item) => {
      if (busy.current) return;
      if (trashUnavailable(shown, item.store)) {
        deleteWithoutTrash(item);
        return;
      }
      busy.current = true;
      const name = nameOf(item.path);
      void (async () => {
        let path: string;
        await emptying.current;
        try {
          ({ path } = await bridge.trashItem({
            storeId: item.store,
            path: item.path,
            version: item.version,
            direntId: item.direntId,
          }));
        } catch (error) {
          const { code } = normalizeCommandError(error);
          if (code === 'trash-unavailable' && !ownsTeam(shown, item.store)) {
            // The roster did not show that this account cannot create
            // Trash, but the host's attempt found it. An owner's refusal
            // has another cause and is reported as it is.
            await mutationError(error, { report: false });
            deleteWithoutTrash(item);
          } else if (code === 'conflict') {
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
        // The toast's button and ⌘Z share one Undo; whichever comes first
        // runs it, and ⌘Z replaces the toast so its button is gone.
        const dedupeKey = `trash-undo:${item.store}|${path}`;
        let undone = false;
        const run = (fromKeyboard = false) => {
          if (undone) return;
          undone = true;
          if (fromKeyboard) toasts.show(`Restoring ${name}…`, { dedupeKey });
          undo(item, path);
        };
        // The toast closes when ⌘Z stops working, even while it is held.
        const until = Date.now() + UNDO_DURATION_MS;
        pendingUndo.current = { run: () => run(true), until };
        await refresh(
          `Moved ${name} to Trash`,
          storeOf(shown, item.store)?.server,
          {
            action: { label: 'Undo', onAction: () => run() },
            durationMs: UNDO_DURATION_MS,
            expiresAt: until,
            dedupeKey,
          },
        );
      })()
        .catch((error: unknown) => mutationError(error, { item }))
        .finally(() => {
          busy.current = false;
        });
    },
    [
      bridge,
      deleteWithoutTrash,
      mutationError,
      refresh,
      refreshSnapshot,
      shown,
      toasts,
      undo,
    ],
  );

  // Automatic emptying, when this device opts in: entries deleted longer ago
  // than the chosen age are removed permanently, one store at a time, and
  // only those this account can change. It starts once the catalog has been
  // unchanged for a short time and skips while a dialog is open, a Trash
  // action or an Undo runs, or an Undo is offered, so it does not take the
  // host's mutation permit next to a change the person makes.
  useEffect(() => {
    const days = storedTrashAutoEmpty();
    if (days === null) return;
    const offered = pendingUndo.current;
    const delay = Math.max(
      AUTO_EMPTY_QUIET_MS,
      offered ? offered.until - Date.now() : 0,
    );
    const timer = window.setTimeout(() => {
      const pending = pendingUndo.current;
      if (
        busy.current ||
        undoing.current ||
        emptying.current ||
        (pending && Date.now() <= pending.until) ||
        anyDialogOpen()
      )
        return;
      const now = Date.now();
      for (const store of shown.stores) {
        const last = autoEmptied.current.get(store.id);
        if (last !== undefined && now - last < AUTO_EMPTY_INTERVAL_MS) continue;
        const expired = trashEntries(shown, store.id).filter(
          (entry) =>
            trashEntryExpired(entry, now, days * DAY_MS) &&
            !itemActionProblem(shown, entry.item, true, accessNow()),
        );
        if (!expired.length) continue;
        autoEmptied.current.set(store.id, now);
        // The job ends after the refresh that follows it: the host
        // discards the store's catalog when it writes, so a Trash action
        // waiting for the job selects its item in a catalog read again.
        const job = bridge
          .emptyTrash({
            storeId: store.id,
            entries: expired.map(({ item }) => ({
              path: item.path,
              version: item.version,
              direntId: item.direntId,
            })),
          })
          .then(
            () =>
              refresh(
                `Deleted ${expired.length} ${expired.length === 1 ? 'entry' : 'entries'} permanently from Trash in ${store.name}: they were deleted more than ${days} days ago.`,
                store.server,
              ),
            (error: unknown) => {
              // Another change held the permit: try again at the next
              // catalog change rather than an hour later.
              if (normalizeCommandError(error).code === 'mutation-in-flight')
                autoEmptied.current.delete(store.id);
              return mutationError(error, { report: false });
            },
          )
          .catch((error: unknown) => mutationError(error, { report: false }))
          .finally(() => {
            if (emptying.current === job) emptying.current = null;
          });
        emptying.current = job;
        return;
      }
    }, delay);
    return () => window.clearTimeout(timer);
  }, [accessNow, bridge, mutationError, refresh, shown]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      const pending = pendingUndo.current;
      if (
        !pending ||
        !matchesShortcut(event, 'undoTrash') ||
        // A text field keeps its own ⌘Z.
        typingTarget(event.target) ||
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
