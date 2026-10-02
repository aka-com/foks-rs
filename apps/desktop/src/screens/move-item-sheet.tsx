import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Field, SheetDialog } from '../components';
import type { Bridge } from '../bridge';
import { normalizeCommandError } from '../bridge';
import { nameOf, storeOf } from '../model';
import type { AgentSnapshot, Item } from '../model';
import type { MutationFailureHandler } from '../mutation-recovery';
import { useSheetGuard } from '../navigation-guard';
import { itemActionProblem } from './store-access';
import type { WriteWorkflow } from './write-workflows';

export function MoveSheet({
  snapshot,
  bridge,
  item,
  setWorkflow,
  onApplied,
  onMutationError,
  accessNow,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  item: Item;
  setWorkflow: (workflow: WriteWorkflow) => void;
  onApplied: (message: string, profile?: string) => Promise<void>;
  onMutationError: MutationFailureHandler;
  accessNow: () => number;
}): ReactNode {
  const [destination, setDestination] = useState(item.path);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [uncertain, setUncertain] = useState(false);
  const problem = itemActionProblem(snapshot, item, true, accessNow());
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
        `Moved ${nameOf(item.path)}`,
        storeOf(snapshot, item.store)?.server,
      );
    } catch (failure) {
      await onMutationError(failure, { report: false });
    }
  };
  return (
    <SheetDialog
      title="Rename or move"
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
        Move {nameOf(item.path)} within this vault. Enter the full destination
        path, including its new name. The destination folder must already exist.
      </p>
      <Field
        label="Destination path"
        value={destination}
        disabled={saving || uncertain}
        onChange={(value) => {
          setDestination(value);
          setError(null);
        }}
      />
      {problem || error ? <p role="alert">{problem ?? error}</p> : null}
    </SheetDialog>
  );
}
