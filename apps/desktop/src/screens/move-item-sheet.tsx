import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Field, Inset, SectionLabel, SheetDialog } from '../components';
import type { Bridge } from '../bridge';
import { normalizeCommandError } from '../bridge';
import { displayPathComponent, kindOf, nameOf, storeOf } from '../model';
import type { AgentSnapshot, Item } from '../model';
import type { MutationFailureHandler } from '../mutation-recovery';
import { useSheetGuard } from '../navigation-guard';
import { BigTile, itemTile } from './item-tile';
import { itemActionProblem } from './store-access';
import { parentPath } from './scope';
import type { Landing, NewKind, WriteWorkflow } from './write-workflows';

export function MoveSheet({
  snapshot,
  bridge,
  item,
  setWorkflow,
  onApplied,
  onLanded,
  onMutationError,
  accessNow,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  item: Item;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onApplied: (message: string, profile?: string) => Promise<void>;
  onLanded?: (landing: Landing) => void;
  onMutationError: MutationFailureHandler;
  accessNow: () => number;
}): ReactNode {
  const [destination, setDestination] = useState(item.path);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [uncertain, setUncertain] = useState(false);
  const problem = itemActionProblem(snapshot, item, true, accessNow());
  // A destination in the same folder is a rename: the button, its progress
  // label and the toast say so.
  const renaming = parentPath(destination) === parentPath(item.path);
  const folderName = (path: string): string =>
    path === '/'
      ? (storeOf(snapshot, item.store)?.name ?? item.store)
      : displayPathComponent(nameOf(path));
  const valid =
    destination.startsWith('/') &&
    destination !== '/' &&
    destination !== item.path &&
    !destination.endsWith('/') &&
    !destination
      .split('/')
      .slice(1)
      .some((part) => !part || part === '.' || part === '..');
  useSheetGuard(
    saving
      ? { verdict: 'refuse', reason: 'Wait for the move to finish.' }
      : null,
    !saving,
  );
  const submit = async () => {
    if (saving || uncertain || !valid) return;
    const blocked = itemActionProblem(snapshot, item, true, accessNow());
    if (blocked) {
      setError(blocked);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await bridge.moveItem({
        direntId: item.direntId,
        storeId: item.store,
        path: item.path,
        version: item.version,
        destination,
      });
    } catch (failure) {
      const typed = normalizeCommandError(failure);
      setError(
        typed.code === 'conflict'
          ? 'The source changed or the destination already exists. Close this sheet and refresh before trying again.'
          : typed.message,
      );
      setUncertain(Boolean(typed.ambiguous) || typed.code === 'conflict');
      await onMutationError(failure, { item, report: false });
      setSaving(false);
      return;
    }
    // A failed refresh must never offer to repeat an already acknowledged move.
    setWorkflow(null);
    try {
      await onApplied(
        renaming
          ? `Renamed ${displayPathComponent(nameOf(item.path))} to ${displayPathComponent(nameOf(destination))}`
          : `Moved ${displayPathComponent(nameOf(item.path))} to ${folderName(parentPath(destination))}`,
        storeOf(snapshot, item.store)?.server,
      );
      onLanded?.({
        type: 'item',
        store: item.store,
        path: destination,
        kind: kindOf({ ...item, path: destination }) as NewKind,
      });
    } catch (failure) {
      await onMutationError(failure, { report: false });
    }
  };
  const storeName = storeOf(snapshot, item.store)?.name ?? item.store;
  return (
    <SheetDialog
      title="Rename or Move"
      subtitle={`${nameOf(item.path)} in ${storeName}`}
      glyph={<BigTile face={itemTile(item)} />}
      onClose={() => setWorkflow(null)}
      dismissible={!saving}
      footer={
        <>
          <Button disabled={saving} onClick={() => setWorkflow(null)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={saving || uncertain || !valid || Boolean(problem)}
            title={problem}
            onClick={() => {
              void submit();
            }}
          >
            {saving
              ? renaming
                ? 'Renaming…'
                : 'Moving…'
              : renaming
                ? 'Rename'
                : 'Move'}
          </Button>
        </>
      }
    >
      <SectionLabel>Destination</SectionLabel>
      <Inset>
        <Field
          label="Path"
          mono
          autoFocus
          value={destination}
          disabled={saving || uncertain}
          error={error ?? undefined}
          onChange={(value) => {
            setDestination(value);
            setError(null);
          }}
        />
      </Inset>
      <p className="hint">
        Enter the full path, including the new name, in an existing folder of{' '}
        {storeName}. An item already at that path is never overwritten.
      </p>
      {valid && kindOf({ ...item, path: destination }) !== kindOf(item) ? (
        <p className="hint">
          {kindOf(item) === 'Password'
            ? 'Outside /logins, this item is listed with Documents.'
            : 'Under /logins, this item is listed with Passwords.'}
        </p>
      ) : null}
      {/* A refused destination is stated on the field itself; this is the
          account's access to the item, which no path can change. */}
      {problem ? (
        <p className="action-error" role="alert">
          {problem}
        </p>
      ) : null}
    </SheetDialog>
  );
}
