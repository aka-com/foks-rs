import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Field, Inset, SectionLabel, SheetDialog } from '../components';
import { normalizeCommandError } from '../bridge';
import { isTrashPath, nameOf, storeOf } from '../model';
import type { Item } from '../model';
import { noteMoved } from '../recent-favorites';
import { useSheetGuard } from '../navigation-guard';
import { FolderPicker, pickerFolders } from './folder-picker';
import {
  baseName,
  checkMove,
  checkMoveMany,
  folderExists,
  inFolder,
  itemCount,
  joinPath,
  nameProblem,
  parentOf,
  toFolder,
  validFolderPath,
  validItemPath,
} from './item-moves';
import type { MoveCheck } from './item-moves';
import { newFolderRoles } from './item-roles';
import { itemActionProblem } from './store-access';
import type { WriteWorkflow } from './write-workflows';
import { useShellServices, useShellSnapshot } from '../shell-services';

/**
 * Why the sheet cannot move `items`, given each item's reason or undefined:
 * one item's reason, and of several only a reason all of them have. The
 * progress sheet refuses the others one by one, as it does for a drag.
 */
function blockingProblem(
  problems: readonly (string | undefined)[],
): string | undefined {
  return problems.length === 1 || problems.every(Boolean)
    ? problems.find(Boolean)
    : undefined;
}

/**
 * Names the chosen items a move of several leaves where they are, and why.
 * `problems` holds each item's reason, or undefined for one that moves.
 */
function leftOutNote(
  items: readonly Item[],
  problems: readonly (string | undefined)[],
): string | null {
  const left = items.filter((_, index) => problems[index]);
  const reason = problems.find(Boolean);
  if (!left.length || !reason) return null;
  if (left.length === 1)
    return `${nameOf(left[0].path)} will not move. ${reason}`;
  const names = left.slice(0, 3).map((item) => nameOf(item.path));
  const more = left.length - names.length;
  return `${left.length} items will not move: ${names.join(', ')}${more ? ` and ${more} more` : ''}. The progress sheet lists the reason for each.`;
}

/**
 * Renames one item or moves it, or moves several items of one store, to a
 * folder of the same store. The folder comes from the store's tree, where
 * New folder creates one in place, or from a typed path behind the Path
 * disclosure; a typed folder that does not exist yet is created, with its
 * parents, before the move. One item moves here; several hand over to the
 * progress sheet, which moves them one at a time.
 */
