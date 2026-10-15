import { useState } from 'react';
import type { ReactNode } from 'react';
import {
  Band,
  Button,
  CardSelect,
  Field,
  Inset,
  RadioCard,
  RadioGroup,
  SectionLabel,
  SheetDialog,
} from '../../../components';
import type { KvRoleInput } from '../../../bridge';
import { normalizeCommandError } from '../../../bridge';
import {
  canCreateInStore,
  displayPath,
  isTrashPath,
  nameOf,
  serverDisplayLabelForStore,
  storeAvailability,
  storeNavigationOrder,
  storeOf,
  trashUnavailable,
} from '../../../model';
import type { AgentSnapshot, Item, Store } from '../../../model';
import { useSheetGuard } from '../../../navigation-guard';
import { FolderPicker, pickerFolders } from '../folder-picker';
import {
  baseName,
  copyProblem,
  folderExists,
  inFolder,
  joinPath,
  nameProblem,
  parentOf,
} from '../../item-moves';
import { DEFAULT_READ_ROLE, DEFAULT_WRITE_ROLE } from '../../item-roles';
import { itemActionProblem } from '../../store-access';
import { AccessBlock, storeOption, writeBlockReason } from '../../store-choice';
import type { WriteWorkflow } from '../../write-workflow-model';
import { useShellServices, useShellSnapshot } from '../../../shell-services';

/**
 * The vault a copy goes to when the sheet opens: the first team this account
 * can write to, else any other vault it can write to.
 */
function initialTarget(snapshot: AgentSnapshot, item: Item): string {
  const others = storeNavigationOrder(snapshot).filter(
    (store) => store.id !== item.store && canCreateInStore(snapshot, store.id),
  );
  return (others.find((store) => store.kind === 'team') ?? others[0])?.id ?? '';
}

/** The folder of `store` the copy starts in: the item's own, when it exists. */
function initialFolder(
  snapshot: AgentSnapshot,
  item: Item,
  store: string,
): string {
  const folder = parentOf(item.path);
  return folderExists(snapshot, store, folder) ? folder : '/';
}

/**
 * Copies one secret or file into another vault, such as a personal
 * login into a team. The host reads the item and writes the copy, so its
 * content does not pass through this sheet. The copy takes the roles chosen
 * here in a team. Moving the original to Trash afterwards is a second step:
 * when it fails, the copy stays and the original is reported as kept.
 */
export function CopyItemSheet({
  item,
  setWorkflow,
}: {
  item: Item;
  setWorkflow: (workflow: WriteWorkflow) => void;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const {
    bridge,
    refresh: onApplied,
    mutationError: onMutationError,
    accessNow,
  } = useShellServices();
  const source = storeOf(snapshot, item.store);
  const [targetId, setTargetId] = useState(() => initialTarget(snapshot, item));
  const [folder, setFolder] = useState(() =>
    initialFolder(snapshot, item, targetId),
  );
  const [name, setName] = useState(baseName(item.path));
  const [readRole, setReadRole] = useState<KvRoleInput>(DEFAULT_READ_ROLE);
  const [writeRole, setWriteRole] = useState<KvRoleInput>(DEFAULT_WRITE_ROLE);
  const [removeOriginal, setRemoveOriginal] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [uncertain, setUncertain] = useState(false);
  useSheetGuard(
    saving
      ? { verdict: 'refuse', reason: 'Wait for the copy to finish.' }
      : null,
    !saving,
  );
  if (!source) return null;

  const target = storeOf(snapshot, targetId);
  const label = (store: Store): string =>
    serverDisplayLabelForStore(snapshot, store);
  const otherServer = Boolean(target && target.server !== source.server);
  const options = storeNavigationOrder(snapshot)
    .filter((store) => store.id !== item.store)
    .map((store) =>
      storeOption(snapshot, store, store.server !== source.server),
    );
  const destination = joinPath(folder, name.trim());
  const itemName = nameOf(item.path);
  const destinationProblem = (): string | undefined => {
    if (!target)
      return options.length
        ? 'Choose a vault to copy to.'
        : 'There is no other vault to copy to.';
    const blocked = writeBlockReason(snapshot, target);
    if (blocked) return blocked;
    const invalid = nameProblem(name);
    if (invalid) return invalid;
    if (isTrashPath(destination))
      return 'Trash uses this name at the top level. Enter another name.';
    if (
      folderExists(snapshot, target.id, destination) ||
      snapshot.items.some(
        (entry) => entry.store === target.id && entry.path === destination,
      )
    )
      return `Something named ${name.trim()} is already ${inFolder(folder)} of ${target.name}.`;
    return undefined;
  };
  const problem =
    copyProblem(item) ??
    itemActionProblem(snapshot, item, false, accessNow()) ??
    destinationProblem();
  // Moving the original is a Trash move in its own vault, with its rules.
  const moveProblem =
    itemActionProblem(snapshot, item, true, accessNow()) ??
    (trashUnavailable(snapshot, item.store)
      ? `Trash is not set up in ${source.name} yet, so the original cannot be moved there.`
      : undefined);
  const removing = removeOriginal && !moveProblem;
  const summary = target
    ? `Copies ${itemName} to ${displayPath(destination)} in ${target.name}${removing ? ', then moves the original to Trash' : ''}.`
    : '';

  const chooseTarget = (id: string): void => {
    setTargetId(id);
    setFolder(initialFolder(snapshot, item, id));
    setError(null);
  };

  const submit = async (): Promise<void> => {
    if (saving || uncertain || problem || !target) return;
    if (
      !storeAvailability(snapshot, target, { nowSeconds: accessNow() })
        .available
    ) {
      setError(writeBlockReason(snapshot, target));
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await bridge.copyItemToStore({
        storeId: item.store,
        path: item.path,
        version: item.version,
        direntId: item.direntId,
        targetStoreId: target.id,
        targetPath: destination,
        ...(target.kind === 'team' ? { readRole, writeRole } : {}),
      });
    } catch (failure) {
      const typed = normalizeCommandError(failure);
      setError(
        typed.code === 'already-exists'
          ? `Something named ${name.trim()} is already ${inFolder(folder)} of ${target.name}. Enter another name or choose another folder.`
          : typed.code === 'conflict'
            ? 'This item was changed or removed elsewhere. Close this sheet and refresh before copying it.'
            : typed.message,
      );
      setUncertain(Boolean(typed.ambiguous) || typed.code === 'conflict');
      // A copy across servers holds the application-wide reservation, which
      // only a refresh of every server releases after an uncertain write;
      // the source's server alone would not read the destination either.
      await onMutationError(
        failure,
        otherServer ? { report: false } : { item, report: false },
      );
      setSaving(false);
      return;
    }
    let kept: string | null = null;
    if (removing) {
      try {
        await bridge.trashItem({
          storeId: item.store,
          path: item.path,
          version: item.version,
          direntId: item.direntId,
        });
      } catch (failure) {
        // The copy is saved; the original stays where it is.
        kept = normalizeCommandError(failure).message;
        await onMutationError(failure, { item, report: false });
      }
    }
    // A failed refresh must never offer to repeat an acknowledged copy.
    setWorkflow(null);
    const message =
      kept !== null
        ? `Copied ${itemName} to ${target.name}. The original is still in ${source.name}: ${kept}`
        : removing
          ? `Moved ${itemName} to ${target.name}. The original is in Trash in ${source.name}.`
          : `Copied ${itemName} to ${target.name}`;
    try {
      // A copy within one server refreshes that server's vaults alone.
      await onApplied(
        message,
        otherServer ? undefined : target.server,
        kept !== null ? { tone: 'warning' } : undefined,
      );
    } catch (failure) {
      await onMutationError(failure, { report: false });
    }
  };

  return (
    <SheetDialog
      title="Copy to vault"
      onClose={() => setWorkflow(null)}
      dismissible={!saving}
      width="mid"
      footer={
        <>
          <span
            className={problem ? 'move-preview bad' : 'move-preview'}
            aria-live="polite"
          >
            {problem ?? summary}
          </span>
          <Button disabled={saving} onClick={() => setWorkflow(null)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={saving || uncertain || Boolean(problem)}
            onClick={() => {
              void submit();
            }}
          >
            {saving ? 'Copying…' : removing ? 'Copy and move original' : 'Copy'}
          </Button>
        </>
      }
    >
      <p>
        {`Copy ${itemName} from ${source.name} into another vault. The copy is a new item: later changes to one do not reach the other.`}
      </p>
      <SectionLabel>Copy to</SectionLabel>
      <Inset>
        <CardSelect
          label="Copy to vault"
          options={options}
          value={targetId}
          onChange={chooseTarget}
          placeholder="Choose a vault"
          disabled={saving || uncertain}
        />
        <Field
          label="Name"
          value={name}
          disabled={saving || uncertain}
          verbatim
          onChange={(value) => {
            setName(value);
            setError(null);
          }}
        />
      </Inset>
      {target && otherServer ? (
        <Band
          live
        >{`${target.name} is on ${label(target)}, a different server from ${source.name} on ${label(source)}. The copy is stored and encrypted there.`}</Band>
      ) : null}
      {target ? (
        <>
          <SectionLabel>Folder</SectionLabel>
          <FolderPicker
            label={`Destination folder in ${target.name}`}
            storeName={target.name}
            folders={pickerFolders(snapshot, target.id)}
            value={folder}
            current={null}
            moving={[]}
            disabled={saving || uncertain}
            onChange={(path) => {
              setFolder(path);
              setError(null);
            }}
          />
          <AccessBlock
            snapshot={snapshot}
            store={target}
            readRole={readRole}
            writeRole={writeRole}
            onReadRole={setReadRole}
            onWriteRole={setWriteRole}
          />
        </>
      ) : null}
      <SectionLabel>Original</SectionLabel>
      <Inset>
        <RadioGroup label="Original">
          <RadioCard
            selected={!removing}
            title="Keep the original"
            detail={`${itemName} stays in ${source.name}.`}
            disabled={saving || uncertain}
            onSelect={() => setRemoveOriginal(false)}
          />
          <RadioCard
            selected={removing}
            title="Move the original to Trash"
            detail={
              moveProblem ??
              `After the copy is saved, ${itemName} moves to Trash in ${source.name}, where it can be restored.`
            }
            off={Boolean(moveProblem)}
            disabled={saving || uncertain}
            onSelect={() => setRemoveOriginal(true)}
          />
        </RadioGroup>
      </Inset>
      {error ? <p role="alert">{error}</p> : null}
    </SheetDialog>
  );
}