export function MoveSheet({
  items,
  setWorkflow,
}: {
  items: readonly Item[];
  setWorkflow: (workflow: WriteWorkflow) => void;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const {
    bridge,
    refresh: onApplied,
    mutationError: onMutationError,
    reloadCatalog: onReload,
    accessNow,
  } = useShellServices();
  const first = items[0];
  const single = items.length === 1 ? first : null;
  const store = first ? storeOf(snapshot, first.store) : undefined;
  const current =
    first && items.every((item) => parentOf(item.path) === parentOf(first.path))
      ? parentOf(first.path)
      : null;
  const [folder, setFolder] = useState(current ?? '/');
  const [name, setName] = useState(single ? baseName(single.path) : '');
  // The Path disclosure's text while it is open: the full destination of
  // one item, or the folder of several.
  const [typed, setTyped] = useState<string | null>(null);
  // Folders created from this sheet, before a catalog read lists them.
  const [known, setKnown] = useState<readonly string[]>([]);
  const [newFolder, setNewFolder] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [uncertain, setUncertain] = useState(false);
  const busy = saving || creating;
  useSheetGuard(
    busy ? { verdict: 'refuse', reason: 'Wait for the move to finish.' } : null,
    !busy,
  );
  if (!first || !store) return null;

  const destination =
    typed ?? (single ? joinPath(folder, name.trim()) : folder);
  const typedName = single && typed === null ? nameProblem(name) : null;
  const check: MoveCheck = typedName
    ? { ok: false, problem: typedName }
    : single
      ? checkMove(snapshot, single, destination, known)
      : checkMoveMany(snapshot, items, destination, known);
  const destinationFolder = single ? parentOf(destination) : destination;
  const pickerValue =
    validFolderPath(destinationFolder) &&
    folderExists(snapshot, store.id, destinationFolder, known)
      ? destinationFolder
      : null;
  const problems = items.map((item) =>
    itemActionProblem(snapshot, item, true, accessNow()),
  );
  const problem = blockingProblem(problems);
  const leftOut = problem ? null : leftOutNote(items, problems);
  const moving = items
    .filter((item) => item.kind === 'Folder')
    .map((item) => item.path);

  const pick = (path: string): void => {
    setFolder(path);
    setError(null);
    if (typed !== null)
      setTyped(single ? joinPath(path, baseName(typed) || name) : path);
  };
  const togglePath = (): void => {
    if (typed === null) {
      setTyped(destination);
      return;
    }
    // Closing keeps what was typed when it reads as a destination.
    if (single && validItemPath(typed)) {
      setFolder(parentOf(typed));
      setName(baseName(typed));
    } else if (!single && validFolderPath(typed)) setFolder(typed);
    setTyped(null);
  };

  const parentForNew = pickerValue ?? '/';
  const createFolder = async (): Promise<void> => {
    if (newFolder === null || creating) return;
    const invalid = nameProblem(newFolder);
    if (invalid) {
      setError(invalid);
      return;
    }
    const path = joinPath(parentForNew, newFolder.trim());
    if (isTrashPath(path)) {
      setError('Trash uses this name at the top level. Enter another name.');
      return;
    }
    if (
      known.includes(path) ||
      snapshot.items.some(
        (item) => item.store === store.id && item.path === path,
      )
    ) {
      setError(
        `Something named ${newFolder.trim()} is already ${inFolder(parentForNew)}.`,
      );
      return;
    }
    setCreating(true);
    setError(null);
    try {
      await bridge.createFolder({
        storeId: store.id,
        path,
        ...newFolderRoles(store),
      });
    } catch (failure) {
      setError(normalizeCommandError(failure).message);
      await onMutationError(failure, { report: false });
      setCreating(false);
      return;
    }
    setKnown((list) => [...list, path]);
    setNewFolder(null);
    pick(path);
    setCreating(false);
    try {
      await onApplied(`Folder created in ${store.name}`, store.server);
    } catch (failure) {
      await onMutationError(failure, { report: false });
    }
  };

  const submit = async (): Promise<void> => {
    if (busy || uncertain || !check.ok) return;
    const blocked = blockingProblem(
      items.map((item) => itemActionProblem(snapshot, item, true, accessNow())),
    );
    if (blocked) {
      setError(blocked);
      return;
    }
    setSaving(true);
    setError(null);
    if (check.creates) {
      try {
        await bridge.createFolder({
          storeId: store.id,
          path: check.creates,
          ...newFolderRoles(store),
        });
      } catch (failure) {
        // Another session may have created it meanwhile; the move decides.
        const typed = normalizeCommandError(failure);
        if (typed.code !== 'already-exists') {
          setError(typed.message);
          await onMutationError(failure, { report: false });
          setSaving(false);
          return;
        }
      }
    }
    if (!single) {
      setWorkflow({ kind: 'bulk-move', items, folder: destination });
      return;
    }
    let target = single;
    try {
      // A folder created from this sheet retired the host's catalog of the
      // store, and the host selects the item in a catalog it has read.
      if (check.creates || known.length) {
        const next = await onReload(store.server);
        const fresh = next.items.find(
          (item) =>
            item.store === single.store &&
            item.path === single.path &&
            item.direntId === single.direntId,
        );
        if (!fresh) {
          setError(
            'This item was changed or removed elsewhere. Close this sheet and review it before trying again.',
          );
          setUncertain(true);
          setSaving(false);
          return;
        }
        target = fresh;
      }
      await bridge.moveItem({
        direntId: target.direntId,
        storeId: target.store,
        path: target.path,
        version: target.version,
        destination,
      });
      noteMoved(target, destination);
    } catch (failure) {
      const typed = normalizeCommandError(failure);
      setError(
        typed.code === 'conflict'
          ? 'The source changed or the destination already exists. Close this sheet and refresh before trying again.'
          : typed.message,
      );
      setUncertain(Boolean(typed.ambiguous) || typed.code === 'conflict');
      await onMutationError(failure, { item: target, report: false });
      setSaving(false);
      return;
    }
    // A failed refresh must never offer to repeat an already acknowledged move.
    setWorkflow(null);
    try {
      await onApplied(
        parentOf(destination) === parentOf(single.path)
          ? `Renamed ${nameOf(single.path)} to ${nameOf(destination)}`
          : `Moved ${nameOf(single.path)} ${toFolder(parentOf(destination))}`,
        store.server,
      );
    } catch (failure) {
      await onMutationError(failure, { report: false });
    }
  };

  return (
    <SheetDialog
      title={single ? 'Rename or move' : `Move ${itemCount(items.length)}`}
      onClose={() => setWorkflow(null)}
      dismissible={!busy}
      width="mid"
      footer={
        <>
          <span
            className={check.ok ? 'move-preview' : 'move-preview bad'}
            aria-live="polite"
          >
            {check.ok ? check.summary : check.problem}
          </span>
          <Button disabled={busy} onClick={() => setWorkflow(null)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={busy || uncertain || !check.ok || Boolean(problem)}
            onClick={() => {
              void submit();
            }}
          >
            {saving ? 'Moving…' : 'Move'}
          </Button>
        </>
      }
    >
      <p>
        {single
          ? `Rename ${nameOf(single.path)}, or move it to another folder of ${store.name}.`
          : `Move ${itemCount(items.length)} to one folder of ${store.name}. Each keeps its name.`}
      </p>
      {single ? (
        <Inset>
          <Field
            label="Name"
            value={typed === null ? name : baseName(typed)}
            disabled={busy || uncertain || typed !== null}
            verbatim
            onChange={(value) => {
              setName(value);
              setError(null);
            }}
          />
        </Inset>
      ) : null}
      <SectionLabel
        action={
          <button
            type="button"
            className="path-toggle"
            aria-expanded={typed !== null}
            disabled={busy}
            onClick={togglePath}
          >
            {typed === null ? 'Path' : 'Hide Path'}
          </button>
        }
      >
        Folder
      </SectionLabel>
      <FolderPicker
        label={`Destination folder in ${store.name}`}
        storeName={store.name}
        folders={pickerFolders(snapshot, store.id, known)}
        value={pickerValue}
        current={current}
        moving={moving}
        disabled={busy || uncertain}
        onChange={pick}
      />
      <div className="picker-actions">
        <Button
          size="sm"
          icon="plus"
          disabled={busy || uncertain || newFolder !== null}
          onClick={() => setNewFolder('')}
        >
          New folder
        </Button>
      </div>
      {newFolder !== null ? (
        <Inset>
          <Field
            label="New folder"
            value={newFolder}
            placeholder="Folder name"
            disabled={creating}
            verbatim
            hint={`Created ${inFolder(parentForNew)}`}
            onChange={(value) => {
              setNewFolder(value);
              setError(null);
            }}
            action={
              <>
                <Button
                  size="sm"
                  disabled={creating}
                  onClick={() => setNewFolder(null)}
                >
                  Cancel
                </Button>
                <Button
                  size="sm"
                  variant="primary"
                  disabled={creating || !newFolder.trim()}
                  onClick={() => {
                    void createFolder();
                  }}
                >
                  {creating ? 'Creating…' : 'Create'}
                </Button>
              </>
            }
          />
        </Inset>
      ) : null}
      {typed !== null ? (
        <Inset>
          <Field
            label={single ? 'Destination path' : 'Destination folder'}
            value={typed}
            mono
            verbatim
            disabled={busy || uncertain}
            onChange={(value) => {
              setTyped(value);
              setError(null);
            }}
          />
        </Inset>
      ) : null}
      {leftOut ? <p>{leftOut}</p> : null}
      {problem || error ? <p role="alert">{problem ?? error}</p> : null}
    </SheetDialog>
  );
}
